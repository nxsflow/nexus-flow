//! The agent transcript store (nxf epic 6wt2, ticket f8c9): the durable half of the Claude Agent
//! SDK stream capture. The sidecar's normalizer (`agent-sidecar/src/transcript.mjs`) coalesces one
//! SDK session into wire entries and pipes them to `nxc transcript append`; this module parses them,
//! assigns `seq`, and folds them into the plain `agent_transcript` table.
//!
//! Shaped exactly like [`crate::session_map`]: an `impl ChatStore` block over plain SQL, NOT a
//! reducer. `agent_transcript` is device-local and never op-folded — see the DDL comment in
//! `schema.rs` for the full reasoning. Nothing here touches the `ops` log.
//!
//! The wire contract (load-bearing in BOTH directions — the sidecar emits exactly this):
//!
//! ```json
//! { "kind": "tool_use", "at": "2026-07-26T07:42:39.123Z", "toolUseId": "toolu_abc",
//!   "parentToolUseId": "toolu_parent", "subagentType": "code-reviewer", "data": { } }
//! ```
//!
//! camelCase on the wire, snake_case in Rust. `kind` and `data` are required; every tag key is
//! optional and ABSENT rather than null when it does not apply. `seq` is deliberately not on the
//! wire — the store assigns it, so a `resume` that starts a second sidecar process for the same
//! internal session continues the transcript instead of colliding with it.

use rusqlite::params;
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::error::{NxfError, Result};
use crate::store::ChatStore;

/// One parsed wire entry, seq-less (the store assigns `seq` on append).
///
/// `data` stays a [`serde_json::Value`] on purpose: it is the kind-specific payload and it is
/// **opaque to the store** — the consumer (T3's nested read) owns its shape, so a new `kind` or a
/// new payload field never needs a change here. It also means the `skip_serializing_if` attrs below
/// govern the four top-level TAG keys and nothing else: absent-vs-null *inside* `data` passes
/// through verbatim either way, because a [`serde_json::Value`] round-trips whatever it was given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    /// `session_init` | `assistant` | `thinking` | `tool_use` | `tool_result` | `result` | `error`.
    /// Not a Rust enum: the sidecar owns the vocabulary, and an unknown kind from a newer sidecar
    /// must still be STORED (it is evidence), not rejected. Emptiness is the one thing rejected —
    /// see [`TranscriptEntry::validate`].
    pub kind: String,
    /// ISO-8601 UTC from the sidecar's own clock, stamped when the block OPENED.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// On `tool_use` this call's own id; on `tool_result` the id it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    /// Absent ⇒ the main conversation; present ⇒ this entry happened inside a Task-spawned
    /// subagent, and the value is the spawning Task `tool_use` id. This is what makes the nested
    /// sub-timeline reconstructible — beads dropped subagent entries on its persist path and lost
    /// them on reload; here they are first-class rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
    /// The subagent's type, when the SDK reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_type: Option<String>,
    /// The kind-specific payload. Opaque — stored as JSON text, never inspected.
    pub data: serde_json::Value,
}

impl TranscriptEntry {
    /// The one structural invariant the store enforces. `kind` is the only field a reader keys off
    /// to interpret `data` at all, so an empty one makes the row uninterpretable forever — a loud
    /// `validation` error, never a silent skip or a silent empty-string row. Everything else is
    /// accepted as-is: the store is a recorder, and rejecting evidence it merely does not recognise
    /// would defeat the point.
    ///
    /// The message is a bare PREDICATE ("has an empty …"), never a whole sentence: every caller
    /// prefixes the coordinate its own input is framed in — `append_transcript` a batch index, the
    /// CLI a stdin line number — and both must call THIS rather than re-implementing the check, so
    /// a second invariant added here is reported with the right coordinate on both paths.
    pub(crate) fn validate(&self) -> Result<()> {
        if self.kind.trim().is_empty() {
            return Err(NxfError::validation("has an empty `kind`"));
        }
        Ok(())
    }
}

// ---- retention (nxf 6j6v.t7pa) -------------------------------------------------------------
//
// `agent_transcript` used to have no lever at all: `append_transcript`'s doc comment and
// `facade::transcript`'s both named retention as the thing to reach for "if that ever bites", and
// neither pointed at anything. This is that lever.
//
// ## What it deletes, and how it decides
//
// WHOLE SESSIONS, keyed on the session's LAST recorded activity (`MAX(at)`) — never individual
// rows. Two independent reasons, and either alone would settle it:
//
//   1. A transcript cut in half reads back indistinguishable from a complete one. That is the same
//      argument `append_transcript` makes for batch atomicity, and it applies with more force here:
//      a torn batch at least fails loudly at the time, whereas a torn *history* is silent forever.
//   2. A session's early rows are the part that explains its late ones. Age-by-row would delete a
//      long-running session's `session_init` and its first tool calls while keeping the tail that
//      makes no sense without them.
//
// ## Why age, and not a session count
//
// The complaint on the ticket is that the table is unbounded across the workspace's LIFETIME. An
// age window bounds it to whatever a fixed window of work produces, which is exactly that property;
// a session count does not bound disk at all, because a session is not a unit of anything — one is
// 4 KB of a role answering a question, the next is 40 MB of an agent rewriting a large file in a
// loop. Age is also the only one an operator can reason about ("what ran last month"); nobody knows
// what "200 sessions ago" was.
//
// It also gives the safety property for free: a session that is being written to right now has its
// last activity AT the reference instant, so a strict `<` cutoff can never delete a live session —
// at any window, including zero. A count-based cap has no such guarantee (a burst of triggered
// sessions can push a live one out of the top N).
//
// ## The clock — and the one direction a stamp is allowed to push
//
// A session's AGE comes from `at`, the SIDECAR's stamp. `MAX(at)` over it is a lexical maximum,
// which is chronologically correct because the producer emits `new Date().toISOString()`
// (`agent-sidecar/src/transcript.mjs:93`) — fixed-width, Z-suffixed UTC. The value is PARSED before
// it is compared, so a stamp in some other shape does not silently mis-sort into the past: it fails
// to parse, and the session is treated as undatable, which means KEPT.
//
// **The reference instant — "now" — is THIS DEVICE's clock, never a stamp off the wire**, on the
// automatic path exactly as on the manual one (`session_map::resolve_now`, so `NXC_NOW` pins both).
// An earlier revision of this used the incoming batch's own newest `at`, reasoning that the sidecar
// is a local child process and therefore the same clock. That reasoning covered skew between two
// CORRECT clocks and nothing else, and the gap it left was not theoretical — measured on the shipped
// build: one flush carrying a single stamp of `2031-01-01` made the cutoff `2031-01-01 − 30 days`,
// and the very next statement deleted every other session in the table, atomically and silently.
// A stamp is UNTRUSTED INPUT: it arrives over a pipe, it is validated for RFC3339 shape and nothing
// else, and an NTP jump, a dead RTC at boot, or one miswritten line is enough to produce it.
//
// With the device clock as the reference, a stamp can only ever move its OWN session's `MAX(at)`,
// and only in the keep direction: a future-dated entry makes that one transcript look younger and
// survive longer, which is the harmless way to be wrong. That is what the claim below actually rests
// on now.
//
// Retention fails safe in the keep direction everywhere — it is deleting evidence, and the cost of
// keeping too much is disk, while the cost of deleting too much is unrecoverable.

/// The default retention window: a session's transcript is kept for 30 days after its last recorded
/// activity. Long enough that "what did that role do last month" is still answerable, short enough
/// that the table is bounded by a month of work rather than by the workspace's age.
pub const TRANSCRIPT_KEEP_DAYS: i64 = 30;

/// The environment variable that overrides [`TRANSCRIPT_KEEP_DAYS`]: a non-negative number of days,
/// or the literal `off` to disable automatic retention entirely.
pub const KEEP_DAYS_ENV: &str = "NXC_TRANSCRIPT_KEEP_DAYS";

/// What one prune did, or (under `dry_run`) would do. Returned by
/// [`prune_transcripts`](ChatStore::prune_transcripts) and printed verbatim by
/// `nxc transcript prune --json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptPruneReport {
    /// The sessions whose transcripts were deleted, in session-id order (the GROUP BY that finds
    /// them has no inherent order, so this is sorted rather than left to the query planner).
    pub sessions: Vec<String>,
    /// How many rows that removed in total, across all of [`sessions`](Self::sessions).
    pub entries: u64,
    /// Sessions left alone because nothing in them carries an `at` the store can parse, so their
    /// age is unknown. Reported rather than swallowed: kept-because-undatable is a real gap in the
    /// policy's coverage, and an operator watching the table refuse to shrink needs to see it.
    pub undated: Vec<String>,
    /// Nothing was written — `sessions`/`entries` describe what a real run would have removed.
    pub dry_run: bool,
}

/// The configured retention window: `Some(days)`, or `None` when [`KEEP_DAYS_ENV`] says `off`.
/// Unset or empty ⇒ [`TRANSCRIPT_KEEP_DAYS`].
pub fn configured_keep_days() -> Result<Option<i64>> {
    parse_keep_days(std::env::var(KEEP_DAYS_ENV).ok().as_deref())
}

/// [`configured_keep_days`] over an explicit value — the whole parse, kept out of the environment so
/// it can be tested without mutating process-global state (`session_map.rs`'s `NXC_NOW` test already
/// carries that hazard for this crate; a second env-mutating test would make them race).
///
/// A value that is neither a non-negative integer nor `off` is a loud `validation` error naming the
/// variable, never a silent fallback to the default — a typo'd window is a misconfiguration an
/// operator wants told, and this is a setting that DELETES things.
fn parse_keep_days(raw: Option<&str>) -> Result<Option<i64>> {
    let raw = match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s,
        None => return Ok(Some(TRANSCRIPT_KEEP_DAYS)),
    };
    if raw.eq_ignore_ascii_case("off") {
        return Ok(None);
    }
    match raw.parse::<i64>() {
        Ok(n) if n >= 0 => Ok(Some(n)),
        _ => Err(NxfError::validation(format!(
            "{KEEP_DAYS_ENV} `{raw}` is neither a non-negative number of days nor `off`"
        ))),
    }
}

/// `now` minus `keep_days`, as the instant a session's last activity must be at or after to survive.
/// A negative window, an unparseable `now`, and an arithmetic overflow are all `validation` errors —
/// `time::Duration::days` is UNCHECKED and would otherwise panic, which release mode does not strip
/// (the same guard `facade::parse_duration` carries).
fn retention_cutoff(now: &str, keep_days: i64) -> Result<OffsetDateTime> {
    if keep_days < 0 {
        return Err(NxfError::validation(format!(
            "retention window {keep_days} is negative — days to keep, not a date"
        )));
    }
    let base = OffsetDateTime::parse(now, &Rfc3339)
        .map_err(|e| NxfError::validation(format!("cannot parse now `{now}` as RFC3339: {e}")))?;
    let secs = keep_days.checked_mul(86_400).ok_or_else(|| {
        NxfError::validation(format!("retention window {keep_days} days is too large"))
    })?;
    base.checked_sub(time::Duration::seconds(secs))
        .ok_or_else(|| {
            NxfError::validation(format!(
                "retention window {keep_days} days reaches past the representable date range"
            ))
        })
}

/// The prune itself, over an already-open connection so both entry points share ONE implementation:
/// [`ChatStore::prune_transcripts`] wraps it in its own transaction, and `append_transcript` calls
/// it inside the transaction it already holds.
///
/// Deletes every session whose last recorded activity is STRICTLY before `before`. The strictness is
/// load-bearing, not a rounding choice: it is what makes the automatic caller unable to delete the
/// transcript it was called for (see [`ChatStore::append_transcript`]).
///
/// ## Cost model, stated rather than left to be discovered
///
/// The deciding read is a scan of the WHOLE table's session summary — it is not amortized to the
/// marginal work of whatever new session triggered it (PR review, Code Quality #2). Two things keep
/// that honest rather than quadratic-in-disguise, and both are worth knowing before this is called
/// from anywhere new:
///
/// - it is a COVERING scan of `agent_transcript_session_at`, two narrow columns, so it never pages
///   through a single `tool_use` payload — the thing that actually makes this table big is exactly
///   what the scan does not touch (gated by `schema.rs`'s `retention_reads_never_touch_the_payloads`);
/// - retention itself bounds what there is to scan. Once it has run, the table holds one window of
///   work, so the scan is O(a month of sessions), not O(the workspace's lifetime).
///
/// The case that is genuinely slow is the FIRST prune of a workspace that predates retention — a
/// years-deep table, scanned once, under the write lock. That is a one-time cost, and it is why
/// `nxc transcript prune --dry-run` exists to tell an operator what is coming before they take it.
/// If it ever needs to be cheaper, the lever is a high-water mark that lets the scan skip sessions
/// already known to be inside the window; nothing here forecloses that.
fn prune_before(
    conn: &rusqlite::Connection,
    before: OffsetDateTime,
    dry_run: bool,
) -> Result<TranscriptPruneReport> {
    let mut sessions: Vec<String> = Vec::new();
    let mut undated: Vec<String> = Vec::new();
    let mut entries: u64 = 0;
    {
        // One pass over the table's session summary — `MAX(at)` is the session's last activity and
        // `COUNT(*)` its size, so a dry run can report the same numbers a real one removes without a
        // second query. Scoped so the statement is dropped before the DELETE below prepares its own.
        let mut st = conn.prepare(
            "SELECT internal_session, MAX(at), COUNT(*) FROM agent_transcript
             GROUP BY internal_session",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let session: String = r.get(0)?;
            let last: Option<String> = r.get(1)?;
            let count: i64 = r.get(2)?;
            // Parsed, not string-compared: a stamp the store cannot read is UNKNOWN age, which
            // means kept. See the module note above on failing safe in the keep direction.
            match last
                .as_deref()
                .and_then(|s| OffsetDateTime::parse(s, &Rfc3339).ok())
            {
                Some(t) if t < before => {
                    entries += count.max(0) as u64;
                    sessions.push(session);
                }
                Some(_) => {}
                None => undated.push(session),
            }
        }
    }
    sessions.sort();
    undated.sort();
    if !dry_run {
        let mut del = conn.prepare("DELETE FROM agent_transcript WHERE internal_session = ?1")?;
        for s in &sessions {
            del.execute([s])?;
        }
    }
    Ok(TranscriptPruneReport {
        sessions,
        entries,
        undated,
        dry_run,
    })
}

/// The cutoff the automatic prune runs at, or `None` when there is nothing defensible to run:
/// retention is off, the clock could not be read, or the window is unusable. `now` is THIS DEVICE's
/// clock, never a stamp off the wire — the module note above is where that distinction is argued,
/// and it is the whole safety property, not a detail.
///
/// **`Option`, not `Result`, on purpose** — the type is the rule: nothing this returns can fail the
/// flush it rides. `retention_cutoff` rejects a window so large its arithmetic overflows, which is a
/// real misconfiguration and one an operator hears about from `nxc transcript prune`; here it joins
/// "no clock" and "retention off" as one more reason to simply not prune. The alternative is a
/// housekeeping setting able to abort a role turn, which is the thing
/// [`append_transcript`](ChatStore::append_transcript) explicitly refuses.
fn automatic_cutoff(now: Option<&str>, keep_days: Option<i64>) -> Option<OffsetDateTime> {
    retention_cutoff(now?, keep_days?).ok()
}

/// A stored entry: its assigned `seq` plus the entry itself. `#[serde(flatten)]` keeps the wire
/// shape flat (`{"seq":0,"kind":…}`) while the Rust type stays literally "a [`TranscriptEntry`] plus
/// a seq" — so a round-trip test can compare `row.entry` against the entry that was appended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptRow {
    pub seq: i64,
    #[serde(flatten)]
    pub entry: TranscriptEntry,
}

impl ChatStore {
    /// Append a batch of transcript entries to `internal_session`, assigning `seq` monotonically
    /// from `COALESCE(MAX(seq), -1) + 1` and preserving within-batch order. Returns the number of
    /// rows written; an empty batch is a no-op returning 0, not an error (the sidecar flushes on a
    /// timer and legitimately has nothing to say sometimes).
    ///
    /// ONE `BEGIN IMMEDIATE` transaction, and validation runs BEFORE it opens.
    ///
    /// **IMMEDIATE, not the `unchecked_transaction()` default of DEFERRED** — same hazard and same
    /// fix as `nxs_foundation::schema::migrate` (`crates/foundation/src/schema.rs:80-88`). A
    /// deferred transaction takes its read snapshot at the `MAX(seq)` SELECT and only asks for the
    /// write lock at the first INSERT. That read→write UPGRADE fails INSTANTLY against any
    /// concurrent write-lock holder — whatever that holder goes on to do: SQLite deliberately does
    /// not invoke the busy handler for an upgrade (it cannot wait without risking deadlock, since
    /// the reader itself blocks the other writer's commit), so the `busy_timeout=5000` set in
    /// `workspace.rs` does nothing and the call returns "database is locked" in milliseconds. A
    /// holder that COMMITS additionally invalidates the snapshot, which is the narrower
    /// `SQLITE_BUSY_SNAPSHOT` layered on top — not the only way in. And a concurrent writer is the
    /// normal case here, not an exotic one: this is the SAME `db.sqlite` the ops log lives in, so
    /// the role's own in-session `nxc send` is exactly that. Not survivable either way: the sidecar
    /// propagates everything except clap's "unrecognized subcommand", so it would abort the whole
    /// role turn over a telemetry write. Taking the write lock UP FRONT makes a concurrent writer a
    /// plain `SQLITE_BUSY` that the busy handler DOES retry (measured on the regression fixture in
    /// `tests/transcript.rs`: DEFERRED errors after 1.5ms, IMMEDIATE waits 286ms and succeeds).
    ///
    /// Atomicity: a batch with a bad entry, or one that loses a write race, writes nothing at all
    /// rather than a torn prefix. Note that this is NOT about making a retry safe — the sidecar does
    /// not retry: `main.mjs` clears its buffer BEFORE the write, deliberately, so a failure cannot
    /// double-write. It is about what the store is left holding. A torn batch would persist an
    /// arbitrary prefix — cut at whichever entry failed — that reads back indistinguishable from a
    /// complete flush, while the remainder is gone with no marker. Either the whole flush is
    /// evidence or none of it is.
    ///
    /// Assigning `seq` inside the same transaction as the inserts is what makes two sidecar
    /// processes for one session (the `resume` case) safe: neither can read a high-water mark the
    /// other is about to invalidate.
    ///
    /// ## `data` is stored WHOLE, with no cap of its own
    ///
    /// The unbounded field on the wire is `tool_use`'s `data.input` — a `Write`/`Edit` of a large
    /// file puts the entire body in one row, dwarfing the sidecar's 4000-char thinking and 8000-char
    /// tool_result caps (beads behaves the same way). Two smaller ones are uncapped too: a fallback
    /// `assistant` `data.text` (`transcript.mjs:191`, emitted only when partials never arrived) and
    /// `result` `data.resultText` (`:239`) — both bounded in practice by model output length.
    /// Deliberately accepted rather than bounded here, for three reasons:
    ///
    /// 1. `data` is opaque to the store by contract — capping `data.input` means the store learning
    ///    the `tool_use` payload shape, and cap policy would then live in two languages across two
    ///    processes instead of in the normalizer, where both existing caps sit and are unit-tested
    ///    together.
    /// 2. The cost is local disk in `.nxs/db.sqlite` only — these rows are never folded into the op
    ///    log and never sync, so no peer and no shared log pays for them. Sizing, honestly: JSON
    ///    escaping inflates a large input, and an agent loop that rewrites the same big file writes
    ///    it once per attempt, so the figure is single-digit MB per session and **unbounded across
    ///    the workspace's lifetime** — was, until 6j6v.t7pa: retention, the cheaper lever, now
    ///    bounds it by a window of WORK instead (see "Retention rides this path" below and
    ///    [`prune_transcripts`](ChatStore::prune_transcripts)).
    /// 3. A tool INPUT is the agent's own product and the exact evidence a transcript exists to
    ///    preserve, unlike a tool RESULT, which is re-derivable by re-running the read — truncating
    ///    it would destroy the most valuable rows first.
    ///
    /// If a bound is ever wanted it belongs in `transcript.mjs`, next to MAX_THINKING_CHARS/
    /// MAX_TOOL_RESULT_CHARS: it sees the value pre-serialization and also saves the pipe cost, so
    /// the store would only ever be a redundant second gate. What the store DOES guarantee is that
    /// it never truncates anything silently — every byte handed to it is stored, or the whole batch
    /// is a loud error.
    ///
    /// ## The per-member channel timeout rides this path too (nxf 6j6v.nf38)
    ///
    /// A channel member's deadline hangs on its own thread, behind which stands exactly one session
    /// (nxf 6j6v.pf6j), and the owner's rule is that every transcript output restarts it. This
    /// function is the ONE write path every transcript append goes through, so the reset lives here
    /// and nowhere else — see the comment at the call, after the commit, for why it is best-effort.
    /// [`ChatStore::reset_member_deadlines_of_session`] is the whole of it; a session that is not a
    /// channel member moves nothing.
    ///
    /// ## Retention rides this path (nxf 6j6v.t7pa)
    ///
    /// When this call is the FIRST flush of a session — the one that assigns `seq` 0 — it also
    /// retires every session whose last activity is older than the configured window
    /// ([`configured_keep_days`]). That is the "when does it run" decision, and it was made here
    /// rather than at the two places the ticket offered:
    ///
    /// - **not on open.** Every `nxc` invocation opens the store, including the pure reads
    ///   (`inbox`, `threads list`, `transcript show`). A DELETE there would take a write lock on
    ///   paths that take none today — precisely the read→write hazard the paragraphs above spend
    ///   their length on — turning every reader into a writer for a disk chore.
    /// - **not on a schedule.** The epic's own architecture decision is to avoid a permanent
    ///   background service, so there is no scheduler to hang this on and inventing one for
    ///   housekeeping would be the tail wagging the dog.
    /// - **here, because this is where the growth is.** The table grows only when a transcript is
    ///   appended to, and it grows a new *slot* only when a new session starts. So the prune runs
    ///   exactly once per session — the same cadence as the problem, not once per flush (the sidecar
    ///   flushes every 32 entries) and not once per command. It is already inside this transaction's
    ///   write lock, so it costs no extra lock and inherits the same atomicity.
    ///
    /// The reference instant is THIS DEVICE's clock ([`crate::session_map::resolve_now`], so `NXC_NOW` pins
    /// it), the very same one `nxc transcript prune` anchors on — one policy, not two, so a
    /// `prune --dry-run` predicts exactly what the automatic pass will take. It is emphatically NOT
    /// a stamp off the wire; the module note above records what that cost when it was, and why an
    /// `at` may only ever move its own session's age in the KEEP direction.
    ///
    /// **Nothing here can fail the flush**, and that is a stronger claim than it first looks, so it
    /// is worth being exact about how it is held:
    ///
    /// - a malformed [`KEEP_DAYS_ENV`], an unreadable clock, or a window whose arithmetic overflows
    ///   all yield no cutoff ([`automatic_cutoff`] returns `Option`, not `Result`);
    /// - and the prune's own SQL is run BEST-EFFORT. This is not belt-and-braces: the read that
    ///   decides what is stale touches EVERY session in the table, so a single corrupt row in some
    ///   long-dead session — a foreign writer, a truncated file, a BLOB where TEXT was expected —
    ///   would otherwise abort the turn of a completely healthy session that merely happened to
    ///   start next. The blast radius of a chore must not exceed the chore.
    ///
    /// The reason all of that matters: the sidecar propagates everything except clap's "unrecognized
    /// subcommand", so an error on this path aborts the whole role turn. Housekeeping must never cost
    /// a turn's evidence. `nxc transcript prune` runs the identical policy and DOES surface every one
    /// of these failures — that is the path with an operator standing on it, and it is where a
    /// retention that has quietly stopped working becomes visible.
    pub fn append_transcript(
        &mut self,
        internal_session: &str,
        entries: &[TranscriptEntry],
    ) -> Result<u64> {
        // Both `unwrap_or(None)`/`.ok()` ARE the "nothing here can fail the flush" rule above, in
        // the one place it is decided.
        let keep_days = configured_keep_days().unwrap_or(None);
        let now = crate::session_map::resolve_now().ok();
        self.append_transcript_with_retention(internal_session, entries, keep_days, now.as_deref())
    }

    /// [`append_transcript`](ChatStore::append_transcript) with the retention window AND the
    /// reference instant passed in instead of resolved from the environment and the clock:
    /// `Some(days)` + `Some(now)` to prune on a new session's first flush, `None` for either to
    /// append and nothing else. Both resolutions live in the public wrapper, so what the tests drive
    /// is the behaviour itself rather than a second implementation of it — and so a retention test
    /// never has to pin a process-global `NXC_NOW`.
    pub(crate) fn append_transcript_with_retention(
        &mut self,
        internal_session: &str,
        entries: &[TranscriptEntry],
        keep_days: Option<i64>,
        now: Option<&str>,
    ) -> Result<u64> {
        if entries.is_empty() {
            return Ok(0);
        }
        for (i, e) in entries.iter().enumerate() {
            e.validate().map_err(|err| {
                NxfError::validation(format!("transcript entry {i}: {}", err.msg))
            })?;
        }
        // BEGIN IMMEDIATE — see the doc comment; `unchecked_transaction()` would be DEFERRED.
        let tx = rusqlite::Transaction::new_unchecked(
            self.connection(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM agent_transcript WHERE internal_session = ?1",
            [internal_session],
            |r| r.get(0),
        )?;
        if seq == 0 {
            // A brand-new transcript (see "Retention rides this path"). Before the inserts, so the
            // session being opened is not even in the table yet — it cannot be its own candidate on
            // any code path, not merely on the arithmetic.
            if let Some(before) = automatic_cutoff(now, keep_days) {
                // Best-effort, deliberately — the `?` this does NOT have is the point. See the doc
                // comment: this read spans every session in the table, so one corrupt row anywhere
                // would otherwise take down the turn of a healthy session that merely started next.
                // A statement error leaves the transaction usable (SQLite does not roll back on one),
                // and anything that DOES force a rollback would fail the INSERT below on its own —
                // with the error that actually describes the flush.
                let _ = prune_before(&tx, before, false);
            }
        }
        {
            let mut st = tx.prepare(
                "INSERT INTO agent_transcript(
                     internal_session, seq, kind, tool_use_id, parent_tool_use_id,
                     subagent_type, at, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for e in entries {
                // `data` goes in whole — the no-cap decision is argued in the doc comment above.
                let data = serde_json::to_string(&e.data)
                    .map_err(|err| NxfError::io(format!("serializing transcript data: {err}")))?;
                st.execute(params![
                    internal_session,
                    seq,
                    e.kind,
                    e.tool_use_id,
                    e.parent_tool_use_id,
                    e.subagent_type,
                    e.at,
                    data,
                ])?;
                seq += 1;
            }
        }
        tx.commit()?;
        // **THE PER-MEMBER TIMEOUT RESET (nxf 6j6v.nf38).** A channel member's deadline hangs on its
        // own thread, behind which stands exactly one session (nxf 6j6v.pf6j) — this one — and the
        // owner's rule is that every transcript output restarts it. This is the ONE write path every
        // transcript append goes through, so it is the one place that has to know: resetting from
        // the N callers that produce entries would be N chances to forget, and the sidecar is not
        // the only producer.
        //
        // AFTER the commit, not inside it, and BEST-EFFORT — both for the reason this function's own
        // doc gives at length: nothing here may fail the flush. An error resolving the clock costs
        // this member its reset (it may time out early, and the next flush restarts it anyway),
        // while propagating would abort the whole role turn and lose its evidence. The blast radius
        // of bookkeeping must not exceed the bookkeeping. `now` is the same reference instant
        // retention uses; `None` (an unreadable clock) means there is nothing to reset TO, so the
        // append stands alone.
        //
        // **Best-effort is not silent.** A failure here reverts this member to the old stubbornly
        // running deadline, and a PERSISTENT one reverts the whole feature — so it breadcrumbs on
        // stderr, exactly as the neighbouring best-effort site in `orchestration.rs` (the timeout
        // tick's scheduling) does. Nobody watches a sidecar's stderr closely, but a feature that has
        // quietly stopped working must at least have said so somewhere.
        if let Some(now) = now {
            if let Err(e) = self.reset_member_deadlines_of_session(internal_session, now) {
                eprintln!(
                    "warning: could not restart the channel timeout for session \
                     {internal_session}: {e}"
                );
            }
        }
        Ok(entries.len() as u64)
    }

    /// Retire every session whose last recorded activity is older than `keep_days` before `now`,
    /// and report what went (nxf 6j6v.t7pa). The operator's half of retention — the automatic half
    /// rides [`append_transcript`](ChatStore::append_transcript) and is described there, along with
    /// why whole sessions and why age; the policy itself is identical, only the reference instant
    /// differs (here it is the CALLER's clock, because an operator clearing out a workspace that has
    /// been idle for months means "older than today", not "older than the last thing anyone
    /// recorded").
    ///
    /// `dry_run` reports the same `sessions`/`entries` a real run would remove and writes nothing —
    /// it takes no write lock either, so it is safe to point at a busy workspace.
    ///
    /// A session it deletes and then sees again (a `resume` after the window elapsed) simply starts
    /// a fresh transcript at `seq` 0; nothing collides, because nothing is left to collide with.
    ///
    /// ## What this does and does not do to the file on disk
    ///
    /// It bounds GROWTH, and stops there. SQLite puts the pages a `DELETE` frees on the freelist and
    /// reuses them, so `.nxs/db.sqlite` stops growing — but it does not shrink, and an operator who
    /// runs this expecting the file to get smaller today will be disappointed by `ls`. Shrinking
    /// takes a `VACUUM`, which is deliberately NOT run here: `agent_transcript` shares `db.sqlite`
    /// with the ops log, and a vacuum rewrites that whole file under an exclusive lock — a heavy,
    /// blocking, whole-workspace operation to hand someone as the tail of a housekeeping command,
    /// especially in a workspace with live role sessions. Run it by hand if the file size itself is
    /// the problem; the table's unbounded growth, which is what the ticket is about, is fixed either
    /// way.
    pub fn prune_transcripts(
        &mut self,
        now: &str,
        keep_days: i64,
        dry_run: bool,
    ) -> Result<TranscriptPruneReport> {
        let before = retention_cutoff(now, keep_days)?;
        if dry_run {
            return prune_before(self.connection(), before, true);
        }
        // IMMEDIATE for the same reason `append_transcript` uses it: the read that decides what to
        // delete and the deletes themselves must not be a DEFERRED read→write upgrade, which SQLite
        // fails instantly (no busy handler) against any concurrent writer — and a role's own sidecar
        // flush is exactly that.
        let tx = rusqlite::Transaction::new_unchecked(
            self.connection(),
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let report = prune_before(&tx, before, false)?;
        tx.commit()?;
        Ok(report)
    }

    /// Every stored entry for `internal_session` in `seq` order — the flat read (T3 builds the
    /// nested subagent view on top of it). An unknown session is an empty vec, not `not_found`: a
    /// session whose sidecar never flushed is indistinguishable from one that had nothing to say.
    pub fn transcript_rows(&self, internal_session: &str) -> Result<Vec<TranscriptRow>> {
        // `-1` because `seq` starts at 0 (`COALESCE(MAX(seq), -1) + 1`), so "after -1" is "all".
        self.transcript_rows_after(internal_session, -1)
    }

    /// [`transcript_rows`](ChatStore::transcript_rows) restricted to entries newer than `after_seq`
    /// — what a live follower needs (nxf 6j6v.p6m1's `--stream`).
    ///
    /// It exists because the follower re-reads on a short interval, and `transcript_rows` returns
    /// (and JSON-parses) the WHOLE session every time: on a long-running role that is thousands of
    /// rows with unbounded `tool_use` inputs, re-parsed once a second, to find the one entry that is
    /// new. The `seq` is monotonic per session, so the cursor is exact — nothing is skipped and
    /// nothing is delivered twice.
    pub fn transcript_rows_after(
        &self,
        internal_session: &str,
        after_seq: i64,
    ) -> Result<Vec<TranscriptRow>> {
        self.transcript_rows_page(internal_session, after_seq, None)
    }

    /// [`transcript_rows_after`](ChatStore::transcript_rows_after) with an optional cap on how many
    /// rows come back — the paging half of nxf 6j6v.t7pa, and the only one of these three reads that
    /// bounds what a single call holds in memory.
    ///
    /// The cap is applied in SQL (`LIMIT`), not after the fact: the whole point is to never
    /// materialize the `tool_use` payloads beyond the window, and a Rust-side truncate would have
    /// parsed every one of them first. A `limit` of zero or less is treated as no limit — a page of
    /// nothing is a caller mistake with no useful answer, and returning an empty vec would look
    /// exactly like "end of session" to the cursor walk above.
    pub fn transcript_rows_page(
        &self,
        internal_session: &str,
        after_seq: i64,
        limit: Option<i64>,
    ) -> Result<Vec<TranscriptRow>> {
        let conn = self.connection();
        // `LIMIT -1` is SQLite's own spelling of "no limit", so one statement serves both shapes and
        // the query plan stays the single index-backed range scan the schema gate pins.
        let limit = limit.filter(|n| *n > 0).unwrap_or(-1);
        let mut st = conn.prepare(
            "SELECT seq, kind, tool_use_id, parent_tool_use_id, subagent_type, at, data
             FROM agent_transcript WHERE internal_session = ?1 AND seq > ?2 ORDER BY seq
             LIMIT ?3",
        )?;
        // Streamed row by row, NOT `query_map(..).collect()` then parsed in a second pass. The
        // reason for a hand-rolled loop at all is unchanged — `query_map`'s closure can only yield
        // `rusqlite::Error`, and a `data` column that is not valid JSON deserves its own message
        // naming the row — but collecting the raw column tuples FIRST held every raw JSON string
        // and every parsed `Value` alive at overlapping times. T3's `facade::transcript` calls this
        // on a WHOLE session, unbounded `tool_use` inputs included (the no-cap decision above), so
        // that doubled peak memory for no benefit. Here each raw string is dropped as soon as its
        // `Value` exists.
        let mut rows = st.query(rusqlite::params![internal_session, after_seq, limit])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let seq: i64 = r.get(0)?;
            let raw: String = r.get(6)?;
            let data = serde_json::from_str(&raw).map_err(|e| {
                NxfError::io(format!(
                    "transcript {internal_session} seq {seq}: stored data is not JSON: {e}"
                ))
            })?;
            out.push(TranscriptRow {
                seq,
                entry: TranscriptEntry {
                    kind: r.get(1)?,
                    at: r.get(5)?,
                    tool_use_id: r.get(2)?,
                    parent_tool_use_id: r.get(3)?,
                    subagent_type: r.get(4)?,
                    data,
                },
            });
        }
        Ok(out)
    }

    /// The transcript high-water mark for `internal_session`: `MAX(seq) + 1`, i.e. how many entries
    /// the session has produced, and `0` for a session that has produced none (including one that
    /// does not exist). A monotonically non-decreasing count is all a comparison needs — the value
    /// itself is never shown to anyone.
    ///
    /// It lived in `liveness.rs` until 6j6v.dvyq §3 removed the run engine, and moved here rather
    /// than going with it: what it counts is transcript entries, and the one reader left is the
    /// per-member channel deadline this same module resets on every flush.
    pub fn transcript_progress(&self, internal_session: &str) -> Result<i64> {
        Ok(self.connection().query_row(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM agent_transcript WHERE internal_session = ?1",
            [internal_session],
            |r| r.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;
    use serde_json::json;

    /// A minimal entry of `kind` with an opaque payload — the shape most tests only need to be
    /// distinguishable by.
    fn entry(kind: &str, data: serde_json::Value) -> TranscriptEntry {
        TranscriptEntry {
            kind: kind.to_string(),
            at: Some("2026-07-26T07:42:39.123Z".to_string()),
            tool_use_id: None,
            parent_tool_use_id: None,
            subagent_type: None,
            data,
        }
    }

    /// Append WITHOUT the retention window — the entry point every test below that is about `seq`,
    /// ordering or a round-trip uses.
    ///
    /// The public [`ChatStore::append_transcript`] resolves the window from the environment and the
    /// REAL clock, and prunes on a new session's first flush across the whole table. Combined with
    /// the fixed `at` these fixtures carry, that gave the tests an expiry date: on 2026-08-25 the
    /// fixture instant fell out of the default 30-day window, and opening a second session started
    /// deleting the first one's rows — `seq_is_per_session_not_global` went red with nothing
    /// changed (6j6v.v6mb). Retention has its own tests, which pass `now`/`keep_days` explicitly
    /// for exactly this reason; nothing else here has any business reading the wall clock.
    fn append(s: &mut ChatStore, session: &str, entries: &[TranscriptEntry]) -> Result<u64> {
        s.append_transcript_with_retention(session, entries, None, None)
    }

    #[test]
    fn a_batch_round_trips_in_order_with_seq_assigned_from_zero() {
        let mut s = ChatStore::open_in_memory(1);
        let batch = vec![
            entry(
                "session_init",
                json!({ "sdkSessionId": "real-abc", "tools": 12 }),
            ),
            entry(
                "thinking",
                json!({ "text": "let me check the tests first" }),
            ),
            entry("assistant", json!({ "text": "Looking at it now." })),
        ];
        assert_eq!(append(&mut s, "s-1", &batch).unwrap(), 3);
        let rows = s.transcript_rows("s-1").unwrap();
        assert_eq!(rows.iter().map(|r| r.seq).collect::<Vec<_>>(), [0, 1, 2]);
        // The entries come back byte-identical, INCLUDING the opaque `data` payloads.
        assert_eq!(rows.into_iter().map(|r| r.entry).collect::<Vec<_>>(), batch);
    }

    #[test]
    fn a_second_append_continues_seq_instead_of_restarting() {
        // The `resume` case: a second sidecar process for the SAME internal session appends more
        // entries. `seq` must continue (3, 4), never restart at 0 — a restart would collide on the
        // primary key and, worse, would scramble the transcript's order if it did not.
        let mut s = ChatStore::open_in_memory(1);
        append(
            &mut s,
            "s-1",
            &[
                entry("session_init", json!({ "sdkSessionId": "real-abc" })),
                entry("assistant", json!({ "text": "first turn" })),
                entry("result", json!({ "subtype": "success", "numTurns": 1 })),
            ],
        )
        .unwrap();
        append(
            &mut s,
            "s-1",
            &[
                entry("assistant", json!({ "text": "second turn" })),
                entry("result", json!({ "subtype": "success", "numTurns": 2 })),
            ],
        )
        .unwrap();
        let rows = s.transcript_rows("s-1").unwrap();
        assert_eq!(
            rows.iter().map(|r| r.seq).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert_eq!(
            rows.iter()
                .map(|r| r.entry.kind.as_str())
                .collect::<Vec<_>>(),
            ["session_init", "assistant", "result", "assistant", "result"]
        );
        assert_eq!(rows[3].entry.data["text"], "second turn");
    }

    #[test]
    fn seq_is_per_session_not_global() {
        // Two concurrent role sessions each own their own numbering, and neither read sees the
        // other's rows.
        let mut s = ChatStore::open_in_memory(1);
        append(&mut s, "s-1", &[entry("assistant", json!({ "text": "a" }))]).unwrap();
        append(&mut s, "s-2", &[entry("assistant", json!({ "text": "b" }))]).unwrap();
        append(&mut s, "s-1", &[entry("assistant", json!({ "text": "c" }))]).unwrap();
        let one = s.transcript_rows("s-1").unwrap();
        let two = s.transcript_rows("s-2").unwrap();
        assert_eq!(one.iter().map(|r| r.seq).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(two.iter().map(|r| r.seq).collect::<Vec<_>>(), [0]);
        assert_eq!(two[0].entry.data["text"], "b");
    }

    #[test]
    fn subagent_provenance_survives_the_round_trip_nested_under_its_spawning_task() {
        // The gap beads left open (its persist path dropped subagent entries, so sub-timelines
        // vanished on reload): a Task-spawned subagent's own tool_use/tool_result/assistant entries
        // are stored as first-class rows carrying the spawning Task's id, interleaved in the same
        // stream as the main conversation. T3 reconstructs the nesting from exactly these columns,
        // so all three must survive verbatim.
        let mut s = ChatStore::open_in_memory(1);
        let sub =
            |kind: &str, tool_use_id: Option<&str>, data: serde_json::Value| TranscriptEntry {
                kind: kind.to_string(),
                at: Some("2026-07-26T07:42:40.000Z".to_string()),
                tool_use_id: tool_use_id.map(str::to_string),
                parent_tool_use_id: Some("toolu_task_1".to_string()),
                subagent_type: Some("code-reviewer".to_string()),
                data,
            };
        let batch = vec![
            TranscriptEntry {
                tool_use_id: Some("toolu_task_1".to_string()),
                ..entry(
                    "tool_use",
                    json!({ "name": "Task", "input": { "prompt": "review" } }),
                )
            },
            sub("assistant", None, json!({ "text": "reading the diff" })),
            sub(
                "tool_use",
                Some("toolu_read_9"),
                json!({ "name": "Read", "input": { "file_path": "/x.rs" } }),
            ),
            sub(
                "tool_result",
                Some("toolu_read_9"),
                json!({ "content": "fn main() {}", "isError": false }),
            ),
            entry("assistant", json!({ "text": "the reviewer found nothing" })),
        ];
        append(&mut s, "s-1", &batch).unwrap();
        let rows = s.transcript_rows("s-1").unwrap();
        assert_eq!(
            rows.iter().map(|r| r.entry.clone()).collect::<Vec<_>>(),
            batch
        );
        // The spawning Task's own id is what the four subagent rows point back at; the main
        // conversation's trailing entry carries no parent at all.
        assert_eq!(rows[0].entry.tool_use_id.as_deref(), Some("toolu_task_1"));
        assert_eq!(rows[0].entry.parent_tool_use_id, None);
        for r in &rows[1..4] {
            assert_eq!(r.entry.parent_tool_use_id.as_deref(), Some("toolu_task_1"));
            assert_eq!(r.entry.subagent_type.as_deref(), Some("code-reviewer"));
        }
        assert_eq!(rows[4].entry.parent_tool_use_id, None);
        assert_eq!(rows[4].entry.subagent_type, None);
    }

    #[test]
    fn pre_capped_thinking_and_tool_result_payloads_round_trip_unchanged() {
        // Both caps are applied PRODUCER-side (transcript.mjs's MAX_THINKING_CHARS = 4000 /
        // MAX_TOOL_RESULT_CHARS = 8000), so the store must be a faithful recorder of what already
        // arrived capped — it never re-caps and never re-truncates. The exact truncation marker T1
        // appends is part of the value; it must survive byte-for-byte, or a consumer can no longer
        // recognise an overflowed block.
        let mut s = ChatStore::open_in_memory(1);
        let thinking = format!("{}\n…[truncated 137 chars]", "t".repeat(4000));
        let tool_result = format!("{}\n…[truncated 24601 chars]", "r".repeat(8000));
        append(
            &mut s,
            "s-1",
            &[
                entry("thinking", json!({ "text": thinking })),
                entry(
                    "tool_result",
                    json!({ "content": tool_result, "isError": false }),
                ),
            ],
        )
        .unwrap();
        let rows = s.transcript_rows("s-1").unwrap();
        assert_eq!(rows[0].entry.data["text"], json!(thinking));
        assert_eq!(rows[1].entry.data["content"], json!(tool_result));
    }

    #[test]
    fn an_unbounded_tool_use_input_is_stored_whole_never_silently_truncated() {
        // The decision recorded at the INSERT in `append_transcript`: `data.input` is the one
        // unbounded field on the wire (a `Write` of a large file), and the store accepts it whole
        // rather than reaching into an opaque payload. This test is that decision's guard — if a cap
        // is ever added here it must be added loudly, and this test is what will red.
        let mut s = ChatStore::open_in_memory(1);
        let big = "x".repeat(200_000);
        append(
            &mut s,
            "s-1",
            &[entry(
                "tool_use",
                json!({ "name": "Write", "input": { "content": big } }),
            )],
        )
        .unwrap();
        let rows = s.transcript_rows("s-1").unwrap();
        assert_eq!(rows[0].entry.data["input"]["content"], json!(big));
    }

    #[test]
    fn an_empty_batch_is_a_no_op_returning_zero() {
        let mut s = ChatStore::open_in_memory(1);
        assert_eq!(s.append_transcript("s-1", &[]).unwrap(), 0);
        assert!(s.transcript_rows("s-1").unwrap().is_empty());
    }

    #[test]
    fn a_row_whose_stored_data_is_not_json_errors_naming_the_session_and_seq() {
        // The one reason `transcript_rows` hand-rolls its loop instead of using `query_map`
        // (whose closure can only yield a bare `rusqlite::Error`): a corrupt `data` column must be
        // reported with the coordinate needed to find it. Only reachable by writing around the
        // append path — which is exactly what a foreign writer or a truncated file looks like.
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute(
                "INSERT INTO agent_transcript(internal_session, seq, kind, data)
                 VALUES ('s-1', 7, 'assistant', 'not json at all')",
                [],
            )
            .unwrap();
        let err = s.transcript_rows("s-1").unwrap_err();
        assert_eq!(err.kind, ErrorKind::Io);
        assert!(err.msg.contains("s-1"), "names the session: {}", err.msg);
        assert!(err.msg.contains("seq 7"), "names the row: {}", err.msg);
    }

    #[test]
    fn transcript_rows_of_an_unwritten_session_is_empty_not_an_error() {
        let s = ChatStore::open_in_memory(1);
        assert!(s.transcript_rows("never-existed").unwrap().is_empty());
    }

    #[test]
    fn an_entry_with_an_empty_kind_is_a_validation_error_and_the_whole_batch_is_rolled_back() {
        // Loud, never a silent skip — and atomic: the two good entries flanking the bad one must not
        // land, or the store would keep a prefix of a flush that failed, indistinguishable on read
        // from one that succeeded (see `append_transcript`'s doc comment; the sidecar does not retry,
        // so nothing recovers the rest).
        let mut s = ChatStore::open_in_memory(1);
        let err = s
            .append_transcript(
                "s-1",
                &[
                    entry("assistant", json!({ "text": "before" })),
                    entry("   ", json!({})),
                    entry("assistant", json!({ "text": "after" })),
                ],
            )
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("entry 1"), "names the entry: {}", err.msg);
        assert!(
            s.transcript_rows("s-1").unwrap().is_empty(),
            "a rejected batch writes nothing"
        );
    }

    // ---- retention (nxf 6j6v.t7pa) ------------------------------------------------------------

    /// The reference instant the retention tests measure from — this device's clock, as
    /// `append_transcript` resolves it in production. Passed EXPLICITLY here rather than pinned
    /// through `NXC_NOW`: that variable is process-global, and `session_map.rs`'s own test already
    /// sets and removes it in this same binary.
    const NOW: &str = "2026-08-12T00:00:00Z";

    /// An entry stamped at `at` — retention keys off exactly this field, so the tests that exercise
    /// it need to place entries on the calendar rather than take [`entry`]'s one fixed instant.
    fn at(kind: &str, at: &str) -> TranscriptEntry {
        TranscriptEntry {
            at: Some(at.to_string()),
            ..entry(kind, json!({ "text": "x" }))
        }
    }

    #[test]
    fn a_session_whose_last_entry_predates_the_window_is_pruned_whole() {
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-old",
            &[
                at("assistant", "2026-06-01T00:00:00Z"),
                at("result", "2026-06-01T00:05:00Z"),
            ],
            None,
            None,
        )
        .unwrap();
        let report = s
            .prune_transcripts("2026-08-12T00:00:00Z", 30, false)
            .unwrap();
        assert_eq!(report.sessions, ["s-old"]);
        assert_eq!(report.entries, 2);
        assert!(s.transcript_rows("s-old").unwrap().is_empty());
    }

    #[test]
    fn a_session_inside_the_window_is_untouched() {
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-new",
            &[at("assistant", "2026-08-01T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        let report = s
            .prune_transcripts("2026-08-12T00:00:00Z", 30, false)
            .unwrap();
        assert!(report.sessions.is_empty(), "{report:?}");
        assert_eq!(s.transcript_rows("s-new").unwrap().len(), 1);
    }

    #[test]
    fn retention_keys_off_the_last_entry_so_a_long_session_is_never_cut_in_half() {
        // Whole sessions or nothing. Age is the session's LAST activity, not each row's own `at`:
        // a session opened in June and still writing in August is one live transcript, and deleting
        // its early rows would leave a torn one that reads back indistinguishable from a complete
        // one — the same reason `append_transcript` is atomic.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-long",
            &[
                at("session_init", "2026-06-01T00:00:00Z"),
                at("assistant", "2026-08-11T00:00:00Z"),
            ],
            None,
            None,
        )
        .unwrap();
        let report = s
            .prune_transcripts("2026-08-12T00:00:00Z", 30, false)
            .unwrap();
        assert!(report.sessions.is_empty(), "{report:?}");
        assert_eq!(s.transcript_rows("s-long").unwrap().len(), 2, "both kept");
    }

    #[test]
    fn a_session_nothing_can_date_is_kept_and_named_in_the_report() {
        // `at` is optional on the wire and the store never invents one. An undatable session is
        // therefore of UNKNOWN age, and retention keeps what it cannot date rather than guessing —
        // but it says so, so the kept rows are not a silent hole in the policy.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-undated",
            &[TranscriptEntry {
                at: None,
                ..entry("assistant", json!({ "text": "x" }))
            }],
            None,
            None,
        )
        .unwrap();
        let report = s
            .prune_transcripts("2026-08-12T00:00:00Z", 0, false)
            .unwrap();
        assert!(report.sessions.is_empty());
        assert_eq!(report.undated, ["s-undated"]);
        assert_eq!(s.transcript_rows("s-undated").unwrap().len(), 1);
    }

    #[test]
    fn an_unparseable_at_reads_as_undated_rather_than_as_ancient() {
        // A stamp the store cannot parse must fail SAFE (keep), not fall through to "older than any
        // cutoff" — the fail-open direction here would delete evidence over a producer's formatting
        // bug.
        let s = ChatStore::open_in_memory(1);
        s.connection()
            .execute(
                "INSERT INTO agent_transcript(internal_session, seq, kind, at, data)
                 VALUES ('s-junk', 0, 'assistant', 'yesterday-ish', '{}')",
                [],
            )
            .unwrap();
        let mut s = s;
        let report = s
            .prune_transcripts("2026-08-12T00:00:00Z", 0, false)
            .unwrap();
        assert!(report.sessions.is_empty());
        assert_eq!(report.undated, ["s-junk"]);
    }

    #[test]
    fn a_dry_run_reports_what_it_would_delete_and_deletes_nothing() {
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-old",
            &[
                at("assistant", "2026-06-01T00:00:00Z"),
                at("result", "2026-06-01T00:01:00Z"),
            ],
            None,
            None,
        )
        .unwrap();
        let report = s
            .prune_transcripts("2026-08-12T00:00:00Z", 30, true)
            .unwrap();
        assert!(report.dry_run);
        assert_eq!(report.sessions, ["s-old"]);
        assert_eq!(report.entries, 2);
        assert_eq!(
            s.transcript_rows("s-old").unwrap().len(),
            2,
            "a dry run writes nothing"
        );
    }

    #[test]
    fn a_zero_day_window_still_keeps_a_session_whose_last_entry_is_now() {
        // The cutoff is STRICT: `keep_days = 0` means "older than this instant", so the session
        // that is writing right now survives its own prune. That is what makes the automatic path
        // below unable to delete the transcript it was called for, at any window.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-now",
            &[at("assistant", "2026-08-12T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        let report = s
            .prune_transcripts("2026-08-12T00:00:00Z", 0, false)
            .unwrap();
        assert!(report.sessions.is_empty(), "{report:?}");
        assert_eq!(s.transcript_rows("s-now").unwrap().len(), 1);
    }

    #[test]
    fn a_negative_window_is_a_validation_error_not_a_prune_into_the_future() {
        let mut s = ChatStore::open_in_memory(1);
        let err = s
            .prune_transcripts("2026-08-12T00:00:00Z", -1, false)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
    }

    #[test]
    fn an_unparseable_now_is_a_validation_error_naming_it() {
        let mut s = ChatStore::open_in_memory(1);
        let err = s.prune_transcripts("whenever", 30, false).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
        assert!(err.msg.contains("whenever"), "{}", err.msg);
    }

    // ---- the automatic half: retention rides the first flush of a NEW session ------------------

    #[test]
    fn the_first_flush_of_a_new_session_retires_the_stale_ones() {
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-old",
            &[at("assistant", "2026-06-01T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        // A brand-new session's first batch: its own newest `at` is the reference instant.
        s.append_transcript_with_retention(
            "s-new",
            &[at("session_init", "2026-08-12T00:00:00Z")],
            Some(30),
            Some(NOW),
        )
        .unwrap();
        assert!(s.transcript_rows("s-old").unwrap().is_empty(), "retired");
        assert_eq!(s.transcript_rows("s-new").unwrap().len(), 1, "kept");
    }

    #[test]
    fn a_later_flush_of_a_running_session_does_not_prune() {
        // Once per SESSION, not once per flush: the sidecar flushes every 32 entries, and a scan of
        // the whole table on each of those would put a table-sized cost on the hot write path for a
        // disk chore. `seq == 0` is exactly "a new transcript just started".
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-old",
            &[at("assistant", "2026-06-01T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        s.append_transcript_with_retention(
            "s-new",
            &[at("session_init", "2026-08-12T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        // Second flush of s-new, WITH retention on — it must not fire, because s-new is not new.
        s.append_transcript_with_retention(
            "s-new",
            &[at("assistant", "2026-08-12T00:01:00Z")],
            Some(30),
            Some(NOW),
        )
        .unwrap();
        assert_eq!(
            s.transcript_rows("s-old").unwrap().len(),
            1,
            "no prune on a continuation flush"
        );
    }

    #[test]
    fn the_automatic_prune_cannot_delete_the_session_it_was_called_for() {
        // The reference instant is the incoming batch's own newest stamp, so the session being
        // written is by construction not older than it — at ANY window, including zero.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-new",
            &[at("session_init", "2026-08-12T00:00:00Z")],
            Some(0),
            Some(NOW),
        )
        .unwrap();
        assert_eq!(s.transcript_rows("s-new").unwrap().len(), 1);
    }

    #[test]
    fn a_future_dated_batch_cannot_delete_anyone_elses_transcript() {
        // THE regression test for the worst thing this feature could do (PR review, Integrity #1 —
        // reproduced end to end on the shipped build before it was fixed). The reference instant
        // used to be the incoming batch's own newest `at`, so ONE flush carrying `2031-01-01` made
        // the cutoff `2031-01-01 − 30 days` and the next statement deleted every other session in
        // the table — every real transcript in the workspace, atomically and silently, from a single
        // untrusted string off a pipe.
        //
        // Now `now` is the device's clock and a stamp can only move its OWN session's age, in the
        // KEEP direction. Both neighbours must survive, and the absurdly-future one must survive too
        // (it looks younger than everything, which is the harmless way to be wrong).
        let mut s = ChatStore::open_in_memory(1);
        for (session, stamp) in [
            ("s-a", "2026-08-10T09:00:00Z"),
            ("s-b", "2026-08-11T09:00:00Z"),
        ] {
            s.append_transcript_with_retention(session, &[at("assistant", stamp)], None, None)
                .unwrap();
        }
        s.append_transcript_with_retention(
            "s-skewed",
            &[at("session_init", "2031-01-01T00:00:00Z")],
            Some(30),
            Some(NOW),
        )
        .unwrap();
        assert_eq!(s.transcript_rows("s-a").unwrap().len(), 1, "untouched");
        assert_eq!(s.transcript_rows("s-b").unwrap().len(), 1, "untouched");
        assert_eq!(s.transcript_rows("s-skewed").unwrap().len(), 1);
    }

    #[test]
    fn a_batch_with_no_stamp_at_all_still_prunes_on_the_devices_clock() {
        // The flip side of the same change: what the batch says stopped mattering, so a batch with
        // no `at` whatever no longer disables retention the way it once did. Retention is a property
        // of the CLOCK and the window, not of the payload that happened to trigger it.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-old",
            &[at("assistant", "2026-06-01T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        s.append_transcript_with_retention(
            "s-new",
            &[TranscriptEntry {
                at: None,
                ..entry("session_init", json!({}))
            }],
            Some(30),
            Some(NOW),
        )
        .unwrap();
        assert!(s.transcript_rows("s-old").unwrap().is_empty(), "retired");
    }

    #[test]
    fn the_window_boundary_is_exact_at_a_nonzero_number_of_days() {
        // PR review, Test Quality #2: the "a live session survives" property was pinned only at
        // `keep_days = 0`, where the day->seconds arithmetic is trivially zero and an off-by-one in
        // `checked_mul(86_400)` would hide. Pin the real boundary: `NOW` is 2026-08-12T00:00:00Z, so
        // at 30 days the cutoff is exactly 2026-07-13T00:00:00Z — that instant survives (the cutoff
        // is strict), one second earlier does not.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-on-the-line",
            &[at("assistant", "2026-07-13T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        s.append_transcript_with_retention(
            "s-one-second-older",
            &[at("assistant", "2026-07-12T23:59:59Z")],
            None,
            None,
        )
        .unwrap();
        let report = s.prune_transcripts(NOW, 30, false).unwrap();
        assert_eq!(report.sessions, ["s-one-second-older"]);
        assert_eq!(
            s.transcript_rows("s-on-the-line").unwrap().len(),
            1,
            "exactly `keep_days` old is still inside the window"
        );
    }

    #[test]
    fn a_corrupt_row_in_some_other_session_cannot_abort_a_healthy_flush() {
        // PR review, Integrity #2 / Test Quality #1. The prune's read spans EVERY session in the
        // table, so a value the store cannot read — a foreign writer, a truncated file, a BLOB where
        // TEXT was expected — used to propagate straight out of a brand-new session's first flush and
        // abort the whole role turn. A chore must not have a blast radius bigger than itself.
        let mut s = ChatStore::open_in_memory(1);
        s.connection()
            .execute(
                "INSERT INTO agent_transcript(internal_session, seq, kind, at, data)
                 VALUES ('s-corrupt', 0, 'assistant', X'DEADBEEF', '{}')",
                [],
            )
            .unwrap();
        // A healthy new session flushes. It must land, in full.
        s.append_transcript_with_retention(
            "s-healthy",
            &[
                at("session_init", "2026-08-12T00:00:00Z"),
                at("assistant", "2026-08-12T00:00:01Z"),
            ],
            Some(30),
            Some(NOW),
        )
        .expect("a corrupt neighbour must not fail the flush");
        assert_eq!(s.transcript_rows("s-healthy").unwrap().len(), 2);
    }

    #[test]
    fn the_manual_prune_does_surface_what_the_automatic_one_swallows() {
        // The other half of the split, and the reason swallowing above is not simply hiding a fault:
        // the operator's path runs the identical policy and reports the failure, so a retention that
        // has quietly stopped working is discoverable rather than lost.
        let mut s = ChatStore::open_in_memory(1);
        s.connection()
            .execute(
                "INSERT INTO agent_transcript(internal_session, seq, kind, at, data)
                 VALUES ('s-corrupt', 0, 'assistant', X'DEADBEEF', '{}')",
                [],
            )
            .unwrap();
        assert!(
            s.prune_transcripts(NOW, 30, false).is_err(),
            "the path with an operator on it reports it"
        );
    }

    #[test]
    fn a_window_too_large_to_compute_skips_the_prune_instead_of_failing_the_flush() {
        // `NXC_TRANSCRIPT_KEEP_DAYS=999999999999` parses as a number of days and then overflows the
        // cutoff arithmetic. `nxc transcript prune` reports that; HERE it must join "retention off"
        // and "no stamp" as one more reason not to prune, because this path is the sidecar's mid-run
        // flush and an error on it aborts the whole role turn.
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-old",
            &[at("assistant", "2026-06-01T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        s.append_transcript_with_retention(
            "s-new",
            &[at("session_init", "2026-08-12T00:00:00Z")],
            Some(999_999_999_999),
            Some(NOW),
        )
        .expect("the flush lands");
        assert_eq!(s.transcript_rows("s-new").unwrap().len(), 1, "flushed");
        assert_eq!(s.transcript_rows("s-old").unwrap().len(), 1, "not pruned");
        // …and the operator's path is where that same window IS reported.
        let err = s
            .prune_transcripts("2026-08-12T00:00:00Z", 999_999_999_999, false)
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation);
    }

    #[test]
    fn retention_off_leaves_every_session_alone() {
        let mut s = ChatStore::open_in_memory(1);
        s.append_transcript_with_retention(
            "s-old",
            &[at("assistant", "2026-01-01T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        s.append_transcript_with_retention(
            "s-new",
            &[at("session_init", "2026-08-12T00:00:00Z")],
            None,
            None,
        )
        .unwrap();
        assert_eq!(s.transcript_rows("s-old").unwrap().len(), 1);
    }

    // ---- the configured window ------------------------------------------------------------------

    #[test]
    fn the_keep_days_setting_parses_a_count_the_off_switch_and_nothing_else() {
        assert_eq!(parse_keep_days(None).unwrap(), Some(TRANSCRIPT_KEEP_DAYS));
        assert_eq!(
            parse_keep_days(Some("")).unwrap(),
            Some(TRANSCRIPT_KEEP_DAYS)
        );
        assert_eq!(parse_keep_days(Some("7")).unwrap(), Some(7));
        assert_eq!(parse_keep_days(Some(" 7 ")).unwrap(), Some(7));
        assert_eq!(parse_keep_days(Some("0")).unwrap(), Some(0));
        // The escape hatch: this is the first thing in the store that deletes stored evidence on
        // its own, so there has to be a way to say "don't".
        assert_eq!(parse_keep_days(Some("off")).unwrap(), None);
        assert_eq!(parse_keep_days(Some("OFF")).unwrap(), None);
        for bad in ["-1", "30d", "thirty", "off!"] {
            let err = parse_keep_days(Some(bad)).unwrap_err();
            assert_eq!(err.kind, ErrorKind::Validation, "{bad}");
            assert!(err.msg.contains(KEEP_DAYS_ENV), "{bad}: {}", err.msg);
        }
    }

    #[test]
    fn the_wire_form_deserializes_camelcase_tags_and_defaults_the_absent_ones() {
        // The T1↔T2 contract verbatim: camelCase tag keys, `kind` + `data` required, every tag key
        // ABSENT (not null) when it does not apply.
        let tagged: TranscriptEntry = serde_json::from_str(
            r#"{"kind":"tool_use","at":"2026-07-26T07:42:39.123Z","toolUseId":"toolu_abc",
                "parentToolUseId":"toolu_parent","subagentType":"code-reviewer",
                "data":{"name":"Read","input":{"file_path":"/x.rs"}}}"#,
        )
        .unwrap();
        assert_eq!(tagged.tool_use_id.as_deref(), Some("toolu_abc"));
        assert_eq!(tagged.parent_tool_use_id.as_deref(), Some("toolu_parent"));
        assert_eq!(tagged.subagent_type.as_deref(), Some("code-reviewer"));

        let bare: TranscriptEntry =
            serde_json::from_str(r#"{"kind":"result","at":"2026-07-26T07:42:41.000Z","data":{}}"#)
                .unwrap();
        assert_eq!(bare.tool_use_id, None);
        assert_eq!(bare.parent_tool_use_id, None);
        assert_eq!(bare.subagent_type, None);

        // `kind` and `data` are required — a line missing either is a parse error the caller must
        // surface, not a defaulted-empty row.
        assert!(serde_json::from_str::<TranscriptEntry>(r#"{"kind":"result"}"#).is_err());
        assert!(serde_json::from_str::<TranscriptEntry>(r#"{"data":{}}"#).is_err());
    }

    #[test]
    fn a_stored_row_serializes_back_to_the_wire_shape_with_absent_tags_omitted() {
        // T3 serves these rows; an absent tag must render as an ABSENT key, exactly as it arrived,
        // so `parentToolUseId` stays the unambiguous "is this a subagent entry" signal (a `null`
        // would still be falsy in JS, but it would no longer match what the sidecar emits).
        let mut s = ChatStore::open_in_memory(1);
        append(
            &mut s,
            "s-1",
            &[entry("assistant", json!({ "text": "hi" }))],
        )
        .unwrap();
        let rows = s.transcript_rows("s-1").unwrap();
        let v = serde_json::to_value(&rows[0]).unwrap();
        assert_eq!(v["seq"], 0);
        assert_eq!(v["kind"], "assistant");
        assert!(v.get("toolUseId").is_none(), "absent tag omitted: {v}");
        assert!(
            v.get("parentToolUseId").is_none(),
            "absent tag omitted: {v}"
        );
        assert!(v.get("subagentType").is_none(), "absent tag omitted: {v}");
    }
}
