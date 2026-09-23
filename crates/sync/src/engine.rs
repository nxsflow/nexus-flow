//! The client anti-entropy engine (feature `engine`). Client-driven, realtime deferred
//! (spec §4.3/§4.4). One `sync` pass:
//!
//! 1. **PUSH** every un-pushed **local-origin** op (rowid > `pushed_through`) to the
//!    relay in bounded batches (`page_limit` ops each), advancing `pushed_through` per
//!    batch — so the relay can cap per-request work without rejecting a large history.
//! 2. **PULL** `read_since(pulled_through)` and fold each op through `core::apply`,
//!    advancing `pulled_through` per page until the relay's cursor stops advancing. The stop
//!    condition is the CURSOR, never the page's shape — see [`Transport::pull`].
//!
//! Two watermarks (§4.3):
//! - `pushed_through` — the durable **local insertion sequence** = the SQLite `rowid` of
//!   the `ops` table. Deliberately not ULID/wall-clock: under concurrent `nxf` processes
//!   (WAL) ULIDs of different processes interleave, but `rowid` stays monotone per store.
//! - `pulled_through` — the **server cursor** (§5).
//!
//! Push filters by `site` so an op we received via *pull* (a foreign site) is never
//! re-pushed: only ops authored at *this* replica's site leave through push. That keeps
//! the relay from accumulating echoes while still catching every concurrent local write.

use nexus_flow_core::model::Op;
use nexus_flow_core::store::Store;

use crate::presence::{self, Presence};
use crate::protocol::{
    Cursor, MachineHello, MachinesResponse, RegisterOutcome, RegisterRequest, StreamId,
};
use crate::wire::{WireOp, ENVELOPE_VERSION};

/// A transport the engine pushes to / pulls from. Abstract so the engine is testable
/// without a live server and so the relay endpoint is swappable.
pub trait Transport {
    fn push(&self, stream: &StreamId, ops: &[WireOp]) -> Result<(), TransportError>;

    /// One page of ops after `since`, plus the cursor to resume from.
    ///
    /// The returned cursor is how far the relay **scanned**, not what it returned. The two
    /// coincide today (the relay serves every op it scans), but keeping them distinct is what
    /// lets E4 add ACL-scoped reads server-side: a page may come back empty or short because ops
    /// in that range are not this reader's to see, while the stream still holds visible ops
    /// further along. Callers must therefore treat page SHAPE as carrying no information about
    /// exhaustion — only a cursor that stops advancing means the stream is drained (forward-compat
    /// invariant 4, 6j6v.xsf3).
    fn pull(
        &self,
        stream: &StreamId,
        since: Cursor,
        limit: usize,
    ) -> Result<(Vec<WireOp>, Cursor), TransportError>;

    /// Register this replica's `prefix` for `stream` under its durable `replica_uuid`
    /// (bab/§6, Step 0 of a sync). The relay reassigns a fresh prefix on collision.
    fn register(
        &self,
        stream: &StreamId,
        prefix: &str,
        replica_uuid: &str,
    ) -> Result<RegisterOutcome, TransportError>;

    /// Tell the relay this machine is here (6j6v.f0b5) — `POST /streams/{id}/machines`. The
    /// background service calls it after every pass that synced; see [`crate::presence`] for what
    /// the relay keeps and how a reader judges it.
    ///
    /// A DEFAULT method, so a `Transport` written before presence existed — a test double, or an
    /// embedding host's own wrapper around [`HttpTransport`] — keeps compiling. The default is an
    /// ERROR naming the omission, not [`Announced::Unsupported`]: a wrapper that forgot to forward
    /// must not make a relay that has presence look like one that does not. A wrapper that wants
    /// presence forwards this to the transport it wraps.
    fn announce(
        &self,
        _stream: &StreamId,
        _hello: &MachineHello,
    ) -> Result<Announced, TransportError> {
        Err(not_forwarded("announce"))
    }

    /// The machines the relay knows for `stream` (6j6v.f0b5) — `GET /streams/{id}/machines`, as
    /// the relay sent it, unjudged. `Ok(None)` means the relay has no presence route. Callers want
    /// [`machines`], which judges the list; this is the raw half a wrapper forwards. A default
    /// method that errors, for the same reason as [`Transport::announce`].
    fn machines(&self, _stream: &StreamId) -> Result<Option<MachinesResponse>, TransportError> {
        Err(not_forwarded("machines"))
    }
}

/// The error a [`Transport`] that does not forward presence answers with — see
/// [`Transport::announce`].
fn not_forwarded(method: &str) -> TransportError {
    TransportError(format!(
        "this transport does not forward presence (Transport::{method}) — a wrapper must forward \
         it to the transport it wraps"
    ))
}

/// What a relay did with an announcement ([`Transport::announce`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Announced {
    /// The relay recorded this machine as seen, now.
    Recorded,
    /// The relay has no presence route — it predates 6j6v.f0b5. Syncing is unaffected; this
    /// machine just does not appear in anyone's list until the relay is upgraded.
    Unsupported,
}

/// Which machines sync `stream`, and which of them are online (6j6v.f0b5) — the ONE read the CLI
/// (`nxs sync machines`) and every embedding host make, so both show the same thing.
///
/// A hosted host that holds a relay URL and a stream id calls it with an [`HttpTransport`] (or
/// its own wrapper, forwarding [`Transport::machines`]); the verdict per machine comes from
/// [`presence::judge`]. A relay without presence is [`Presence::Unsupported`], never an error.
pub fn machines(transport: &dyn Transport, stream: &StreamId) -> Result<Presence, SyncError> {
    Ok(match transport.machines(stream)? {
        Some(resp) => presence::judge_all(&resp),
        None => Presence::Unsupported,
    })
}

/// The most a presence answer may weigh. A full answer is at most
/// [`presence::MAX_LISTED_MACHINES`] entries of a few hundred bytes; anything past this is not a
/// relay answering honestly, and reading it whole would let one response take the client's memory.
const MAX_MACHINES_BODY_BYTES: u64 = 256 * 1024;

/// How much of a refusal's body a presence error quotes — the relay's reason is one sentence.
const MAX_REASON_BYTES: u64 = 512;

/// The credential a transport holds, in every form a message could carry it back: the Basic
/// header's base64 payload, the `user:password` pair and each half on its own — percent-encoded as
/// configured and decoded. Scrubbed from every error on top of the userinfo rule, because a gateway
/// that echoes the request's `Authorization` header into its refusal body hands the key back in a
/// form [`redact_userinfo`](crate::redact::redact_userinfo) cannot see (review of PR #487,
/// Integrity #2). A form shorter than four characters is left out: masking every `u` in a message
/// would say nothing and hide everything.
#[derive(Default)]
struct Secret {
    forms: Vec<String>,
}

impl Secret {
    fn of(user: &str, password: &str, base64_payload: &str) -> Secret {
        let mut forms: Vec<String> = [
            base64_payload.to_string(),
            format!("{user}:{password}"),
            format!("{}:{}", percent_decode(user), percent_decode(password)),
            user.to_string(),
            password.to_string(),
            percent_decode(user),
            percent_decode(password),
        ]
        .into_iter()
        .filter(|form| form.chars().count() >= 4)
        .collect();
        // Longest first, so a pair is masked whole before either half is looked for inside it.
        forms.sort_by_key(|form| std::cmp::Reverse(form.len()));
        forms.dedup();
        Secret { forms }
    }

    fn scrub(&self, text: &str) -> String {
        let mut out = crate::redact::redact_userinfo(text);
        for form in &self.forms {
            out = out.replace(form.as_str(), crate::redact::MASK);
        }
        out
    }
}

/// `%XX` decoded — for the credential as a person typed it rather than as the URL carries it.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).copied(),
            bytes.get(i + 2).copied(),
        ) {
            (b'%', Some(h), Some(l)) if hex(h).is_some() && hex(l).is_some() => {
                out.push((hex(h).unwrap() * 16 + hex(l).unwrap()) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A transport-layer failure (network / HTTP / decode). Opaque string payload.
#[derive(Debug)]
pub struct TransportError(pub String);

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transport: {}", self.0)
    }
}

impl std::error::Error for TransportError {}

/// A sync failure: either the local store or the transport.
#[derive(Debug)]
pub enum SyncError {
    Storage(String),
    Transport(String),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncError::Storage(m) => write!(f, "sync storage: {m}"),
            SyncError::Transport(m) => write!(f, "sync transport: {m}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<rusqlite::Error> for SyncError {
    fn from(e: rusqlite::Error) -> SyncError {
        SyncError::Storage(e.to_string())
    }
}

impl From<TransportError> for SyncError {
    fn from(e: TransportError) -> SyncError {
        SyncError::Transport(e.0)
    }
}

/// The two durable watermarks, owned by the caller (the CLI persists them in sync meta).
/// Default = `0/0` — nothing pushed, nothing pulled (the start position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Watermarks {
    pub pushed_through: i64,
    pub pulled_through: i64,
}

/// What one [`sync`] pass moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncOutcome {
    pub pushed: usize,
    pub pulled: usize,
    /// Set when the pull loop stopped because it hit [`PULL_PAGE_CEILING`], not because the
    /// stream was actually exhausted (Integrity #3, PR #263) — the caller should surface this so
    /// an operator knows another pass is needed to finish draining a genuinely huge backlog.
    /// `pulled_through` is still advanced through the last page taken (the loop's normal
    /// per-page bookkeeping, untouched by why it stopped), so the NEXT pass resumes exactly
    /// where this one left off rather than losing progress.
    pub pull_ceiling_hit: bool,
    /// Set when the pull loop stopped because its [`PassBudget`] said so (6j6v.25f6) — the
    /// wall-clock twin of [`pull_ceiling_hit`], and a clean stop in exactly the same sense: the
    /// watermark already sits at the resume point.
    pub budget_exhausted: bool,
    /// How many pull pages this pass fetched, and how many of them came back with NO ops while
    /// still advancing the cursor.
    ///
    /// **They exist to tell two very different situations apart** (6j6v.25f6). A pass that stops
    /// early looks identical from the outside whether the stream really is enormous or the relay is
    /// answering every request with almost nothing and nudging the cursor a little each time. The
    /// first is a backlog that will drain; the second never does, and it is the shape a wedged or
    /// hostile relay has. The operator could not distinguish them at all before, and the counters
    /// are what a report is built from — the engine itself draws no conclusion.
    pub pull_pages: usize,
    pub pull_empty_pages: usize,
    /// Ops this replica already held WITH a signature that the relay delivered WITHOUT one — its
    /// own coming back from its own push included (6j6v.pzkb). Direct evidence that the relay drops
    /// the envelope's signature pair: one older than 5crb, or one that tampers. Nothing acts on an
    /// unsigned op either way, so it costs actions and grants none — but it must never be silent.
    pub signatures_stripped: usize,
    /// Ops NEW to this replica, delivered unsigned, from a site whose other ops carry a signature
    /// (6j6v.pzkb) — the receiver's view of the same downgrade: a replica that signs does not send
    /// unsigned ops, so somebody between it and here removed the signature. (A replica rolled back
    /// to a build that does not sign produces this too, and that is worth a look as well.)
    pub unsigned_from_signers: usize,
}

/// Asked once per pull page: is this pass out of budget?
///
/// **This is how a wall-clock bound reaches a deliberately clock-free engine.** The design keeps
/// all time in the daemon, where `Scheduler`/`DaemonState` are clock-injected and provable without
/// sleeping; putting an `Instant::now()` in `sync()` would take that apart. So the engine does not
/// learn the time — it asks a question, and whoever built the budget answers it out of a clock it
/// owns. A test answers it out of a counter, which is why the bound can be proven without a second
/// of real time passing (6j6v.25f6's DoD asks for exactly that).
pub trait PassBudget {
    /// `true` ⇒ stop after the page just applied. Called once per iteration, never mid-page, so a
    /// pass always stops on a durable boundary.
    fn exhausted(&self) -> bool;
}

/// No budget at all — the pass runs until the stream is exhausted or the page ceiling stops it.
///
/// Named rather than implied (there is no `Option<&dyn PassBudget>`): a caller that wants an
/// unbounded pass says so, and the one production caller that must NOT be unbounded cannot get
/// there by leaving an argument off.
pub struct Unbounded;

impl PassBudget for Unbounded {
    fn exhausted(&self) -> bool {
        false
    }
}

/// Hard per-pass ceiling on pull pages: without one, the pull loop follows `next` until the
/// relay reports the stream exhausted, and a relay that keeps returning a FULL page with an
/// ADVANCING cursor — buggy, compromised, or just serving a pathologically large stream — wedges
/// the pass forever. The sync daemon (`nxs::sync::daemon::serve`) is a single-threaded poll loop
/// that sweeps every registered workspace in turn, so one wedged workspace starves ALL the
/// others, not just itself. 20,000 pages at the client's own page size (`SYNC_PAGE_LIMIT = 500`,
/// `nxs::sync::mod`) is 10,000,000 ops in ONE pass — this project's entire history is nowhere
/// near two orders of magnitude of that, so tripping the ceiling is itself a signal something is
/// wrong (or the stream is enormous), not a false positive on legitimate use. Tripping it is not
/// an error: the loop's normal per-page watermark advance already leaves `pulled_through` at
/// exactly the right resume point, so the pass stops cleanly and the next pass just continues.
const PULL_PAGE_CEILING: usize = 20_000;

/// Run one client-driven anti-entropy pass against `transport`, mutating `marks` in place.
/// `local_site` is this replica's op site (from the workspace identity); it scopes the
/// push so foreign (pulled) ops are not echoed back.
pub fn sync(
    store: &mut Store,
    local_site: i64,
    stream: &StreamId,
    marks: &mut Watermarks,
    transport: &dyn Transport,
    page_limit: usize,
    budget: &dyn PassBudget,
) -> Result<SyncOutcome, SyncError> {
    sync_with_pull_ceiling(
        store,
        local_site,
        stream,
        marks,
        transport,
        page_limit,
        PULL_PAGE_CEILING,
        budget,
    )
}

/// The actual body behind [`sync`], with the pull-page ceiling taken as a parameter rather than
/// baked in: proving the cap terminates a wedged pull loop only needs a FEW pages tripping it,
/// not the production 20,000 — looping that for real would make the test needlessly slow without
/// exercising anything the mechanism doesn't already prove at a smaller number. `sync` is the
/// only production caller (always passing [`PULL_PAGE_CEILING`]); this file's own test on a
/// relay that never exhausts the stream calls this directly with a small ceiling.
#[allow(clippy::too_many_arguments)]
fn sync_with_pull_ceiling(
    store: &mut Store,
    local_site: i64,
    stream: &StreamId,
    marks: &mut Watermarks,
    transport: &dyn Transport,
    page_limit: usize,
    pull_page_ceiling: usize,
    budget: &dyn PassBudget,
) -> Result<SyncOutcome, SyncError> {
    // A zero page would panic `chunks(0)` in push (and spin the pull loop). The sole caller
    // passes a fixed non-zero limit, but `sync` is `pub`, so clamp once here rather than
    // make non-zero an unenforced caller contract — this makes the `.expect` below provably
    // unreachable.
    let page_limit = page_limit.max(1);

    // ---- PUSH: un-pushed local-origin ops, paginated by `page_limit`, in rowid order ----
    // Mirrors pull: send ceil(N/page) bounded batches rather than one unbounded request, so
    // the relay can enforce a deliberate per-request op-count cap without rejecting a large
    // legitimate history (k64). pushed_through advances PER batch — a transport failure
    // partway through leaves the already-pushed batches durable and the watermark exact, so
    // the next pass resumes without re-pushing or skipping an op.
    let local = local_ops_since(store.connection(), local_site, marks.pushed_through)?;
    let pushed = local.len();
    for batch in local.chunks(page_limit) {
        let max_rowid = batch
            .last()
            .map(|(rowid, _)| *rowid)
            .expect("chunk is non-empty");
        let ops: Vec<WireOp> = batch.iter().map(|(_, w)| w.clone()).collect();
        transport.push(stream, &ops)?;
        marks.pushed_through = max_rowid;
    }

    // ---- PULL: read_since(cursor), fold each op, paginate until exhausted (or capped) ----
    //
    // The loop terminates on **the cursor standing still**, never on the SHAPE of a page.
    // `read_since` reports how far it SCANNED, not what it returned, so an empty or short page
    // means "nothing for you in that range" — not "the stream is exhausted". Reading it as
    // exhaustion is what would hard-wire "whole log to everyone" into the client: the E4 auth
    // slice (6j6v.6aza) adds an ACL filter by kind/channel keyed on the authenticated identity,
    // and under one, a run of ops this reader may not see would end the pass early and — for an
    // EMPTY page, whose watermark never moved — be re-scanned on every pass forever. Terminating
    // on progress instead keeps that a server-side change (forward-compat invariant 4, 6j6v.xsf3).
    //
    // The cost of not short-circuiting on `n < page_limit` is one extra round-trip per pass to
    // observe the standstill. That is the deliberate price of the invariant: a per-pass constant,
    // against a client rewrite later.
    let mut pulled = 0;
    let mut pull_pages = 0usize;
    let mut pull_empty_pages = 0usize;
    let mut pull_ceiling_hit = false;
    let mut budget_exhausted = false;
    let (mut signatures_stripped, mut unsigned_from_signers) = (0usize, 0usize);
    loop {
        let since = marks.pulled_through;
        let (wires, next) = transport.pull(stream, Cursor(since), page_limit)?;
        let page_was_empty = wires.is_empty();
        // Advance FIRST, and unconditionally: the scan position is durable progress even when the
        // page came back empty, and a relay that hands back a stale/rewound cursor must not drag
        // the watermark backwards into re-pulling settled history.
        marks.pulled_through = next.0.max(since);
        if !wires.is_empty() {
            pulled += wires.len();
            // Asked BEFORE the page is applied: the question is what this replica held until now.
            let (stripped, unsigned) = downgrades(store.connection(), &wires)?;
            signatures_stripped += stripped;
            unsigned_from_signers += unsigned;
            let ops: Vec<Op> = wires.into_iter().map(Op::from).collect();
            // `apply` is the idempotent union: INSERT OR IGNORE on op_id dedups re-delivered
            // ops, and the fold is order-independent (§4.4). Re-pull never double-counts.
            store.apply(&ops);
        }
        if marks.pulled_through == since {
            // The cursor did not move: the relay has nothing beyond here. The one true stop.
            break;
        }
        pull_pages += 1;
        if page_was_empty {
            // Counted only for a page whose cursor DID move — an empty page that also stands still
            // is the ordinary "nothing new" answer and left through the arm above.
            pull_empty_pages += 1;
        }
        if pull_pages >= pull_page_ceiling {
            // Stop CLEANLY, not an error (see the const's doc): `pulled_through` above already
            // sits at the resume point for the next pass.
            pull_ceiling_hit = true;
            break;
        }
        // Asked AFTER the page is applied and the watermark advanced, so a pass that runs out of
        // budget still keeps everything it did — the same clean-stop shape as the ceiling above.
        if budget.exhausted() {
            budget_exhausted = true;
            break;
        }
    }

    Ok(SyncOutcome {
        pushed,
        pulled,
        pull_ceiling_hit,
        budget_exhausted,
        pull_pages,
        pull_empty_pages,
        signatures_stripped,
        unsigned_from_signers,
    })
}

/// Count the silent downgrades on one pull page (6j6v.pzkb): see
/// [`SyncOutcome::signatures_stripped`] and [`SyncOutcome::unsigned_from_signers`].
///
/// Only an op delivered WITHOUT any part of a signature is looked at. An op this replica already
/// holds unsigned is an ordinary re-delivery (every op written before signing existed comes back
/// that way) and counts as nothing; one it holds signed is a strip; a new one counts when its site
/// is known to sign — from this log, or from a signed op on the same page.
fn downgrades(conn: &rusqlite::Connection, wires: &[WireOp]) -> Result<(usize, usize), SyncError> {
    use rusqlite::OptionalExtension;
    use std::collections::{HashMap, HashSet};
    let signing_on_page: HashSet<i64> = wires
        .iter()
        .filter(|w| w.carries_a_signature())
        .map(|w| w.site)
        .collect();
    let mut held = conn.prepare_cached("SELECT key_id IS NOT NULL FROM ops WHERE op_id = ?1")?;
    let mut signs =
        conn.prepare_cached("SELECT 1 FROM ops WHERE site = ?1 AND key_id IS NOT NULL LIMIT 1")?;
    let mut site_signs: HashMap<i64, bool> = HashMap::new();
    let (mut stripped, mut unsigned) = (0, 0);
    for wire in wires.iter().filter(|w| !w.carries_a_signature()) {
        match held
            .query_row([&wire.op_id], |r| r.get::<_, bool>(0))
            .optional()?
        {
            Some(true) => stripped += 1,
            Some(false) => {}
            None => {
                let known = match site_signs.get(&wire.site) {
                    Some(known) => *known,
                    None => {
                        let known = signing_on_page.contains(&wire.site)
                            || signs
                                .query_row([wire.site], |_| Ok(()))
                                .optional()?
                                .is_some();
                        site_signs.insert(wire.site, known);
                        known
                    }
                };
                if known {
                    unsigned += 1;
                }
            }
        }
    }
    Ok((stripped, unsigned))
}

/// Step 0 of a sync pass (§6/§8): register this replica's `prefix` for `stream` under its
/// durable `replica_uuid`. If the relay reassigns a new prefix — another replica already
/// claimed this one — remap the local store in place (only the prefix component of its ids,
/// free text included, via [`Store::remap_prefix`]) BEFORE any push/pull, and return the
/// new prefix so the caller can adopt it for future mints. Returns `None` when the prefix
/// was accepted unchanged. Idempotent: re-registering after adoption returns `None`, and a
/// re-issued reassignment re-runs a remap that is itself a no-op once nothing of the old
/// prefix remains.
pub fn register_prefix(
    store: &mut Store,
    stream: &StreamId,
    prefix: &str,
    replica_uuid: &str,
    transport: &dyn Transport,
) -> Result<Option<String>, SyncError> {
    match transport.register(stream, prefix, replica_uuid)? {
        RegisterOutcome::Registered => Ok(None),
        RegisterOutcome::Reassigned { new_prefix } => {
            // The relay is otherwise opaque/untrusted; a malformed `new_prefix` (one with a
            // '.' or the edge separator) would corrupt every id the remap splices it into.
            // Validate BEFORE touching the store — refuse rather than corrupt.
            if !nexus_flow_core::id::is_valid_prefix(&new_prefix) {
                return Err(SyncError::Transport(format!(
                    "relay reassigned a malformed prefix {new_prefix:?}; refusing to remap"
                )));
            }
            store.remap_prefix(prefix, &new_prefix);
            Ok(Some(new_prefix))
        }
    }
}

/// Read this replica's un-pushed local ops: `rowid > after_rowid AND site = local_site`,
/// in rowid order, each wrapped in the current envelope. Reads the stable `ops` substrate
/// table read-only via the exposed connection (the core is untouched in the spine).
fn local_ops_since(
    conn: &rusqlite::Connection,
    local_site: i64,
    after_rowid: i64,
) -> Result<Vec<(i64, WireOp)>, SyncError> {
    let mut stmt = conn.prepare(
        "SELECT rowid, op_id, lamport, site, domain, target_kind, target_id, field, op_type,
                value, author, wall_clock, key_id, sig
         FROM ops
         WHERE rowid > ?1 AND site = ?2
         ORDER BY rowid",
    )?;
    let rows = stmt.query_map([after_rowid, local_site], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            WireOp {
                envelope_version: ENVELOPE_VERSION,
                op_id: r.get(1)?,
                lamport: r.get(2)?,
                site: r.get(3)?,
                domain: r.get(4)?,
                target_kind: r.get(5)?,
                target_id: r.get(6)?,
                field: r.get(7)?,
                op_type: r.get(8)?,
                value: r.get(9)?,
                author: r.get(10)?,
                wall_clock: r.get(11)?,
                // A local op is authored by THIS build, so there is nothing it does not know about
                // its own envelope — except its signature pair, which travels in the catch-all
                // (see `WireOp::extra` for why) exactly as the log holds it.
                extra: [
                    (crate::wire::KEY_ID_FIELD, r.get::<_, Option<String>>(12)?),
                    (crate::wire::SIG_FIELD, r.get::<_, Option<String>>(13)?),
                ]
                .into_iter()
                .filter_map(|(name, part)| {
                    part.map(|v| (name.to_string(), serde_json::Value::String(v)))
                })
                .collect(),
            },
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Percent-encode ONE url path segment: the RFC 3986 §2.3 unreserved set survives, every
/// other byte becomes `%XX`. Hand-rolled rather than a crate — the encode set here is
/// exactly "path segment", which is 15 lines and one test table.
///
/// Without this a `stream_id` carrying `/` silently becomes SEVERAL path segments and the
/// relay's `/streams/:id/ops` route never matches (404); a `#` truncates the path and a `?`
/// injects query parameters. `--join <id>` takes arbitrary operator input, so this is
/// reachable. The bind seam ALSO gates the charset (nexus-flow p5sa) — this is the
/// defensive half, for ids already persisted in a `.nxs/sync.toml`.
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// HTTP transport (`ureq`) against a relay base URL, e.g. `http://127.0.0.1:8787`. Carries
/// connect/read timeouts so a hung or unreachable relay fails the sync pass instead of
/// blocking `nxs sync` indefinitely.
///
/// **A credential in the URL never stays in it** (6j6v.q3kk). A relay behind a gateway takes its
/// key as userinfo — `https://<user>:<password>@relay.example` — and ureq quotes the request URL in
/// every error it raises, so that key used to travel from a failed pass into `service.log`, the
/// heartbeat and every reader of `service_attendance`. [`new`](Self::new) therefore takes the
/// userinfo off and sends it as the `Authorization: Basic` header ureq would have derived from it;
/// every URL this transport formats is the bare one, and every error it raises is scrubbed of the
/// credential in any form ([`Secret`]). Deliberately no `Debug`: the header value is the key.
///
/// Two things ureq did with a userinfo it would NOT do with an explicit header, kept on purpose
/// (review of PR #487, Code Quality #1): it sends no header for an empty userinfo (`http://@host`),
/// and a redirect to the same host still carries the credential (`RedirectAuthHeaders::SameHost` —
/// ureq drops an explicit header on every redirect by default, where it re-derived the userinfo of a
/// relative `Location`). Never to another host, and never from https down to http.
pub struct HttpTransport {
    /// The relay's base URL WITHOUT its userinfo — the only form any message may quote.
    base: String,
    /// `Basic <base64(user:password)>` when the configured URL carried a credential.
    authorization: Option<String>,
    secret: Secret,
    agent: ureq::Agent,
}

impl HttpTransport {
    pub fn new(base: impl Into<String>) -> HttpTransport {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(30))
            .redirect_auth_headers(ureq::RedirectAuthHeaders::SameHost)
            .build();
        let (base, authorization, secret) = split_credentials(base.into());
        HttpTransport {
            base,
            authorization,
            secret,
            agent,
        }
    }

    /// Every error this transport raises is built here, so what it says passes the credential
    /// rules on its way out — whatever produced it: ureq's own message, a decode error, a relay's
    /// refusal. The userinfo never reaches a URL ureq formats in the first place; this also catches
    /// the forms a relay's own words can carry (see [`Secret`]).
    fn failed(&self, message: impl std::fmt::Display) -> TransportError {
        TransportError(self.secret.scrub(&message.to_string()))
    }

    /// A non-2xx presence answer as an error that keeps the relay's own reason (a `400` names which
    /// bound the announcement broke), not only ureq's "status code 400".
    fn refused(&self, url: &str, status: u16, resp: ureq::Response) -> TransportError {
        let mut reason = String::new();
        let _ = std::io::Read::read_to_string(
            &mut std::io::Read::take(resp.into_reader(), MAX_REASON_BYTES),
            &mut reason,
        );
        let reason = reason.trim();
        if reason.is_empty() {
            self.failed(format!("{url}: status code {status}"))
        } else {
            self.failed(format!("{url}: status code {status}: {reason}"))
        }
    }

    fn get(&self, url: &str) -> ureq::Request {
        self.authorized(self.agent.get(url))
    }

    fn post(&self, url: &str) -> ureq::Request {
        self.authorized(self.agent.post(url))
    }

    /// Every request goes out through here, so the credential cannot be forgotten on one route.
    fn authorized(&self, request: ureq::Request) -> ureq::Request {
        match &self.authorization {
            Some(value) => request.set("Authorization", value),
            None => request,
        }
    }

    /// The ONE place a relay URL is built, so the encoding cannot be forgotten at one of
    /// the three call sites.
    fn url(&self, stream: &StreamId, suffix: &str) -> String {
        format!(
            "{}/streams/{}/{}",
            self.base,
            encode_segment(stream.as_str()),
            suffix
        )
    }
}

impl Transport for HttpTransport {
    fn push(&self, stream: &StreamId, ops: &[WireOp]) -> Result<(), TransportError> {
        let url = self.url(stream, "ops");
        let body = crate::protocol::PushRequest { ops: ops.to_vec() };
        self.post(&url)
            .send_json(serde_json::to_value(body).map_err(|e| self.failed(e))?)
            .map_err(|e| self.failed(e))?;
        Ok(())
    }

    fn pull(
        &self,
        stream: &StreamId,
        since: Cursor,
        limit: usize,
    ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
        let url = self.url(stream, "ops");
        let resp = self
            .get(&url)
            .query("since", &since.0.to_string())
            .query("limit", &limit.to_string())
            .call()
            .map_err(|e| self.failed(e))?;
        let page: crate::protocol::PullResponse = resp.into_json().map_err(|e| self.failed(e))?;
        Ok((page.ops, page.next))
    }

    fn register(
        &self,
        stream: &StreamId,
        prefix: &str,
        replica_uuid: &str,
    ) -> Result<RegisterOutcome, TransportError> {
        let url = self.url(stream, "register");
        let body = RegisterRequest {
            prefix: prefix.to_string(),
            replica_uuid: replica_uuid.to_string(),
        };
        let payload = serde_json::to_value(body).map_err(|e| self.failed(e))?;
        match self.post(&url).send_json(payload) {
            Ok(resp) => resp.into_json().map_err(|e| self.failed(e)),
            // Forward-compat with a pre-bab relay that has no `/register` route: a 404 means
            // "no registry here", so proceed UNregistered — keep our offline-minted prefix,
            // exactly the Slice-1 behavior (distinctness then rests on birthday odds). This
            // mirrors the op-shape store-then-skip tolerance: an older peer must not hard-fail
            // the whole sync. Any other status is a real transport error.
            Err(ureq::Error::Status(404, _)) => Ok(RegisterOutcome::Registered),
            Err(e) => Err(self.failed(e)),
        }
    }

    fn announce(
        &self,
        stream: &StreamId,
        hello: &MachineHello,
    ) -> Result<Announced, TransportError> {
        let url = self.url(stream, "machines");
        let payload = serde_json::to_value(hello).map_err(|e| self.failed(e))?;
        match self.post(&url).send_json(payload) {
            Ok(_) => Ok(Announced::Recorded),
            Err(ureq::Error::Status(status, _)) if route_absent(status) => {
                Ok(Announced::Unsupported)
            }
            Err(ureq::Error::Status(status, resp)) => Err(self.refused(&url, status, resp)),
            Err(e) => Err(self.failed(e)),
        }
    }

    fn machines(&self, stream: &StreamId) -> Result<Option<MachinesResponse>, TransportError> {
        let url = self.url(stream, "machines");
        match self.get(&url).call() {
            Ok(resp) => serde_json::from_reader(std::io::Read::take(
                resp.into_reader(),
                MAX_MACHINES_BODY_BYTES,
            ))
            .map(Some)
            .map_err(|e| self.failed(format!("{url}: {e}"))),
            Err(ureq::Error::Status(status, _)) if route_absent(status) => Ok(None),
            Err(ureq::Error::Status(status, resp)) => Err(self.refused(&url, status, resp)),
            Err(e) => Err(self.failed(e)),
        }
    }
}

/// Whether an HTTP status on the presence route means "this relay has no such route" — the
/// forward-compat reading `register` already gives a 404, for the same reason: a relay older than
/// the route must not fail a pass (6j6v.f0b5). 405 belongs here too, because axum answers it, not
/// 404, when the path exists for another method. The honest cost: a base URL that points at
/// something that is not a relay at all reads the same way, which is why every caller that prints
/// it says "or the URL is not a relay" rather than asserting an old version.
fn route_absent(status: u16) -> bool {
    matches!(status, 404 | 405)
}

/// Split a configured relay URL into the bare URL, the `Authorization` header its userinfo stands
/// for, and the credential's forms to scrub from messages (6j6v.q3kk).
///
/// **One parse decides everything** (review of PR #487, Integrity #8): the URL parser ureq uses
/// reads the userinfo, the host and the bare URL, so the host this transport contacts is exactly the
/// one ureq would have contacted for the configured text — a URL like `https://a\@b/`, which a
/// textual split and the parser read differently, cannot move a request to another host.
///
/// The header is what ureq derives from a userinfo — `Basic` over `username:password` as the parser
/// reports them, still percent-encoded, an absent password as empty — and, as with ureq, there is
/// none when both halves are empty. The bare URL is the parsed one without its userinfo; the
/// trailing `/` the parser gives an empty path is dropped again, because every URL the transport
/// builds appends `/streams/…` to it. A URL that does not parse, or carries no credential, keeps its
/// text as it is.
fn split_credentials(configured: String) -> (String, Option<String>, Secret) {
    use base64::Engine as _;
    let Ok(parsed) = url::Url::parse(&configured) else {
        return (configured, None, Secret::default());
    };
    let (user, password) = (parsed.username(), parsed.password().unwrap_or(""));
    if user.is_empty() && password.is_empty() {
        return (configured, None, Secret::default());
    }
    let payload = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"));
    let secret = Secret::of(user, password, &payload);
    let mut bare = parsed.clone();
    // Cannot fail on a URL that carried a userinfo: only a cannot-be-a-base URL refuses these, and
    // such a URL has no authority to carry one.
    let _ = bare.set_username("");
    let _ = bare.set_password(None);
    let mut base = bare.to_string();
    if bare.path() == "/" && base.ends_with('/') && !configured.ends_with('/') {
        base.pop();
    }
    (base, Some(format!("Basic {payload}")), secret)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::MachineSeen;
    use nexus_flow_core::model::EdgeKind;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// An in-process stand-in for the relay: an append-only ordered log per stream, with
    /// the same cursor semantics (seq = position+1). Lets the engine's algorithm be
    /// proven without HTTP or the server crate. It also mirrors the prefix registry's
    /// claim-if-free / reassign-on-collision contract so register-step tests are faithful.
    struct MemTransport {
        streams: RefCell<HashMap<String, Vec<WireOp>>>,
        pushes: RefCell<usize>,
        /// The op-count of every `push` CALL, in order — lets a test assert the engine
        /// paginated (multiple bounded batches) rather than sending one unbounded request.
        push_batches: RefCell<Vec<usize>>,
        /// (stream, prefix) -> owning replica_uuid.
        prefix_owner: RefCell<HashMap<(String, String), String>>,
        /// (stream, replica_uuid) -> its claimed prefix (idempotency/stability).
        uuid_prefix: RefCell<HashMap<(String, String), String>>,
    }

    impl MemTransport {
        fn new() -> MemTransport {
            MemTransport {
                streams: RefCell::new(HashMap::new()),
                pushes: RefCell::new(0),
                push_batches: RefCell::new(Vec::new()),
                prefix_owner: RefCell::new(HashMap::new()),
                uuid_prefix: RefCell::new(HashMap::new()),
            }
        }
    }

    /// Deterministic, uuid-distinct 4-char candidate (the test double's stand-in for the
    /// registry's `candidate_prefix`). It need NOT produce the same prefixes as production:
    /// the engine only relies on the registry CONTRACT (distinct, stable reassignment), not
    /// on which prefix is chosen — so this is an independent mirror, not duplicated logic.
    fn mem_candidate(uuid: &str, n: u32) -> String {
        const A: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in uuid.as_bytes().iter().chain(n.to_le_bytes().iter()) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        (0..4)
            .map(|i| A[((h >> (i * 5)) & 0x1f) as usize] as char)
            .collect()
    }

    impl Transport for MemTransport {
        fn push(&self, stream: &StreamId, ops: &[WireOp]) -> Result<(), TransportError> {
            *self.pushes.borrow_mut() += ops.len();
            self.push_batches.borrow_mut().push(ops.len());
            self.streams
                .borrow_mut()
                .entry(stream.0.clone())
                .or_default()
                .extend_from_slice(ops);
            Ok(())
        }

        fn pull(
            &self,
            stream: &StreamId,
            since: Cursor,
            limit: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            let map = self.streams.borrow();
            let log = map.get(&stream.0).cloned().unwrap_or_default();
            let start = since.0 as usize;
            let page: Vec<WireOp> = log.iter().skip(start).take(limit).cloned().collect();
            let next = Cursor((start + page.len()) as i64);
            Ok((page, next))
        }

        // Two maps + a probing loop don't fit the entry API cleanly; this is a test double.
        #[allow(clippy::map_entry)]
        fn register(
            &self,
            stream: &StreamId,
            prefix: &str,
            replica_uuid: &str,
        ) -> Result<RegisterOutcome, TransportError> {
            let mut owners = self.prefix_owner.borrow_mut();
            let mut by_uuid = self.uuid_prefix.borrow_mut();
            let uuid_key = (stream.0.clone(), replica_uuid.to_string());
            // Idempotent / stable answer keyed by uuid.
            if let Some(existing) = by_uuid.get(&uuid_key) {
                return Ok(if existing == prefix {
                    RegisterOutcome::Registered
                } else {
                    RegisterOutcome::Reassigned {
                        new_prefix: existing.clone(),
                    }
                });
            }
            if !owners.contains_key(&(stream.0.clone(), prefix.to_string())) {
                owners.insert(
                    (stream.0.clone(), prefix.to_string()),
                    replica_uuid.to_string(),
                );
                by_uuid.insert(uuid_key, prefix.to_string());
                return Ok(RegisterOutcome::Registered);
            }
            for n in 0.. {
                let cand = mem_candidate(replica_uuid, n);
                if !owners.contains_key(&(stream.0.clone(), cand.clone())) {
                    owners.insert((stream.0.clone(), cand.clone()), replica_uuid.to_string());
                    by_uuid.insert(uuid_key, cand.clone());
                    return Ok(RegisterOutcome::Reassigned { new_prefix: cand });
                }
            }
            unreachable!("prefix space exhausted in test double")
        }
    }

    fn materialized(store: &Store) -> Vec<(String, Option<String>, Option<String>)> {
        store
            .list_items()
            .unwrap()
            .into_iter()
            .map(|i| (i.id, i.title, i.status))
            .collect()
    }

    #[test]
    fn two_replicas_converge_through_a_relay() {
        let stream = StreamId("s".into());
        // Fixture-coordinated distinct prefixes (Slice 1 — bab/remap is Slice 2).
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "A-one", "alice");
        let mut b = Store::open_in_memory(2);
        b.create_item("bbbb.0001", "task", "B-one", "bob");

        let relay = MemTransport::new();
        let mut ma = Watermarks::default();
        let mut mb = Watermarks::default();

        // Each pushes its own, pulls the other's; a second round drains what crossed.
        for _ in 0..2 {
            sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
            sync(&mut b, 2, &stream, &mut mb, &relay, 100, &Unbounded).unwrap();
        }
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();

        assert_eq!(materialized(&a), materialized(&b), "replicas converge");
        assert_eq!(
            a.get_item("bbbb.0001").unwrap().unwrap().title.as_deref(),
            Some("B-one")
        );
        assert_eq!(
            b.get_item("aaaa.0001").unwrap().unwrap().title.as_deref(),
            Some("A-one")
        );
    }

    // ---- silent downgrade, counted (6j6v.pzkb) ------------------------------------------------

    /// The wire form of `store`'s ops, as a client that does not sign — or a relay that dropped the
    /// pair — would carry them.
    fn unsigned_wires(store: &Store) -> Vec<WireOp> {
        store
            .export()
            .iter()
            .map(|op| {
                let mut wire = WireOp::from(op);
                wire.extra.clear();
                wire
            })
            .collect()
    }

    /// The true negative of the downgrade counters (review of PR #489, Test Quality #2): unsigned ops
    /// from a replica that has never signed are an OLD CLIENT, which the spec lets keep syncing
    /// quietly (§2.6) — neither counter moves. The same site's unsigned op AFTER a signed one is
    /// the downgrade, and is counted.
    #[test]
    fn an_old_client_is_not_a_downgrade_and_a_signer_going_quiet_is() {
        let stream = StreamId("s".into());
        let relay = MemTransport::new();
        let mut old_client = Store::open_in_memory(7);
        old_client.create_item("oldc.0001", "task", "from an old client", "carol");
        relay.push(&stream, &unsigned_wires(&old_client)).unwrap();

        let mut b = Store::open_in_memory(2);
        let mut mb = Watermarks::default();
        let first = sync(&mut b, 2, &stream, &mut mb, &relay, 100, &Unbounded).unwrap();
        assert!(first.pulled > 0);
        assert_eq!(
            (first.signatures_stripped, first.unsigned_from_signers),
            (0, 0),
            "an old client's ops are ordinary: {first:?}"
        );

        // The same site signs once — and then an op of it arrives unsigned.
        let before = old_client.export().len();
        old_client.create_item("oldc.0002", "task", "signed now", "carol");
        let signed: Vec<WireOp> = old_client.export()[before..]
            .iter()
            .map(WireOp::from)
            .collect();
        relay.push(&stream, &signed).unwrap();
        let before = old_client.export().len();
        old_client.create_item("oldc.0003", "task", "and stripped again", "carol");
        let stripped: Vec<WireOp> = unsigned_wires(&old_client)[before..].to_vec();
        relay.push(&stream, &stripped).unwrap();
        let second = sync(&mut b, 2, &stream, &mut mb, &relay, 100, &Unbounded).unwrap();
        assert_eq!(second.signatures_stripped, 0);
        assert_eq!(
            second.unsigned_from_signers,
            stripped.len(),
            "a site that signs, sending unsigned, is counted: {second:?}"
        );
    }

    /// The sender's side: an op this replica holds signed, handed back by the relay without its
    /// signature, is a strip — its own included.
    #[test]
    fn an_op_held_signed_that_comes_back_unsigned_is_counted_as_stripped() {
        let stream = StreamId("s".into());
        let relay = MemTransport::new();
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "mine", "alice");
        relay.push(&stream, &unsigned_wires(&a)).unwrap();
        let held = a.export().len();
        let mut ma = Watermarks {
            pushed_through: i64::MAX,
            pulled_through: 0,
        };
        let outcome = sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
        assert_eq!(outcome.signatures_stripped, held, "{outcome:?}");
        assert_eq!(outcome.unsigned_from_signers, 0);
    }

    #[test]
    fn re_pull_does_not_double_count_dedup_by_op_id() {
        // §4.4: re-delivering already-seen ops leaves the state and op-log unchanged.
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "A-one", "alice");
        let relay = MemTransport::new();
        let mut ma = Watermarks::default();
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();

        let count_before = a.op_count();
        let state_before = materialized(&a);

        // Rewind the pull watermark and sync again → every op is re-delivered & re-applied.
        ma.pulled_through = 0;
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();

        assert_eq!(
            a.op_count(),
            count_before,
            "re-applied ops are deduped by op_id"
        );
        assert_eq!(materialized(&a), state_before, "state is unchanged");
    }

    #[test]
    fn push_does_not_echo_pulled_foreign_ops() {
        // After A pulls B's op, A's next push must NOT re-send it (site filter).
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "A-one", "alice");
        let mut b = Store::open_in_memory(2);
        b.create_item("bbbb.0001", "task", "B-one", "bob");

        let relay = MemTransport::new();
        let mut ma = Watermarks::default();
        let mut mb = Watermarks::default();
        // A pushes 3 (type/title/status), B pushes 3.
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
        sync(&mut b, 2, &stream, &mut mb, &relay, 100, &Unbounded).unwrap();
        // A pulls B's 3 ops into its store, then syncs again.
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
        let pushes_after_a_absorbed_b = *relay.pushes.borrow();
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
        assert_eq!(
            *relay.pushes.borrow(),
            pushes_after_a_absorbed_b,
            "A re-pushed nothing: pulled foreign ops are not echoed back"
        );
    }

    #[test]
    fn partial_pull_then_resync_converges() {
        // A small page limit forces multiple pull pages; re-syncing changes nothing.
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        for i in 1..=4 {
            a.create_item(&format!("aaaa.000{i}"), "task", "t", "alice");
        }
        let mut b = Store::open_in_memory(2);
        let relay = MemTransport::new();
        let mut ma = Watermarks::default();
        let mut mb = Watermarks::default();
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
        // B pulls 12 ops two-at-a-time.
        sync(&mut b, 2, &stream, &mut mb, &relay, 2, &Unbounded).unwrap();
        assert_eq!(
            materialized(&a),
            materialized(&b),
            "paginated pull converges"
        );
        // Idempotent re-sync.
        let before = materialized(&b);
        sync(&mut b, 2, &stream, &mut mb, &relay, 2, &Unbounded).unwrap();
        assert_eq!(materialized(&b), before, "re-sync is a no-op");
    }

    #[test]
    fn push_paginates_into_bounded_batches_and_converges() {
        // k64: the engine must NOT push all un-pushed local ops in one unbounded request.
        // With a small page limit it sends ceil(N/PAGE) batches, each <= PAGE ops, advancing
        // pushed_through per batch. A second replica still converges on the full set.
        let stream = StreamId("s".into());
        let relay = MemTransport::new();
        let mut a = Store::open_in_memory(1);
        // 4 creates × 3 ops each (type/title/status) = 12 local ops.
        for i in 1..=4 {
            a.create_item(&format!("aaaa.000{i}"), "task", "t", "alice");
        }
        // Derive the expected batching from the ACTUAL op count (not a hardcoded "3 ops per
        // create") so the assertion survives a change in how many ops a create emits — and a
        // page that does NOT divide the total evenly, so the trailing short batch is still
        // exercised.
        let total = local_ops_since(a.connection(), 1, 0).unwrap().len();
        let page = 5;
        assert_ne!(
            total % page,
            0,
            "fixture must exercise a trailing short batch"
        );
        let mut expected: Vec<usize> = vec![page; total / page];
        expected.push(total % page); // the trailing short batch (non-zero by the assert above)

        let mut ma = Watermarks::default();
        sync(&mut a, 1, &stream, &mut ma, &relay, page, &Unbounded).unwrap();

        let batches = relay.push_batches.borrow().clone();
        assert_eq!(batches, expected, "ceil(N/PAGE) bounded batches");
        assert!(
            batches.iter().all(|&n| n <= page),
            "no batch exceeds the page"
        );
        assert_eq!(
            ma.pushed_through, total as i64,
            "watermark advanced to the last op"
        );

        // A re-sync pushes nothing (watermark already at the end) and B converges.
        sync(&mut a, 1, &stream, &mut ma, &relay, page, &Unbounded).unwrap();
        assert_eq!(
            relay.push_batches.borrow().len(),
            expected.len(),
            "no re-push of pushed ops"
        );

        let mut b = Store::open_in_memory(2);
        let mut mb = Watermarks::default();
        sync(&mut b, 2, &stream, &mut mb, &relay, page, &Unbounded).unwrap();
        assert_eq!(
            materialized(&a),
            materialized(&b),
            "multi-batch push converges"
        );
        assert_eq!(
            b.list_items().unwrap().len(),
            4,
            "all four items crossed the relay"
        );
    }

    #[test]
    fn push_pagination_resumes_after_a_mid_stream_batch_failure() {
        // Advancing pushed_through PER batch (not once at the end) means a transport failure
        // partway through leaves the already-pushed batches durable and the watermark exact —
        // the next sync resumes from there, never re-pushing nor skipping an op.
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        for i in 1..=4 {
            a.create_item(&format!("aaaa.000{i}"), "task", "t", "alice");
        }
        // A transport that accepts the first push then fails the rest of this pass.
        struct FailAfterFirst {
            inner: MemTransport,
            allowed: RefCell<usize>,
        }
        impl Transport for FailAfterFirst {
            fn push(&self, stream: &StreamId, ops: &[WireOp]) -> Result<(), TransportError> {
                let mut left = self.allowed.borrow_mut();
                if *left == 0 {
                    return Err(TransportError("relay unreachable".into()));
                }
                *left -= 1;
                self.inner.push(stream, ops)
            }
            fn pull(
                &self,
                stream: &StreamId,
                since: Cursor,
                limit: usize,
            ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
                self.inner.pull(stream, since, limit)
            }
            fn register(
                &self,
                stream: &StreamId,
                prefix: &str,
                uuid: &str,
            ) -> Result<RegisterOutcome, TransportError> {
                self.inner.register(stream, prefix, uuid)
            }
        }
        let relay = FailAfterFirst {
            inner: MemTransport::new(),
            allowed: RefCell::new(1),
        };
        let mut ma = Watermarks::default();
        // First pass: batch 1 (5 ops) lands, batch 2 fails → the pass errors.
        assert!(sync(&mut a, 1, &stream, &mut ma, &relay, 5, &Unbounded).is_err());
        assert_eq!(
            ma.pushed_through, 5,
            "watermark advanced only through the durable batch"
        );
        assert_eq!(
            *relay.inner.pushes.borrow(),
            5,
            "exactly the first batch crossed"
        );

        // Heal the transport; the next pass pushes ONLY the remaining 7 ops.
        *relay.allowed.borrow_mut() = usize::MAX;
        sync(&mut a, 1, &stream, &mut ma, &relay, 5, &Unbounded).unwrap();
        assert_eq!(ma.pushed_through, 12);
        assert_eq!(
            *relay.inner.pushes.borrow(),
            12,
            "no op pushed twice, none skipped"
        );
    }

    #[test]
    fn a_zero_page_limit_is_clamped_not_a_panic() {
        // `sync` is public; a 0 page_limit must not panic (`chunks(0)`) — it is clamped to 1.
        // With ops to push and ops to pull, the pass still makes progress under the clamp.
        let stream = StreamId("s".into());
        let relay = MemTransport::new();
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "A-one", "alice");
        let mut ma = Watermarks::default();
        let out = sync(&mut a, 1, &stream, &mut ma, &relay, 0, &Unbounded).unwrap();
        assert_eq!(out.pushed, 3, "all local ops pushed even under a 0 page");

        let mut b = Store::open_in_memory(2);
        let mut mb = Watermarks::default();
        sync(&mut b, 2, &stream, &mut mb, &relay, 0, &Unbounded).unwrap();
        assert_eq!(materialized(&a), materialized(&b), "converges under clamp");
    }

    #[test]
    fn same_field_concurrent_edits_converge_on_the_lamport_site_winner() {
        // Both replicas edit the SAME field offline; convergence must pick the deterministic
        // (lamport, site) winner over the wire, not merely agree on "some" value.
        let stream = StreamId("s".into());
        let relay = MemTransport::new();
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "orig", "alice");
        let mut ma = Watermarks::default();
        let mut mb = Watermarks::default();
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
        let mut b = Store::open_in_memory(2);
        sync(&mut b, 2, &stream, &mut mb, &relay, 100, &Unbounded).unwrap(); // b learns the item

        // Concurrent same-field edits land at equal lamport (4); site breaks the tie. The equal
        // lamport is ASSERTED, not assumed (review of PR #482): if b's clock had advanced past a's,
        // b would win on lamport alone and this test would pass without the site half.
        assert_eq!(
            a.clock(),
            b.clock(),
            "precondition: both replicas stand at one lamport"
        );
        a.set_field("aaaa.0001", "title", Some("from-A".into()), "alice"); // (4, site 1)
        b.set_field("aaaa.0001", "title", Some("from-B".into()), "bob"); // (4, site 2)
        assert_eq!(
            a.clock(),
            b.clock(),
            "precondition: the two edits carry one lamport"
        );

        for _ in 0..2 {
            sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
            sync(&mut b, 2, &stream, &mut mb, &relay, 100, &Unbounded).unwrap();
        }

        let ta = a.get_item("aaaa.0001").unwrap().unwrap().title;
        let tb = b.get_item("aaaa.0001").unwrap().unwrap().title;
        assert_eq!(ta, tb, "both replicas converge on one title");
        assert_eq!(
            ta.as_deref(),
            Some("from-B"),
            "higher site wins the equal-lamport tie"
        );
    }

    #[test]
    fn colliding_prefixes_register_remap_then_converge_without_aliasing() {
        // bab + T-remap together (§6/§8): two replicas INDEPENDENTLY mint the SAME prefix
        // AND the same id `aaaa.0001`. Without the registry they would alias into one item
        // on merge. With Step-0 registration A keeps its prefix; B is reassigned, remaps its
        // structure AND its free-text/mention references, so they converge to DISTINCT items.
        let stream = StreamId("s".into());
        let relay = MemTransport::new();

        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "A-one", "alice");
        let mut b = Store::open_in_memory(2);
        b.create_item("aaaa.0001", "task", "B-one", "bob"); // SAME id — the collision
        b.create_item("aaaa.0002", "task", "B-two", "bob");
        // B-two cites B-one in free text + records the reference edge (so the remap must fix
        // the description via dqj AND move the mention edge, not just the bare item id).
        b.set_field(
            "aaaa.0002",
            "description",
            Some("blocked by aaaa.0001".into()),
            "bob",
        );
        b.add_edge("aaaa.0002", "aaaa.0001", EdgeKind::Mentions, "bob");

        // Step 0: register. A is free; B collides and is reassigned + locally remapped.
        assert_eq!(
            register_prefix(&mut a, &stream, "aaaa", "uuid-A", &relay).unwrap(),
            None,
            "A keeps its prefix"
        );
        let b_new = register_prefix(&mut b, &stream, "aaaa", "uuid-B", &relay)
            .unwrap()
            .expect("B is reassigned a new prefix");
        assert_ne!(b_new, "aaaa");
        // B's items were remapped locally BEFORE any merge — no more aaaa.* on B.
        assert!(
            b.get_item("aaaa.0001").unwrap().is_none()
                && b.get_item("aaaa.0002").unwrap().is_none()
        );
        let (b_one, b_two) = (format!("{b_new}.0001"), format!("{b_new}.0002"));

        // Sync to a fixed point.
        let mut ma = Watermarks::default();
        let mut mb = Watermarks::default();
        for _ in 0..2 {
            sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();
            sync(&mut b, 2, &stream, &mut mb, &relay, 100, &Unbounded).unwrap();
        }
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap();

        // Both replicas hold all THREE distinct items — A's create did not alias B's.
        assert_eq!(materialized(&a), materialized(&b), "replicas converge");
        assert_eq!(
            a.get_item("aaaa.0001").unwrap().unwrap().title.as_deref(),
            Some("A-one")
        );
        assert_eq!(
            a.get_item(&b_one).unwrap().unwrap().title.as_deref(),
            Some("B-one")
        );
        assert_eq!(
            a.list_items().unwrap().len(),
            3,
            "three distinct items, no aliasing"
        );
        // Free text + mention edge survived the remap and are resolvable on the PULLING side.
        assert_eq!(
            a.get_item(&b_two).unwrap().unwrap().description.as_deref(),
            Some(format!("blocked by {b_one}").as_str()),
            "dqj-rewritten description converges on A"
        );
        assert_eq!(
            a.mentions_of(&b_two),
            vec![b_one.clone()],
            "mention edge moved"
        );

        // Remap-before-push invariant: B's remap is Step 0, so NO op B authored (site 2) was
        // ever pushed to the relay carrying the old `aaaa.` prefix — an already-pushed op is
        // never subsequently mutated, which would otherwise re-pull a stale aliased id.
        let relay_ops = relay.streams.borrow();
        assert!(
            relay_ops[&stream.0]
                .iter()
                .filter(|o| o.site == 2)
                .all(|o| !o.target_id.contains("aaaa.")
                    && !o.value.as_deref().is_some_and(|v| v.contains("aaaa."))),
            "B never pushed an old-prefix op (remap ran before push)"
        );
    }

    #[test]
    fn registering_a_free_prefix_does_not_remap() {
        let stream = StreamId("s".into());
        let relay = MemTransport::new();
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "A", "alice");
        assert_eq!(
            register_prefix(&mut a, &stream, "aaaa", "uuid-A", &relay).unwrap(),
            None
        );
        assert!(
            a.get_item("aaaa.0001").unwrap().is_some(),
            "no remap on a free prefix"
        );
    }

    /// A transport whose `register` always reassigns a fixed prefix — used to inject a
    /// malformed value as a compromised/buggy relay would.
    struct ReassignTo(&'static str);
    impl Transport for ReassignTo {
        fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
            Ok(())
        }
        fn pull(
            &self,
            _: &StreamId,
            since: Cursor,
            _: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            Ok((vec![], since))
        }
        fn register(
            &self,
            _: &StreamId,
            _: &str,
            _: &str,
        ) -> Result<RegisterOutcome, TransportError> {
            Ok(RegisterOutcome::Reassigned {
                new_prefix: self.0.to_string(),
            })
        }
    }

    #[test]
    fn register_prefix_refuses_a_malformed_reassigned_prefix_and_does_not_remap() {
        // Integrity: a compromised relay returns a `new_prefix` carrying a '.' — splicing it
        // into ids would corrupt them. register_prefix must reject it and leave the store
        // untouched, rather than remap to a poisoned prefix.
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "A", "alice");
        let bad = ReassignTo("ev.l");
        let err = register_prefix(&mut a, &stream, "aaaa", "uuid-A", &bad).unwrap_err();
        assert!(
            matches!(err, SyncError::Transport(_)),
            "rejected as transport error"
        );
        assert!(
            a.get_item("aaaa.0001").unwrap().is_some(),
            "store untouched — no remap to a malformed prefix"
        );
    }

    /// A bare TCP server that answers exactly ONE request with `response` and hands back the raw
    /// request it received — the stand-in for a relay of some other version, at the HTTP level.
    fn answer_once(response: &'static [u8]) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            // Drain the FULL request (headers + declared body) before responding. If we
            // respond+close mid-write, ureq sees a connection-reset transport error instead
            // of a clean status — the source of a parallel-run race.
            let mut buf = Vec::new();
            let mut tmp = [0u8; 256];
            loop {
                let n = sock.read(&mut tmp).unwrap();
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(hdr_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..hdr_end]).to_lowercase();
                    let body_len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= hdr_end + 4 + body_len {
                        break; // whole request consumed
                    }
                }
            }
            sock.write_all(response).unwrap();
            String::from_utf8_lossy(&buf).into_owned()
        });
        (format!("http://{addr}"), handle)
    }

    const NOT_FOUND: &[u8] =
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    #[test]
    fn http_register_against_a_relay_without_the_route_proceeds_unregistered() {
        // Forward-compat (CQ#1): a pre-bab relay 404s on /register. The client must proceed
        // UNregistered (treated as Registered → no remap), not hard-fail the whole sync. A
        // bare TCP server that answers every request with 404 stands in for the old relay.
        let (base, handle) = answer_once(NOT_FOUND);
        let transport = HttpTransport::new(base);
        let outcome = transport
            .register(&StreamId("s".into()), "aaaa", "uuid-A")
            .expect("404 is not a transport error");
        assert_eq!(outcome, RegisterOutcome::Registered, "proceed unregistered");
        handle.join().unwrap();
    }

    fn hello() -> MachineHello {
        MachineHello {
            machine_id: "01j8machine000000000000000".into(),
            name: "Mac mini".into(),
            interval_secs: 300,
        }
    }

    #[test]
    fn a_transport_that_does_not_forward_presence_says_so_instead_of_blaming_the_relay() {
        // `announce`/`machines` are DEFAULT methods, so a Transport written before 6j6v.f0b5 — the
        // test double here, or an embedder's own wrapper — still compiles. But it must not answer
        // like a relay without the route: a wrapper that forgot to forward would then report "this
        // relay has no presence" about a relay that has it (review of PR #485, Integrity #9).
        let relay = MemTransport::new();
        let stream = StreamId("s".into());
        let announced = relay.announce(&stream, &hello()).unwrap_err();
        assert!(announced.0.contains("does not forward"), "{announced}");
        let read = machines(&relay, &stream).unwrap_err();
        assert!(read.to_string().contains("does not forward"), "{read}");
    }

    fn reporting(machines: Vec<MachineSeen>, truncated: bool) -> impl Transport {
        struct Reporting(MachinesResponse);
        impl Transport for Reporting {
            fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
                unreachable!()
            }
            fn pull(
                &self,
                _: &StreamId,
                _: Cursor,
                _: usize,
            ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
                unreachable!()
            }
            fn register(
                &self,
                _: &StreamId,
                _: &str,
                _: &str,
            ) -> Result<RegisterOutcome, TransportError> {
                unreachable!()
            }
            fn machines(&self, _: &StreamId) -> Result<Option<MachinesResponse>, TransportError> {
                Ok(Some(self.0.clone()))
            }
        }
        Reporting(MachinesResponse {
            machines,
            truncated,
        })
    }

    fn sighting(id: &str, name: &str, age_secs: u64) -> MachineSeen {
        MachineSeen {
            machine_id: id.into(),
            name: name.into(),
            last_seen: 1_790_000_000,
            age_secs,
            interval_secs: 300,
        }
    }

    #[test]
    fn a_reader_drops_what_no_honest_client_sent_and_says_the_list_is_not_whole() {
        // The relay authenticates nobody; a list is only as trustworthy as its entries. An entry an
        // up-to-date relay would have refused on the way in — a spoofed name, a malformed id — is
        // left out, and so is anything past the listing cap, and the reading says it is partial.
        let mut flood: Vec<MachineSeen> = vec![
            sighting("real", "MacBook", 5),
            sighting("spoof", "MacBook\u{202E}", 5),
            sighting("bad id", "Mac mini", 5),
        ];
        let Presence::Reported {
            machines: list,
            truncated,
        } = machines(&reporting(flood.clone(), false), &StreamId("s".into())).unwrap()
        else {
            panic!("a relay that answered reported presence");
        };
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].machine_id, "real");
        assert!(truncated, "entries were dropped, so the list is not whole");

        flood = (0..=presence::MAX_LISTED_MACHINES)
            .map(|i| sighting(&format!("m{i}"), "Spam", 5))
            .collect();
        let Presence::Reported {
            machines: list,
            truncated,
        } = machines(&reporting(flood, false), &StreamId("s".into())).unwrap()
        else {
            panic!("reported");
        };
        assert_eq!(list.len(), presence::MAX_LISTED_MACHINES);
        assert!(truncated);

        let Presence::Reported { truncated, .. } = machines(
            &reporting(vec![sighting("real", "MacBook", 5)], true),
            &StreamId("s".into()),
        )
        .unwrap() else {
            panic!("reported");
        };
        assert!(truncated, "the relay's own flag is kept");
    }

    #[test]
    fn machines_judges_every_sighting_the_relay_reported() {
        let relay = reporting(
            vec![
                sighting("awake", "AWAKE", 5),
                sighting("asleep", "ASLEEP", 5_000),
            ],
            false,
        );
        let Presence::Reported {
            machines: list,
            truncated,
        } = machines(&relay, &StreamId("s".into())).unwrap()
        else {
            panic!("a relay that answered reported presence");
        };
        let verdicts: Vec<(&str, bool)> = list
            .iter()
            .map(|m| (m.machine_id.as_str(), m.online))
            .collect();
        assert_eq!(verdicts, [("awake", true), ("asleep", false)], "order kept");
        assert!(!truncated);
    }

    #[test]
    fn http_announce_posts_the_hello_to_the_stream_s_machines_route() {
        let (base, handle) = answer_once(
            b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let announced = HttpTransport::new(base)
            .announce(&StreamId("stream-1".into()), &hello())
            .unwrap();
        assert_eq!(announced, Announced::Recorded);
        let request = handle.join().unwrap();
        assert!(
            request.starts_with("POST /streams/stream-1/machines "),
            "{request}"
        );
        assert!(
            request.contains(r#""machine_id":"01j8machine000000000000000""#),
            "{request}"
        );
        assert!(request.contains(r#""interval_secs":300"#), "{request}");
    }

    #[test]
    fn http_presence_against_a_relay_without_the_route_is_unsupported_not_an_error() {
        // A relay older than 6j6v.f0b5 (manufakt.io's runs v0.35.0) has no /machines route and
        // answers 404. Announcing must not fail the service's pass, and reading must not fail the
        // verb — both say "this relay has no presence".
        let (base, handle) = answer_once(NOT_FOUND);
        assert_eq!(
            HttpTransport::new(base)
                .announce(&StreamId("s".into()), &hello())
                .unwrap(),
            Announced::Unsupported
        );
        handle.join().unwrap();

        let (base, handle) = answer_once(NOT_FOUND);
        assert_eq!(
            machines(&HttpTransport::new(base), &StreamId("s".into())).unwrap(),
            Presence::Unsupported
        );
        assert!(handle
            .join()
            .unwrap()
            .starts_with("GET /streams/s/machines "));

        // axum answers 405, not 404, when a path exists for some OTHER method — the shape a relay
        // that grew only one half of the route would have. Same meaning.
        let (base, handle) = answer_once(
            b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(
            HttpTransport::new(base)
                .announce(&StreamId("s".into()), &hello())
                .unwrap(),
            Announced::Unsupported
        );
        handle.join().unwrap();
    }

    #[test]
    fn http_presence_on_a_server_error_is_a_transport_error_not_a_quiet_unsupported() {
        const BROKEN: &[u8] =
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let (base, handle) = answer_once(BROKEN);
        assert!(HttpTransport::new(base)
            .announce(&StreamId("s".into()), &hello())
            .is_err());
        handle.join().unwrap();

        // A refusal carries the relay's own reason, not just "status code 400" — it is the one line
        // an operator has to go on when a machine never appears.
        let (base, handle) = answer_once(
            b"HTTP/1.1 400 Bad Request\r\nContent-Length: 33\r\nConnection: close\r\n\r\ninterval_secs must be at most 1h.",
        );
        let refused = HttpTransport::new(base)
            .announce(&StreamId("s".into()), &hello())
            .unwrap_err();
        assert!(refused.0.contains("400"), "{refused}");
        assert!(
            refused.0.contains("interval_secs must be at most 1h"),
            "{refused}"
        );
        handle.join().unwrap();

        let (base, handle) = answer_once(BROKEN);
        assert!(machines(&HttpTransport::new(base), &StreamId("s".into())).is_err());
        handle.join().unwrap();
    }

    #[test]
    fn http_machines_reads_the_relay_s_list() {
        const BODY: &str = r#"{"machines":[{"machine_id":"m1","name":"MacBook","last_seen":1790000000,"age_secs":7,"interval_secs":300}]}"#;
        let response: &'static [u8] = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                BODY.len()
            )
            .into_bytes()
            .into_boxed_slice(),
        );
        let (base, handle) = answer_once(response);
        let Presence::Reported { machines: list, .. } =
            machines(&HttpTransport::new(base), &StreamId("s".into())).unwrap()
        else {
            panic!("a 200 is a report");
        };
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "MacBook");
        assert!(list[0].online);
        handle.join().unwrap();
    }

    #[test]
    fn an_already_registered_replica_is_never_remapped_after_pushing() {
        // Locks the remap-before-push invariant from the other side: once a replica is
        // Registered (kept its prefix) and has pushed, re-running Step 0 returns None — its
        // already-pushed ops are never subsequently mutated by a remap.
        let stream = StreamId("s".into());
        let relay = MemTransport::new();
        let mut a = Store::open_in_memory(1);
        a.create_item("aaaa.0001", "task", "A", "alice");

        assert_eq!(
            register_prefix(&mut a, &stream, "aaaa", "uuid-A", &relay).unwrap(),
            None
        );
        let mut ma = Watermarks::default();
        sync(&mut a, 1, &stream, &mut ma, &relay, 100, &Unbounded).unwrap(); // A pushes its ops

        // Re-running Step 0 (e.g. on the next sync) must NOT reassign or remap.
        assert_eq!(
            register_prefix(&mut a, &stream, "aaaa", "uuid-A", &relay).unwrap(),
            None,
            "a registered replica stays put"
        );
        assert!(
            a.get_item("aaaa.0001").unwrap().is_some(),
            "no remap after push"
        );
    }

    #[test]
    fn local_ops_since_carries_the_op_domain_not_a_hardcoded_task() {
        // The push path reads local ops straight from the `ops` table into WireOps. It must carry
        // each op's real `domain` (aye.1.4) so a future product sharing this replica's site never
        // has its fact/message ops silently relabelled `task` on the way to the relay — the same
        // forward-compat hole `From<WireOp>` had on the pull side.
        let mut s = Store::open_in_memory(2);
        // A non-task op minted at THIS replica's site (2). `apply` is the only way to introduce a
        // foreign-domain op; the matching site makes the push path's `site = local_site` select it.
        let fact = Op {
            op_id: "01J0FACT00000000000000000".into(),
            lamport: 1,
            site: 2,
            domain: "fact".into(),
            target_kind: "fact".into(),
            target_id: "f.1".into(),
            field: "body".into(),
            op_type: "set".into(),
            value: Some("hello".into()),
            author: "u".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };
        s.apply(std::slice::from_ref(&fact));
        let pushed = local_ops_since(s.connection(), 2, 0).unwrap();
        assert_eq!(pushed.len(), 1, "the local-site op is selected for push");
        assert_eq!(
            pushed[0].1.domain, "fact",
            "the push path carries the real domain, not a hardcoded task"
        );
    }

    #[test]
    fn unknown_op_shape_pulled_over_the_wire_is_stored_not_folded() {
        // a2p / §7 cross-version guarantee, end to end: a newer peer pushes an op whose
        // shape this build does not understand. Pulling it must NOT panic; the op is kept
        // in the local log (resurface stays possible) but not materialized.
        let stream = StreamId("s".into());
        let relay = MemTransport::new();
        // A future op direct on the relay (envelope_version + op_type from a newer build).
        let future = WireOp {
            envelope_version: ENVELOPE_VERSION + 7,
            op_id: "01HFUTURE0000000000000000".into(),
            lamport: 1,
            site: 99,
            domain: "task".into(),
            target_kind: "gizmo".into(),
            target_id: "zz99.0001".into(),
            field: "sparkle".into(),
            op_type: "future_set".into(),
            value: Some("v".into()),
            author: "newpeer".into(),
            wall_clock: String::new(),
            extra: Default::default(),
        };
        relay.push(&stream, std::slice::from_ref(&future)).unwrap();

        let mut b = Store::open_in_memory(2);
        let mut mb = Watermarks::default();
        let outcome = sync(&mut b, 2, &stream, &mut mb, &relay, 100, &Unbounded).unwrap();

        assert_eq!(outcome.pulled, 1, "the future op was pulled");
        assert_eq!(b.op_count(), 1, "stored in the local log (not dropped)");
        assert!(materialized(&b).is_empty(), "but not folded into the views");
    }

    /// A transport whose `pull` NEVER reports the stream exhausted: every call returns a full
    /// page of freshly-minted (but otherwise valid-looking) ops and an advancing cursor, exactly
    /// what a buggy or hostile relay would do. Without a page ceiling this wedges `sync`'s pull
    /// loop forever.
    struct EndlessPull {
        page_limit: usize,
    }
    impl Transport for EndlessPull {
        fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
            Ok(())
        }
        fn pull(
            &self,
            _: &StreamId,
            since: Cursor,
            limit: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            assert_eq!(
                limit, self.page_limit,
                "engine always asks for its own page size"
            );
            let page: Vec<WireOp> = (0..limit)
                .map(|i| WireOp {
                    envelope_version: ENVELOPE_VERSION,
                    op_id: format!("01ENDLESS{:017}", since.0 as usize + i),
                    lamport: 1,
                    site: 99,
                    domain: "task".into(),
                    target_kind: "task".into(),
                    target_id: "zzzz.0001".into(),
                    field: "title".into(),
                    op_type: "set".into(),
                    value: Some("x".into()),
                    author: "relay".into(),
                    wall_clock: String::new(),
                    extra: Default::default(),
                })
                .collect();
            let next = Cursor(since.0 + limit as i64); // always advances — never signals "done"
            Ok((page, next))
        }
        fn register(
            &self,
            _: &StreamId,
            _: &str,
            _: &str,
        ) -> Result<RegisterOutcome, TransportError> {
            Ok(RegisterOutcome::Registered)
        }
    }

    #[test]
    fn a_relay_that_never_exhausts_the_pull_stream_is_capped_not_wedged() {
        // Integrity #3 (PR #263): a hard per-pass page ceiling must terminate the pull loop even
        // when the relay keeps handing back full pages with an advancing cursor forever. The
        // pass must still return Ok (a clean stop, not an error), report the ceiling was hit, and
        // have advanced the watermark through everything it did pull — so the NEXT pass simply
        // resumes rather than losing progress or re-pulling what already landed.
        //
        // Drives `sync_with_pull_ceiling` directly with a SMALL ceiling rather than looping the
        // real `PULL_PAGE_CEILING` (20,000 pages) — the mechanism being proven (the loop stops
        // exactly at the ceiling, cleanly, with the watermark caught up) does not depend on which
        // number trips it, and looping the production value would just make this test slow.
        let stream = StreamId("s".into());
        let page = 10;
        let ceiling = 3;
        let relay = EndlessPull { page_limit: page };
        let mut a = Store::open_in_memory(1);
        let mut ma = Watermarks::default();

        let outcome = sync_with_pull_ceiling(
            &mut a, 1, &stream, &mut ma, &relay, page, ceiling, &Unbounded,
        )
        .unwrap();

        assert!(outcome.pull_ceiling_hit, "the ceiling trip is surfaced");
        assert_eq!(
            outcome.pulled,
            ceiling * page,
            "pulled exactly ceiling-many pages, not an unbounded amount"
        );
        assert_eq!(
            ma.pulled_through,
            (ceiling * page) as i64,
            "the watermark advanced through everything actually pulled, ready to resume"
        );

        // A follow-up pass (the daemon's next tick, or a re-run of `nxs sync run`) resumes from
        // exactly that watermark rather than re-pulling or losing anything, and this time the
        // (still-endless) relay is capped again — proving the resume is real, not just that the
        // first call stopped.
        let outcome2 = sync_with_pull_ceiling(
            &mut a, 1, &stream, &mut ma, &relay, page, ceiling, &Unbounded,
        )
        .unwrap();
        assert!(outcome2.pull_ceiling_hit);
        assert_eq!(
            ma.pulled_through,
            (2 * ceiling * page) as i64,
            "the second pass continued from where the first stopped"
        );
    }

    // ---- the wall-clock budget (6j6v.25f6) -------------------------------------------------

    /// A relay that answers every request with NOTHING and nudges the cursor by one.
    ///
    /// This is the shape the page ceiling does not really answer. Full pages with an advancing
    /// cursor cost the relay something; empty pages with a one-step cursor cost it nothing, and
    /// PR #311's termination change (stop when the cursor stands still, rather than on a short
    /// page) is what made them enough. Twenty thousand of these against a 30-second read timeout is
    /// over 160 hours in ONE pass, on a single-threaded service that sweeps every workspace in
    /// turn.
    struct TricklingRelay;

    impl Transport for TricklingRelay {
        fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
            Ok(())
        }
        fn pull(
            &self,
            _: &StreamId,
            since: Cursor,
            _: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            Ok((Vec::new(), Cursor(since.0 + 1)))
        }
        fn register(
            &self,
            _: &StreamId,
            _: &str,
            _: &str,
        ) -> Result<RegisterOutcome, TransportError> {
            Ok(RegisterOutcome::Registered)
        }
    }

    /// A relay with nothing in it: the cursor stands still, which is the one true stop.
    struct EmptyRelay;

    impl Transport for EmptyRelay {
        fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
            Ok(())
        }
        fn pull(
            &self,
            _: &StreamId,
            since: Cursor,
            _: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            Ok((Vec::new(), since))
        }
        fn register(
            &self,
            _: &StreamId,
            _: &str,
            _: &str,
        ) -> Result<RegisterOutcome, TransportError> {
            Ok(RegisterOutcome::Registered)
        }
    }

    /// A budget that answers out of a COUNTER, not a clock — which is the whole reason
    /// [`PassBudget`] is a question rather than a deadline. The bound is proven with no real time
    /// passing at all (the DoD asks for exactly that), and the engine cannot tell the difference.
    struct AfterPages(std::cell::Cell<usize>);

    impl AfterPages {
        fn new(n: usize) -> AfterPages {
            AfterPages(std::cell::Cell::new(n))
        }
    }

    impl PassBudget for AfterPages {
        fn exhausted(&self) -> bool {
            let left = self.0.get();
            if left == 0 {
                return true;
            }
            self.0.set(left - 1);
            false
        }
    }

    #[test]
    fn a_relay_that_trickles_forever_is_stopped_by_the_budget_long_before_the_page_ceiling() {
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        let mut ma = Watermarks::default();
        let budget = AfterPages::new(5);

        // The production ceiling would allow 20,000 round-trips here.
        let outcome = sync_with_pull_ceiling(
            &mut a,
            1,
            &stream,
            &mut ma,
            &TricklingRelay,
            10,
            20_000,
            &budget,
        )
        .unwrap();

        assert!(outcome.budget_exhausted, "the budget is what stopped it");
        assert!(
            !outcome.pull_ceiling_hit,
            "and it stopped long before the page ceiling: {outcome:?}"
        );
        assert_eq!(
            outcome.pull_pages, 6,
            "five pages of budget plus the one that spent it"
        );
        assert_eq!(
            outcome.pulled, 0,
            "the relay delivered nothing, which is the point"
        );
    }

    #[test]
    fn a_budgeted_stop_keeps_its_progress_so_the_next_pass_resumes() {
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        let mut ma = Watermarks::default();

        sync_with_pull_ceiling(
            &mut a,
            1,
            &stream,
            &mut ma,
            &TricklingRelay,
            10,
            20_000,
            &AfterPages::new(3),
        )
        .unwrap();
        let after_first = ma.pulled_through;
        assert!(after_first > 0, "the watermark moved with the cursor");

        sync_with_pull_ceiling(
            &mut a,
            1,
            &stream,
            &mut ma,
            &TricklingRelay,
            10,
            20_000,
            &AfterPages::new(3),
        )
        .unwrap();
        assert!(
            ma.pulled_through > after_first,
            "the second pass continued from where the first stopped, not from zero"
        );
    }

    #[test]
    fn a_pass_that_finishes_normally_reports_neither_limit() {
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        let mut ma = Watermarks::default();
        // The cursor stands still on the first page — the one true stop.
        let relay = EmptyRelay;
        let outcome =
            sync_with_pull_ceiling(&mut a, 1, &stream, &mut ma, &relay, 10, 20_000, &Unbounded)
                .unwrap();
        assert!(!outcome.budget_exhausted);
        assert!(!outcome.pull_ceiling_hit);
        assert_eq!(outcome.pull_pages, 0);
        assert_eq!(outcome.pull_empty_pages, 0);
    }

    #[test]
    fn an_empty_page_that_still_moves_the_cursor_is_counted_as_empty() {
        let stream = StreamId("s".into());
        let mut a = Store::open_in_memory(1);
        let mut ma = Watermarks::default();
        let outcome = sync_with_pull_ceiling(
            &mut a,
            1,
            &stream,
            &mut ma,
            &TricklingRelay,
            10,
            20_000,
            &AfterPages::new(4),
        )
        .unwrap();
        assert_eq!(
            (outcome.pull_pages, outcome.pull_empty_pages),
            (5, 5),
            "every page delivered nothing — which is what tells a trickle from a backlog"
        );
    }

    #[test]
    fn a_full_page_relay_reports_no_empty_pages_so_a_real_backlog_is_not_mistaken_for_a_trickle() {
        let stream = StreamId("s".into());
        let page = 10;
        let relay = EndlessPull { page_limit: page };
        let mut a = Store::open_in_memory(1);
        let mut ma = Watermarks::default();
        let outcome =
            sync_with_pull_ceiling(&mut a, 1, &stream, &mut ma, &relay, page, 4, &Unbounded)
                .unwrap();
        assert_eq!(outcome.pull_pages, 4);
        assert_eq!(
            outcome.pull_empty_pages, 0,
            "an enormous stream is not a wedged relay, and must not be reported as one"
        );
    }

    /// A relay that serves a SCOPED view of the stream: it scans the whole log but returns only
    /// the ops this reader may see, reporting `next` as the seq it scanned THROUGH. That is the
    /// shape the E4 auth slice (6j6v.6aza) adds — an ACL filter by kind/channel keyed on the
    /// authenticated identity — and forward-compat invariant 4 (6j6v.xsf3) is exactly the demand
    /// that adding it needs no client rewrite.
    ///
    /// `visible` picks which ops this reader may see; everything else is scanned and skipped.
    struct ScopedRelay {
        log: Vec<WireOp>,
        visible: fn(&WireOp) -> bool,
    }
    impl Transport for ScopedRelay {
        fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
            Ok(())
        }
        fn pull(
            &self,
            _: &StreamId,
            since: Cursor,
            limit: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            let start = since.0 as usize;
            let scanned = self.log.iter().skip(start).take(limit);
            let page: Vec<WireOp> = scanned.filter(|op| (self.visible)(op)).cloned().collect();
            // Scanned-through, NOT last-returned: the page may be empty or short while the stream
            // still holds visible ops further along.
            let next = Cursor(self.log.len().min(start + limit) as i64);
            Ok((page, next))
        }
        fn register(
            &self,
            _: &StreamId,
            _: &str,
            _: &str,
        ) -> Result<RegisterOutcome, TransportError> {
            Ok(RegisterOutcome::Registered)
        }
    }

    fn foreign_op(n: usize, target_kind: &str) -> WireOp {
        WireOp {
            envelope_version: ENVELOPE_VERSION,
            op_id: format!("01SCOPED{:018}", n),
            lamport: n as i64 + 1,
            site: 99,
            domain: "task".into(),
            target_kind: target_kind.into(),
            target_id: format!("zzzz.{n:04}"),
            field: "title".into(),
            op_type: "set".into(),
            value: Some(format!("v{n}")),
            author: "peer".into(),
            wall_clock: String::new(),
            extra: Default::default(),
        }
    }

    #[test]
    fn a_scoped_read_drains_past_a_page_the_reader_may_not_see() {
        // Invariant 4 (6j6v.xsf3): the pull loop must not encode "whole log to everyone". The
        // relay here hides an entire page's worth of ops from this reader, with visible ops
        // BEHIND them. A loop that reads "empty page" or "short page" as "stream exhausted"
        // stops at the hidden run and never reaches what follows — and because an empty page
        // also left the watermark unmoved, every later pass would re-scan the same hidden run
        // forever. Terminating on "the cursor did not advance" is what makes a later ACL filter
        // a server-side change instead of a client rewrite.
        let stream = StreamId("s".into());
        let page = 4;
        // Pages of 4: [visible, visible, hidden, hidden] [hidden x4] [visible, visible]
        let mut log: Vec<WireOp> = vec![foreign_op(0, "item"), foreign_op(1, "item")];
        log.extend((2..8).map(|n| foreign_op(n, "secret")));
        log.extend([foreign_op(8, "item"), foreign_op(9, "item")]);
        let relay = ScopedRelay {
            log,
            visible: |op| op.target_kind == "item",
        };

        let mut a = Store::open_in_memory(1);
        let mut ma = Watermarks::default();
        let outcome = sync(&mut a, 1, &stream, &mut ma, &relay, page, &Unbounded).unwrap();

        assert_eq!(
            outcome.pulled, 4,
            "every op this reader may see arrived — including the two behind the hidden run"
        );
        assert_eq!(
            ma.pulled_through, 10,
            "the watermark tracks what the relay SCANNED, so the next pass never re-reads the \
             hidden run"
        );
        assert!(!outcome.pull_ceiling_hit, "a clean, exhausted stop");
    }

    #[test]
    fn a_fully_hidden_stream_terminates_and_does_not_re_scan() {
        // The degenerate case of the same invariant: this reader may see NOTHING. Every page is
        // empty, yet the pass must still terminate (not spin) and still record how far the relay
        // scanned, so a later grant of access does not re-read the whole history.
        let stream = StreamId("s".into());
        let relay = ScopedRelay {
            log: (0..7).map(|n| foreign_op(n, "secret")).collect(),
            visible: |_| false,
        };
        let mut a = Store::open_in_memory(1);
        let mut ma = Watermarks::default();

        let outcome = sync(&mut a, 1, &stream, &mut ma, &relay, 3, &Unbounded).unwrap();

        assert_eq!(outcome.pulled, 0, "nothing visible, nothing folded");
        assert_eq!(ma.pulled_through, 7, "but the scan position is durable");
        assert!(!outcome.pull_ceiling_hit);
    }

    #[test]
    fn encode_segment_keeps_unreserved_and_escapes_everything_else() {
        // RFC 3986 §2.3 unreserved set survives verbatim; everything else becomes %XX.
        assert_eq!(encode_segment("stream-3f9a2c1d"), "stream-3f9a2c1d");
        assert_eq!(encode_segment("a~b._-"), "a~b._-");
        assert_eq!(
            encode_segment("nxsflow/manufakt-io"),
            "nxsflow%2Fmanufakt-io"
        );
        assert_eq!(encode_segment("a?b"), "a%3Fb");
        assert_eq!(encode_segment("a#b"), "a%23b");
        assert_eq!(encode_segment("a b"), "a%20b");
        assert_eq!(
            encode_segment("\u{fc}"),
            "%C3%BC",
            "multi-byte UTF-8 encodes per byte"
        );
    }

    #[test]
    fn stream_urls_percent_encode_the_id_segment() {
        let t = HttpTransport::new("http://127.0.0.1:8787");
        assert_eq!(
            t.url(&StreamId("nxsflow/manufakt-io".into()), "ops"),
            "http://127.0.0.1:8787/streams/nxsflow%2Fmanufakt-io/ops",
        );
        assert_eq!(
            t.url(&StreamId("stream-3f9a".into()), "register"),
            "http://127.0.0.1:8787/streams/stream-3f9a/register",
        );
    }

    /// An HTTP stand-in that answers `responses.len()` requests in turn with the raw responses given
    /// and hands back each request's head — enough to see what actually went over the wire.
    fn stub_relay(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut heads = Vec::new();
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                }
                stream.write_all(response.as_bytes()).unwrap();
                heads.push(head);
            }
            heads
        });
        (addr.to_string(), handle)
    }

    fn json_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// One request answered with `body` as JSON — the common case.
    fn one_request_relay(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let (addr, heads) = stub_relay(vec![json_response("200 OK", body)]);
        let handle = std::thread::spawn(move || heads.join().unwrap().remove(0));
        (addr, handle)
    }

    /// Taking the userinfo off the URL must not take it off the REQUEST (6j6v.q3kk): a relay behind
    /// a gateway authenticates on it. The header is exactly what ureq built from the userinfo
    /// before — `Basic` over `user:password`, percent-encoding kept — so such a relay cannot tell.
    #[test]
    fn a_credential_in_the_url_still_reaches_the_relay_as_the_basic_header_ureq_would_send() {
        let (addr, relay) = one_request_relay(r#"{"ops":[],"next":0}"#);
        let t = HttpTransport::new(format!("http://relay-user:s3cr%40t@{addr}"));
        t.pull(&StreamId("s".into()), Cursor(0), 10)
            .expect("the stand-in answers");
        let head = relay.join().unwrap();
        assert!(
            head.lines().any(
                |l| l.eq_ignore_ascii_case("authorization: Basic cmVsYXktdXNlcjpzM2NyJTQwdA==")
            ),
            "base64(\"relay-user:s3cr%40t\") must go out as the Authorization header:\n{head}"
        );
        assert!(
            head.starts_with("GET /streams/s/ops?"),
            "and the request line is the bare path:\n{head}"
        );
    }

    /// No credential in the URL, no header — a relay without a gateway sees what it always saw.
    #[test]
    fn a_url_without_userinfo_sends_no_authorization_header() {
        let (addr, relay) = one_request_relay(r#"{"ops":[],"next":0}"#);
        let t = HttpTransport::new(format!("http://{addr}"));
        t.pull(&StreamId("s".into()), Cursor(0), 10).unwrap();
        let head = relay.join().unwrap();
        assert!(
            !head.to_ascii_lowercase().contains("authorization"),
            "{head}"
        );
    }

    /// The failure half (6j6v.q3kk): an unreachable relay behind a URL with userinfo names host,
    /// port and path, and neither half of the key — through every route of the transport.
    #[test]
    fn a_transport_error_names_the_relay_but_never_its_credentials() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let t = HttpTransport::new(format!("http://relay-user:s3cret-key@127.0.0.1:{port}"));
        let stream = StreamId("s".into());
        let hello = MachineHello {
            machine_id: "m".into(),
            name: "n".into(),
            interval_secs: 300,
        };
        let errors = [
            (
                "pull",
                t.pull(&stream, Cursor(0), 10).err().map(|e| e.to_string()),
            ),
            ("push", t.push(&stream, &[]).err().map(|e| e.to_string())),
            (
                "register",
                t.register(&stream, "abcd", "u")
                    .err()
                    .map(|e| e.to_string()),
            ),
            (
                "announce",
                t.announce(&stream, &hello).err().map(|e| e.to_string()),
            ),
            ("machines", t.machines(&stream).err().map(|e| e.to_string())),
        ];
        for (route, message) in errors {
            let message = message.unwrap_or_else(|| panic!("{route} reached a closed port"));
            assert!(
                !message.contains("relay-user") && !message.contains("s3cret-key"),
                "{route} leaks the credential: {message}"
            );
            assert!(
                message.contains(&format!("127.0.0.1:{port}/streams/s/")),
                "{route} must still say which relay and which path failed: {message}"
            );
        }
    }

    /// The one URL that cannot be split (6j6v.q3kk): too malformed to parse, so it keeps its text
    /// and gets no header. ureq refuses it on the first request, and the refusal must not quote the
    /// credential either.
    #[test]
    fn an_unparseable_url_with_userinfo_is_refused_without_quoting_its_credential() {
        let t = HttpTransport::new("http://relay-user:s3cret-key@[relay.example/x");
        let message = t
            .pull(&StreamId("s".into()), Cursor(0), 10)
            .expect_err("ureq refuses an unparseable URL")
            .to_string();
        assert!(
            !message.contains("relay-user") && !message.contains("s3cret-key"),
            "{message}"
        );
    }

    /// `failed` on its own: every form of the credential goes, wherever a message quotes it.
    #[test]
    fn every_form_of_the_credential_is_scrubbed_from_an_error() {
        let t = HttpTransport::new("http://relay-user:s3cr%40t-key@relay.example");
        let payload = "cmVsYXktdXNlcjpzM2NyJTQwdC1rZXk="; // base64("relay-user:s3cr%40t-key")
        let e = t.failed(format!(
            "http://relay-user:s3cr%40t-key@relay.example/streams/s/ops: refused; the gateway saw \
             Authorization: Basic {payload}, i.e. relay-user:s3cr@t-key"
        ));
        let text = e.to_string();
        for leaked in [payload, "relay-user", "s3cr%40t-key", "s3cr@t-key"] {
            assert!(!text.contains(leaked), "{leaked} in: {text}");
        }
        assert!(text.contains("relay.example/streams/s/ops"), "{text}");
    }

    /// A relay (or a gateway in front of it) that echoes the request's `Authorization` header into
    /// its refusal hands the key back as base64 — which no URL rule can see (review of PR #487,
    /// Integrity #2).
    #[test]
    fn a_refusal_that_echoes_the_authorization_header_does_not_bring_the_key_back() {
        let (addr, relay) = stub_relay(vec![json_response(
            "400 Bad Request",
            "rejected Authorization: Basic cmVsYXktdXNlcjpzM2NyZXQta2V5",
        )]);
        let t = HttpTransport::new(format!("http://relay-user:s3cret-key@{addr}"));
        let hello = MachineHello {
            machine_id: "m".into(),
            name: "n".into(),
            interval_secs: 300,
        };
        let message = t
            .announce(&StreamId("s".into()), &hello)
            .expect_err("the stand-in refuses")
            .to_string();
        relay.join().unwrap();
        assert!(
            !message.contains("cmVsYXktdXNlcjpzM2NyZXQta2V5") && !message.contains("s3cret-key"),
            "{message}"
        );
        assert!(message.contains("status code 400"), "{message}");
    }

    /// ureq sends no header for an empty userinfo, so neither does the transport.
    #[test]
    fn an_empty_userinfo_sends_no_authorization_header() {
        let (addr, relay) = one_request_relay(r#"{"ops":[],"next":0}"#);
        let t = HttpTransport::new(format!("http://@{addr}"));
        t.pull(&StreamId("s".into()), Cursor(0), 10).unwrap();
        let head = relay.join().unwrap();
        assert!(
            !head.to_ascii_lowercase().contains("authorization"),
            "{head}"
        );
    }

    /// A redirect to the same host keeps the credential, as ureq's userinfo did for a relative
    /// `Location` — an explicit header would otherwise be dropped on every redirect.
    #[test]
    fn a_redirect_to_the_same_host_still_carries_the_credential() {
        let (addr, relay) = stub_relay(vec![
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: /moved/streams/s/ops?since=0&limit=10\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n"
                .to_string(),
            json_response("200 OK", r#"{"ops":[],"next":0}"#),
        ]);
        let t = HttpTransport::new(format!("http://relay-user:s3cret-key@{addr}"));
        t.pull(&StreamId("s".into()), Cursor(0), 10)
            .expect("followed");
        let heads = relay.join().unwrap();
        assert!(heads[1].starts_with("GET /moved/"), "{heads:?}");
        assert!(
            heads[1]
                .lines()
                .any(|l| l.to_ascii_lowercase().starts_with("authorization: basic ")),
            "the redirected request is authenticated too: {heads:?}"
        );
    }
}
