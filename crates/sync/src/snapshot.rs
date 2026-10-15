//! A replica's folded state with the relay position it reaches (6j6v.mxt2) — so a FRESH replica
//! starts from it and pulls only what came after, instead of folding the whole history again.
//!
//! [`export`] turns a synced replica into bytes; [`import`] starts an empty replica from them and
//! hands back the [`Watermarks`] its first [`sync`](crate::engine::sync) resumes from. That pair is
//! the whole seam, and the CLI (`nxs sync snapshot`, `nxs sync bind --snapshot`) speaks nothing
//! else — a host that stores snapshots itself (in its own bucket, written by its own fold job) calls
//! the same two functions.
//!
//! # What a snapshot is
//!
//! The substrate's [image](nxs_foundation::image) — the whole op log and flow's folded views — plus
//! the stream it belongs to and `pulled_through`, the relay position the log reaches. Gzipped JSON,
//! opaque to its holder. The log rides along on purpose (see the image module): a replica started
//! from a snapshot is an ordinary replica, and a snapshot whose views another engine folded is folded
//! again from that log rather than refused.
//!
//! # The promise
//!
//! **Snapshot plus the rest equals folding the whole history.** A replica that imports a snapshot
//! taken at relay position `k` and then pulls from `k` holds exactly the state of a replica that
//! pulled everything from `0` — late ops sorted into the past and re-delivered duplicates included,
//! because the rest folds through the very same keep-if-beats and idempotent union. The tests below
//! hold that against writers that go offline, a relay that stores pushes twice, and snapshots taken
//! from those writers mid-history.
//!
//! # The version stand: one relay number
//!
//! A stream lives on one relay and a workspace syncs one endpoint, so the relay's sequence number
//! (`pulled_through`) is enough to say how far a snapshot reaches; a vector per source would only be
//! needed if one replica pulled one stream from several relays, which nothing does. What the number
//! cannot say is WHICH relay's numbering it counts in. So [`export`] asks the relay for the op it
//! holds at that position — refusing when the replica does not have it, because then the marks do
//! not describe this relay — and writes that op's id into the snapshot as its anchor; [`import`]
//! asks the relay it is about to pull from for the op at the same position and requires the SAME op
//! ([`Anchor`]). A snapshot taken against another relay, or before a relay's log was rebuilt or
//! reordered (deduplicating a stream renumbers it), fails that check, and the replica then pulls
//! from the start: every op it already holds is skipped as a duplicate, so the fallback costs a
//! full pull, never an op. What the check is, precisely: a check of the numbering at one position,
//! which a different or rebuilt log almost never passes by accident — not a proof that the whole
//! prefix is the same list.
//!
//! # Who may take one, and what it reveals
//!
//! [`export`] refuses a replica holding ops the relay has not seen yet: a snapshot promises that
//! everything in it is reachable from the relay, and an importer never pushes another replica's
//! ops. It carries the log and the views folded from it — nothing a reader of the stream could not
//! fold for itself, and none of the machine-local state that shares the database. While the relay
//! authenticates nobody (6j6v.6aza) that is exactly the exposure of the stream itself; once reads
//! are scoped, a snapshot of the whole stream is only for readers whose scope covers all of it.
//! The relay does not hold snapshots — it cannot fold, and without authenticated writers a snapshot
//! planted there would be folded state nobody can check against its log; a snapshot comes from
//! storage its reader trusts.
//!
//! **A snapshot is taken as given, log and board alike** — as trustworthy as the replica it came
//! from. The import refuses what it could not read back (a log whose types or bounds are wrong, a
//! view cell of the wrong type), so a bad file cannot break the store; it does not re-derive a board
//! from the log, because that would not add trust: a crafted file can forge ops as easily as views.

use crate::engine::{SyncError, Transport, Watermarks};
use crate::protocol::{Cursor, StreamId};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use nexus_flow_core::store::Store;
use nxs_foundation::image::{Image, ImageError, Loaded};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;

/// The newest snapshot format this build reads. **Every build reads every older format**, which is
/// the compatibility promise a stored snapshot rests on (spec D3).
pub const FORMAT: u32 = 2;

/// The format [`export`] writes: the whole log, and the views folded from it (6j6v.mxt2).
pub const FORMAT_WITH_LOG: u32 = 1;

/// The format [`export_based`] writes: the state, and a [`Base`] instead of the ops below its
/// relay position (6j6v.0cfa).
pub const FORMAT_BASED: u32 = 2;

/// The most a snapshot may unpack to. A snapshot comes from storage its reader trusts, but a file
/// handed to `nxs sync bind --snapshot` could be anything, gzip packs a lot into little, and the
/// parsed form weighs several times the text. A real board of 6,065 ops unpacks to about 6 MB.
pub const MAX_UNPACKED_BYTES: u64 = 256 << 20;

/// What a snapshot says about itself — readable without loading it ([`peek`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub format: u32,
    pub stream_id: String,
    /// The relay position the snapshot's log reaches.
    pub pulled_through: i64,
    /// The id of the op the relay held at `pulled_through` when the snapshot was taken — `None`
    /// only for a snapshot through position 0. What [`import`] compares the relay against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// The nxs version that took it.
    pub engine: String,
    /// How many ops it carries.
    pub ops: usize,
    /// Each carried product's fold revision, by domain (6j6v.y3r4) — readable without the body, so
    /// a reader can tell whether this snapshot is one it could take at all before it unpacks it.
    /// Empty in format 1, which compares engine versions and refolds from its log instead.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub revisions: BTreeMap<String, i64>,
    /// The BASE, in format 2 and only there (6j6v.0cfa): what the snapshot carries instead of the
    /// ops below [`pulled_through`](Header::pulled_through). `None` means format-1 semantics — the
    /// whole log rides along.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Base>,
}

/// What a format-2 snapshot says about its base, readable from the header alone (spec §4.2).
///
/// It is the header's half of [`nxs_foundation::image::Base`]; the body carries the same facts and
/// [`import`] checks the two against each other, as it has always checked `ops` against the log.
/// The duplication is the point: a reader that only needs to SIZE the snapshot — the hosted
/// engine's op budget is the one that does (spec §4.6) — reads [`peek`] and never unpacks the body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    /// The Lamport clock the dropped prefix reached — the floor a replica started from it mints
    /// above. Without it a fresh replica would mint at 1 against registers in the hundreds, and
    /// every first write would lose in silence (spec §3, L1).
    pub clock: i64,
    /// How many distinct op ids the writer held at or below the position — with
    /// [`digest_through_p`](Base::digest_through_p), what a relay's own count and digest are
    /// compared against before a snapshot is published, so a replica that skipped a seq cannot pass
    /// the miss on (spec §4.3). Carried by this cut; the relay side of the comparison is slice 2's.
    pub ops_through_p: usize,
    /// A commutative digest over those op ids: the XOR of each id's SHA-256, hex-encoded. Order-free
    /// on purpose — the two sides enumerate the same set in different orders.
    pub digest_through_p: String,
    /// How many of the carried ops are residue: stored-not-folded at the writer (spec L5/L6).
    pub residue: usize,
    /// The domains whose ops below the position ride along in full (spec D9).
    pub kept_domains: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    #[serde(flatten)]
    header: Header,
    image: Image,
}

/// What [`import`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// Where the replica's first sync resumes. `pushed_through` stands past every imported op — none
    /// of them is this replica's to push — and `pulled_through` is the snapshot's own position, or
    /// `0` when the relay did not confirm it ([`Anchor::Mismatch`]).
    pub marks: Watermarks,
    pub header: Header,
    /// Whether the views were taken as they came or folded again from the log.
    pub loaded: Loaded,
    pub anchor: Anchor,
}

/// Whether the relay confirmed the snapshot's position (see the module doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    /// The snapshot reaches position 0 — there is nothing to confirm.
    Nothing,
    /// The relay holds, at that position, the very op the snapshot's source saw there.
    Confirmed,
    /// It does not — another relay, a log rebuilt or reordered since. The replica pulls from the
    /// start.
    Mismatch,
}

/// Why a snapshot could not be taken or loaded. Nothing was written in any of these cases.
#[derive(Debug)]
pub enum SnapshotError {
    /// The replica holds ops it has not pushed yet.
    Unpushed {
        ops: i64,
    },
    /// The relay does not hold, at the replica's `pulled_through`, an op the replica has: the
    /// watermarks do not describe THIS relay.
    Unbacked {
        position: i64,
    },
    /// The snapshot belongs to another stream than the one it is being loaded for.
    WrongStream {
        snapshot: String,
        bound: String,
    },
    /// A newer nxs wrote the snapshot in a format this one does not read.
    FormatTooNew {
        found: u32,
        supported: u32,
    },
    /// The bytes are not a snapshot.
    Corrupt(String),
    /// A BASED snapshot whose views this build cannot take as they are, and whose log it does not
    /// carry to fold instead (6j6v.0cfa, spec §4.5). The caller falls back: another snapshot, or a
    /// full replay. `ours` is `None` when this store registers no reducer for that domain at all.
    NeedsRevision {
        of: String,
        domain: String,
        snapshot: i64,
        ours: Option<i64>,
    },
    /// A product named for a based snapshot has no reducer on the exporting store, so its views are
    /// not in the image to carry — the snapshot would leave that product out. Register every
    /// platform reducer before taking one.
    NotFolded {
        domain: String,
    },
    /// A BASED snapshot that carries no views for a product THIS build folds — so taking it would
    /// leave that product with no state and no log to fold one from. Reachable when a build folds a
    /// product the writer did not carry; the caller falls back, as for a revision mismatch.
    MissingViews {
        domain: String,
    },
    /// A BASED snapshot whose position the relay does not confirm (spec §4.5 step 7). A format-1
    /// snapshot answers this by pulling from 0, which costs a full pull and no ops; a based one
    /// cannot — it holds nothing below its position — so it refuses and the caller falls back.
    AnchorMismatch {
        position: i64,
    },
    /// The substrate refused the image (a store that is not empty, a floor above this engine…).
    Image(ImageError),
    Storage(String),
    Transport(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Unpushed { ops } => write!(
                f,
                "this replica holds {ops} op(s) the relay has not seen yet — sync first, so the \
                 snapshot holds only what every replica can reach"
            ),
            SnapshotError::Unbacked { position } => write!(
                f,
                "the relay does not hold, at position {position}, an op this replica has — its \
                 watermarks do not describe this relay, so a snapshot would promise what it cannot \
                 keep; sync against this relay first"
            ),
            SnapshotError::WrongStream { snapshot, bound } => write!(
                f,
                "the snapshot belongs to stream '{snapshot}', not '{bound}'"
            ),
            SnapshotError::FormatTooNew { found, supported } => write!(
                f,
                "the snapshot is format {found} and this nxs reads up to format {supported} — \
                 upgrade nxs to start from it"
            ),
            SnapshotError::Corrupt(why) => write!(f, "not a readable snapshot: {why}"),
            SnapshotError::NeedsRevision {
                of,
                domain,
                snapshot,
                ours: Some(ours),
            } => write!(
                f,
                "the snapshot carries {of}'s views folded at revision {snapshot} and this nxs folds \
                 {domain} at {ours}; it holds no ops below its position to fold again, so start \
                 from a snapshot taken at revision {ours} or replay the stream"
            ),
            SnapshotError::NeedsRevision { of, domain, .. } => write!(
                f,
                "the snapshot carries {of}'s views and this replica registers no reducer for the \
                 {domain} domain — it would take that product's state and never fold another of \
                 its ops"
            ),
            SnapshotError::NotFolded { domain } => write!(
                f,
                "this replica registers no reducer for the {domain} domain, so a based snapshot \
                 taken here would carry none of that product's state"
            ),
            SnapshotError::MissingViews { domain } => write!(
                f,
                "the snapshot carries no views for the {domain} domain, which this nxs folds — and \
                 no ops below its position to fold them from"
            ),
            SnapshotError::AnchorMismatch { position } => write!(
                f,
                "the relay does not hold what the snapshot says sits at position {position} — \
                 another relay, or a log rebuilt since. A snapshot that carries no ops below its \
                 position cannot fall back to pulling from the start, so this one is refused"
            ),
            SnapshotError::Image(e) => write!(f, "{e}"),
            SnapshotError::Storage(m) => write!(f, "snapshot storage: {m}"),
            SnapshotError::Transport(m) => write!(f, "snapshot transport: {m}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<ImageError> for SnapshotError {
    /// The substrate's two based refusals get names of their own on this seam (spec §7): a hosted
    /// caller branches on them to pick its next source, and reaching through `Image(..)` for that
    /// would make the fallback chain depend on the substrate's error shape.
    fn from(e: ImageError) -> SnapshotError {
        match e {
            ImageError::NeedsRevision {
                of,
                domain,
                snapshot,
                ours,
            } => SnapshotError::NeedsRevision {
                of,
                domain,
                snapshot,
                ours,
            },
            ImageError::MissingViewBase { domain } => SnapshotError::MissingViews { domain },
            other => SnapshotError::Image(other),
        }
    }
}

impl From<rusqlite::Error> for SnapshotError {
    fn from(e: rusqlite::Error) -> SnapshotError {
        SnapshotError::Storage(e.to_string())
    }
}

impl From<SyncError> for SnapshotError {
    fn from(e: SyncError) -> SnapshotError {
        match e {
            SyncError::Storage(m) => SnapshotError::Storage(m),
            SyncError::Transport(m) => SnapshotError::Transport(m),
            // The pull ran; the push could not. To a snapshot start that is a transport failure, and
            // the message names the op.
            too_large @ SyncError::OpTooLarge { .. } => {
                SnapshotError::Transport(too_large.to_string())
            }
        }
    }
}

/// Take a snapshot of `store` — a replica of `stream` at `marks`, whose own ops carry
/// `local_site` — as the bytes [`import`] reads, confirming its position against the relay behind
/// `transport`.
///
/// Refuses ([`SnapshotError::Unpushed`]) while the replica holds ops of its own that it has not
/// pushed: the snapshot promises everything in it is on the relay, and the replica that imports it
/// will never push another replica's ops. And refuses ([`SnapshotError::Unbacked`]) when the relay
/// does not hold, at `marks.pulled_through`, an op this replica has — marks that describe another
/// relay, or none. The op it does hold there is written into the header as the snapshot's anchor.
/// Sync first to make `pulled_through` as current as it can be.
pub fn export(
    store: &Store,
    local_site: i64,
    stream: &StreamId,
    marks: &Watermarks,
    transport: &dyn Transport,
) -> Result<Vec<u8>, SnapshotError> {
    let unpushed: i64 = store.connection().query_row(
        "SELECT COUNT(*) FROM ops WHERE rowid > ?1 AND site = ?2",
        [marks.pushed_through, local_site],
        |r| r.get(0),
    )?;
    if unpushed > 0 {
        return Err(SnapshotError::Unpushed { ops: unpushed });
    }
    let anchor = anchor_at(store, stream, marks, transport)?;
    let image = store.image()?;
    let envelope = Envelope {
        header: Header {
            format: FORMAT_WITH_LOG,
            stream_id: stream.as_str().to_string(),
            pulled_through: marks.pulled_through,
            anchor,
            engine: image.engine.clone(),
            ops: image.op_count(),
            revisions: BTreeMap::new(),
            base: None,
        },
        image,
    };
    pack(&envelope)
}

/// One product in a based snapshot (6j6v.0cfa): the key it records its fold under, the domain whose
/// reducer folds it, and whether its ops below the position ride along in full.
///
/// **One entry per product, and it is the whole description.** The fold revision is NOT in here —
/// it is read off the store's own registered reducer, so a snapshot can never claim a revision the
/// views were not folded at. `keep_log` is spec D9: a product whose actions read per-op provenance
/// rather than only its views keeps its log, or every thread opened below the position would read
/// as held on a replica that starts here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Product {
    /// The `view_watermarks.store_id` the product records its fold under — `flow`, `chat`, `memory`.
    pub of: &'static str,
    /// The op domain its reducer folds — `task`, `message`, `fact`.
    pub domain: &'static str,
    /// Whether its ops below the position ride along in full (spec D9: `message` does).
    pub keep_log: bool,
}

/// The platform's products, as a based snapshot carries them (6j6v.0cfa) — **the one list**, so the
/// CLI that writes a snapshot and the hosted engine that reads one cannot describe the platform
/// differently.
///
/// `message` keeps its log (spec D9). chat's views are a self-contained state like the others, but
/// its ACTIONS read per-op provenance — whether the op that opened a thread may act — and E4-auth
/// forbids taking a verdict from outside, so the verdicts have to stay re-derivable. Dropping them
/// would not corrupt the board; it would hold every thread opened below the position, for ever.
/// `task` and `fact` keep nothing: everything a reader of theirs asks is in the views.
///
/// What a caller still owes: registering each of these reducers on the store before exporting, so
/// the views are in the image to carry. [`export_based`] refuses by name when one is missing.
pub const PLATFORM: &[Product] = &[
    Product {
        of: "flow",
        domain: "task",
        keep_log: false,
    },
    Product {
        of: "chat",
        domain: "message",
        keep_log: true,
    },
    Product {
        of: "memory",
        domain: "fact",
        keep_log: false,
    },
];

/// Take a BASED snapshot of `store` (6j6v.0cfa): its state, and instead of the ops below
/// `marks.pulled_through` only the [`Base`] and what the views cannot stand in for.
///
/// This is the format a reader starts from WITHOUT replaying history, and the reason the hosted
/// engine's op budget stops counting a stream's lifetime (spec §4.6). Everything [`export`] refuses
/// it refuses too, for the same reasons — unpushed ops, a position this relay does not back — plus
/// one of its own: a product in `products` whose reducer this store does not register
/// ([`SnapshotError::NotFolded`]), because its views would not be in the image to carry.
///
/// **Register every platform reducer on `store` first.** Not for size: for state. A domain whose
/// reducer is absent is a domain nothing here folded, so [`Store::based_image`] keeps its whole log
/// rather than dropping it — the snapshot stays complete either way. What an absent reducer DOES
/// cost is the point of the exercise: that product's log is exactly what was supposed to go away.
///
/// [`Store::based_image`]: nxs_foundation::store::Store::based_image
pub fn export_based(
    store: &Store,
    local_site: i64,
    stream: &StreamId,
    marks: &Watermarks,
    transport: &dyn Transport,
    products: &[Product],
) -> Result<Vec<u8>, SnapshotError> {
    let unpushed: i64 = store.connection().query_row(
        "SELECT COUNT(*) FROM ops WHERE rowid > ?1 AND site = ?2",
        [marks.pushed_through, local_site],
        |r| r.get(0),
    )?;
    if unpushed > 0 {
        return Err(SnapshotError::Unpushed { ops: unpushed });
    }
    let mut view_bases = Vec::with_capacity(products.len());
    let mut revisions = BTreeMap::new();
    let mut kept: Vec<&str> = Vec::new();
    for p in products {
        let Some(fold_revision) = store.fold_revision_of(p.domain) else {
            return Err(SnapshotError::NotFolded {
                domain: p.domain.to_string(),
            });
        };
        revisions.insert(p.domain.to_string(), fold_revision);
        view_bases.push(nxs_foundation::image::ViewBase {
            of: p.of.to_string(),
            domain: p.domain.to_string(),
            fold_revision,
        });
        if p.keep_log {
            kept.push(p.domain);
        }
    }
    // Over the WHOLE log, before the image drops any of it, and streamed rather than materialized:
    // a stream at the cap holds tens of thousands of ids.
    let (ops_through_p, digest_through_p) = digest(store)?;
    let anchor = anchor_at(store, stream, marks, transport)?;
    let image = store.based_image(marks.pulled_through, &kept, view_bases)?;
    let base = image
        .base
        .as_ref()
        .expect("based_image carries a base")
        .clone();
    let envelope = Envelope {
        header: Header {
            format: FORMAT_BASED,
            stream_id: stream.as_str().to_string(),
            pulled_through: marks.pulled_through,
            anchor,
            engine: image.engine.clone(),
            ops: image.op_count(),
            revisions,
            base: Some(Base {
                clock: base.clock,
                ops_through_p,
                digest_through_p,
                residue: base.residue,
                kept_domains: base.kept_domains.clone(),
            }),
        },
        image,
    };
    pack(&envelope)
}

/// How many ops `store` holds and a commutative digest over their ids: the XOR of each id's
/// SHA-256, hex-encoded (spec §4.3).
///
/// **XOR, not a running hash, because the two sides enumerate in different orders.** The writer
/// reads its own log; the relay scans a partition. An order-dependent digest would disagree for
/// that reason alone and say nothing about completeness, which is the only question being asked.
/// What XOR gives up is duplicate detection — a set is the right model here, and the log's primary
/// key makes an op id unique anyway.
fn digest(store: &Store) -> Result<(usize, String), SnapshotError> {
    use sha2::{Digest, Sha256};
    let conn = store.connection();
    let mut stmt = conn.prepare("SELECT op_id FROM ops")?;
    let mut rows = stmt.query([])?;
    let mut acc = [0u8; 32];
    let mut count = 0usize;
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        for (a, b) in acc.iter_mut().zip(Sha256::digest(id.as_bytes())) {
            *a ^= b;
        }
        count += 1;
    }
    let hex = acc.iter().fold(String::with_capacity(64), |mut out, b| {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
        out
    });
    Ok((count, hex))
}

/// The op the relay holds at `marks.pulled_through`, as the snapshot's anchor — refusing when this
/// replica does not have it, because then the marks do not describe this relay. Shared by both
/// exports.
fn anchor_at(
    store: &Store,
    stream: &StreamId,
    marks: &Watermarks,
    transport: &dyn Transport,
) -> Result<Option<String>, SnapshotError> {
    if marks.pulled_through <= 0 {
        return Ok(None);
    }
    let held = |id: &str| -> rusqlite::Result<bool> {
        store
            .connection()
            .query_row("SELECT EXISTS(SELECT 1 FROM ops WHERE op_id = ?1)", [id], |r| {
                r.get(0)
            })
    };
    match op_at(transport, stream, marks.pulled_through)? {
        Some(id) if held(&id)? => Ok(Some(id)),
        _ => Err(SnapshotError::Unbacked {
            position: marks.pulled_through,
        }),
    }
}

/// Gzipped JSON, the one encoding a snapshot has.
fn pack(envelope: &Envelope) -> Result<Vec<u8>, SnapshotError> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    serde_json::to_writer(&mut gz, envelope)
        .map_err(|e| SnapshotError::Storage(format!("encoding the snapshot: {e}")))?;
    gz.finish()
        .map_err(|e| SnapshotError::Storage(format!("compressing the snapshot: {e}")))
}

/// The id of the op the relay holds at `position` (1-based, a relay sequence number), if any.
fn op_at(
    transport: &dyn Transport,
    stream: &StreamId,
    position: i64,
) -> Result<Option<String>, SnapshotError> {
    let (ops, _) = transport
        .pull(stream, Cursor(position - 1), 1)
        .map_err(|e| SnapshotError::Transport(e.0))?;
    Ok(ops.into_iter().next().map(|op| op.op_id))
}

/// Unpack a snapshot's bytes, refusing more than `cap` ([`MAX_UNPACKED_BYTES`] in production).
fn unpack(bytes: &[u8], cap: u64) -> Result<Vec<u8>, SnapshotError> {
    let mut json = Vec::new();
    GzDecoder::new(bytes)
        .take(cap + 1)
        .read_to_end(&mut json)
        .map_err(|e| SnapshotError::Corrupt(format!("not gzip: {e}")))?;
    if json.len() as u64 > cap {
        return Err(SnapshotError::Corrupt(format!(
            "it unpacks to more than {cap} bytes"
        )));
    }
    Ok(json)
}

/// The format a snapshot declares — read ALONE first, so a newer format whose header changed shape
/// is refused as newer ([`SnapshotError::FormatTooNew`]) rather than as unreadable.
fn check_format(json: &[u8]) -> Result<(), SnapshotError> {
    #[derive(Deserialize)]
    struct Format {
        format: u32,
    }
    let Format { format } = serde_json::from_slice(json)
        .map_err(|e| SnapshotError::Corrupt(format!("no snapshot format: {e}")))?;
    if format > FORMAT {
        return Err(SnapshotError::FormatTooNew {
            found: format,
            supported: FORMAT,
        });
    }
    Ok(())
}

/// What a snapshot says about itself, without loading it — which stream it belongs to, how far it
/// reaches, who took it. Refuses a format this build cannot read, exactly as [`import`] would.
pub fn peek(bytes: &[u8]) -> Result<Header, SnapshotError> {
    let json = unpack(bytes, MAX_UNPACKED_BYTES)?;
    check_format(&json)?;
    serde_json::from_slice(&json)
        .map_err(|e| SnapshotError::Corrupt(format!("no snapshot header: {e}")))
}

/// The envelope opened: its header and the substrate image it carries, with every check [`import`]
/// makes that needs no store and no network (6j6v.0cfa).
///
/// **For a reader that folds the views somewhere other than a SQLite replica.** The hosted board
/// fold starts a stream from the same bytes a replica does (spec §4.5) and writes them into its own
/// table, so it needs the image rather than a loaded store — and it must not re-derive the format
/// to get at it, or there would be two readers of one format and one of them would rot. The format
/// is this module's; what is done with the views is the caller's.
pub fn unpack_image(bytes: &[u8]) -> Result<(Header, Image), SnapshotError> {
    let json = unpack(bytes, MAX_UNPACKED_BYTES)?;
    check_format(&json)?;
    let envelope: Envelope = serde_json::from_slice(&json)
        .map_err(|e| SnapshotError::Corrupt(format!("the snapshot body: {e}")))?;
    if envelope.header.ops != envelope.image.op_count() {
        return Err(SnapshotError::Corrupt(format!(
            "the header counts {} ops and the log holds {}",
            envelope.header.ops,
            envelope.image.op_count()
        )));
    }
    check_base(&envelope)?;
    envelope.image.check()?;
    Ok((envelope.header, envelope.image))
}

/// Start the EMPTY `store` from a snapshot of `stream`, confirming its position against the relay
/// behind `transport` — the one network round-trip, made after every local refusal and before
/// anything is written.
///
/// On success the store holds the snapshot's log and flow's views (taken, or folded again — see
/// [`Imported::loaded`]), and [`Imported::marks`] is where its first
/// [`sync`](crate::engine::sync) resumes. A snapshot of another stream, a format or schema floor
/// this build cannot read, a log the store could not read back, and a store that already holds ops
/// are all refused by name.
///
/// A replica that will WRITE registers its id prefix before importing
/// ([`register_prefix`](crate::engine::register_prefix)), exactly as it would before its first
/// merge: a prefix reassignment remaps only the replica's own ids, which is sound only while every
/// id carrying that prefix is its own — the case on an empty store, and not after a whole foreign
/// log arrived.
pub fn import(
    store: &mut Store,
    bytes: &[u8],
    stream: &StreamId,
    transport: &dyn Transport,
) -> Result<Imported, SnapshotError> {
    import_bounded(store, bytes, stream, transport, MAX_UNPACKED_BYTES)
}

fn import_bounded(
    store: &mut Store,
    bytes: &[u8],
    stream: &StreamId,
    transport: &dyn Transport,
    cap: u64,
) -> Result<Imported, SnapshotError> {
    let json = unpack(bytes, cap)?;
    check_format(&json)?;
    let envelope: Envelope = serde_json::from_slice(&json)
        .map_err(|e| SnapshotError::Corrupt(format!("the snapshot body: {e}")))?;
    let header = &envelope.header;
    if header.stream_id != stream.as_str() {
        return Err(SnapshotError::WrongStream {
            snapshot: header.stream_id.clone(),
            bound: stream.as_str().to_string(),
        });
    }
    // What the header promises must be what the body is.
    if header.pulled_through < 0 {
        return Err(SnapshotError::Corrupt(format!(
            "a negative position ({})",
            header.pulled_through
        )));
    }
    if header.ops != envelope.image.op_count() {
        return Err(SnapshotError::Corrupt(format!(
            "the header counts {} ops and the log holds {}",
            header.ops,
            envelope.image.op_count()
        )));
    }
    match (&header.anchor, header.pulled_through) {
        (None, 0) => {}
        // A based snapshot drops the ops below its position, the one at it included, so the anchor
        // is not in the log it carries. What the relay holds there is checked by `confirm`; the
        // anchor's own presence is a format-1 self-consistency check and stays one.
        (Some(_), p) if p > 0 && envelope.image.based() => {}
        (Some(anchor), p) if p > 0 && envelope.image.op_ids().contains(anchor.as_str()) => {}
        _ => {
            return Err(SnapshotError::Corrupt(
                "its anchor does not match its position and log".to_string(),
            ))
        }
    }
    check_base(&envelope)?;
    // Every refusal that needs no network, before the one call that does.
    envelope.image.check()?;
    let held = store.op_count();
    if held > 0 {
        return Err(ImageError::NotEmpty { ops: held }.into());
    }
    if envelope.image.min_compatible > nxs_foundation::schema::SCHEMA_VERSION {
        return Err(ImageError::TooNew {
            min_compatible: envelope.image.min_compatible,
            ours: nxs_foundation::schema::SCHEMA_VERSION,
        }
        .into());
    }
    let anchor = confirm(&envelope, stream, transport)?;
    // A format-1 snapshot answers an unconfirmed position by pulling from 0: it carries every op,
    // so the fallback costs a full pull and loses nothing. A based one holds nothing below its
    // position, so pulling from 0 would leave the gap between 0 and P forever empty while the
    // watermarks claimed otherwise. It refuses instead, and the caller falls back (spec §4.5).
    if envelope.image.based() && anchor == Anchor::Mismatch {
        return Err(SnapshotError::AnchorMismatch {
            position: envelope.header.pulled_through,
        });
    }
    let loaded = store.load_image(&envelope.image)?;
    Ok(Imported {
        marks: Watermarks {
            // Taken from the image, not read back after the load: a write that lands in between
            // must stay this replica's to push.
            pushed_through: envelope.image.max_rowid(),
            pulled_through: match anchor {
                Anchor::Mismatch => 0,
                Anchor::Nothing | Anchor::Confirmed => envelope.header.pulled_through,
            },
        },
        header: envelope.header,
        loaded,
        anchor,
    })
}

/// What the header says about the base must be what the body is (6j6v.0cfa) — the same rule the
/// `ops` count has always been held to.
///
/// **Why the two halves exist at all.** The header is read alone by [`peek`], and the hosted
/// engine's op budget is sized off it without ever unpacking the body (spec §4.6). So the facts a
/// sizer needs are in both places, and a snapshot whose halves disagree is refused rather than
/// believed in the half that happens to be read first.
fn check_base(envelope: &Envelope) -> Result<(), SnapshotError> {
    let bad = |why: String| SnapshotError::Corrupt(why);
    match (&envelope.header.base, &envelope.image.base) {
        (None, None) => {
            if envelope.header.format >= FORMAT_BASED {
                return Err(bad(format!(
                    "it declares format {} and carries no base",
                    envelope.header.format
                )));
            }
            return Ok(());
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(bad("only one of its halves carries a base".to_string()))
        }
        (Some(header), Some(image)) => {
            if envelope.header.format < FORMAT_BASED {
                return Err(bad(format!(
                    "it carries a base and declares format {}",
                    envelope.header.format
                )));
            }
            if image.relay_position != envelope.header.pulled_through {
                return Err(bad(format!(
                    "its base stands at position {} and its header at {}",
                    image.relay_position, envelope.header.pulled_through
                )));
            }
            if header.clock != image.clock {
                return Err(bad(format!(
                    "its header carries clock {} and its base {}",
                    header.clock, image.clock
                )));
            }
            if header.residue != image.residue {
                return Err(bad(format!(
                    "its header counts {} residue ops and its base {}",
                    header.residue, image.residue
                )));
            }
            if header.kept_domains != image.kept_domains {
                return Err(bad(format!(
                    "its header keeps {:?} and its base {:?}",
                    header.kept_domains, image.kept_domains
                )));
            }
            let theirs: std::collections::BTreeMap<String, i64> = image
                .view_bases
                .iter()
                .map(|v| (v.domain.clone(), v.fold_revision))
                .collect();
            if envelope.header.revisions != theirs {
                return Err(bad(format!(
                    "its header names revisions {:?} and its views {:?}",
                    envelope.header.revisions, theirs
                )));
            }
            if header.residue > envelope.image.op_count() {
                return Err(bad(format!(
                    "it counts {} residue ops and carries {} ops in all",
                    header.residue,
                    envelope.image.op_count()
                )));
            }
        }
    }
    Ok(())
}

/// Ask the relay for the op at the snapshot's position and compare it with the anchor.
fn confirm(
    envelope: &Envelope,
    stream: &StreamId,
    transport: &dyn Transport,
) -> Result<Anchor, SnapshotError> {
    let Some(anchor) = &envelope.header.anchor else {
        return Ok(Anchor::Nothing);
    };
    Ok(
        match op_at(transport, stream, envelope.header.pulled_through)? {
            Some(id) if &id == anchor => Anchor::Confirmed,
            _ => Anchor::Mismatch,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{self, TransportError, Unbounded};
    use crate::protocol::RegisterOutcome;
    use crate::wire::WireOp;
    use nexus_flow_core::derive;
    use nexus_flow_core::model::EdgeKind;
    use nxs_foundation::image::RefoldReason;
    use std::cell::RefCell;

    const NOW: &str = "2030-01-01T00:00:00Z";

    /// Seeded xorshift — deterministic, no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn chance(&mut self, pct: u64) -> bool {
            self.below(100) < pct
        }
        fn pick<'a, T>(&mut self, of: &'a [T]) -> &'a T {
            &of[self.below(of.len() as u64) as usize]
        }
    }

    /// The relay double: an append-only log, seq = position + 1 — and, like the DynamoDB backend
    /// before it appended idempotently (6j6v.4qjw, fixed by 6j6v.tm4k; streams written before
    /// then still hold the duplicates), one that stores a push TWICE `twice_pct` percent of the
    /// time.
    struct Relay {
        log: RefCell<Vec<WireOp>>,
        twice_pct: u64,
        rng: RefCell<Rng>,
    }

    impl Relay {
        fn new(seed: u64, twice_pct: u64) -> Relay {
            Relay {
                log: RefCell::new(Vec::new()),
                twice_pct,
                rng: RefCell::new(Rng(seed ^ 0x9e37_79b9_7f4a_7c15)),
            }
        }
        fn len(&self) -> usize {
            self.log.borrow().len()
        }
    }

    impl Transport for Relay {
        fn push(&self, _: &StreamId, ops: &[WireOp]) -> Result<(), TransportError> {
            let mut log = self.log.borrow_mut();
            log.extend_from_slice(ops);
            if self.rng.borrow_mut().chance(self.twice_pct) {
                log.extend_from_slice(ops);
            }
            Ok(())
        }
        fn pull(
            &self,
            _: &StreamId,
            since: Cursor,
            limit: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            let log = self.log.borrow();
            let start = (since.0.max(0) as usize).min(log.len());
            let page: Vec<WireOp> = log[start..].iter().take(limit).cloned().collect();
            let next = Cursor((start + page.len()) as i64);
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

    /// The relay as it stood when it held only its first `len` entries — what a reader that synced
    /// back then saw.
    struct Prefix<'a> {
        relay: &'a Relay,
        len: usize,
    }

    impl Transport for Prefix<'_> {
        fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
            unreachable!("a reader pushes nothing")
        }
        fn pull(
            &self,
            stream: &StreamId,
            since: Cursor,
            limit: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            let room = (self.len as i64 - since.0).max(0) as usize;
            self.relay.pull(stream, since, limit.min(room))
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

    fn stream() -> StreamId {
        StreamId("s".into())
    }

    fn pass(
        store: &mut Store,
        site: i64,
        marks: &mut Watermarks,
        t: &dyn Transport,
    ) -> engine::SyncOutcome {
        engine::sync(store, site, &stream(), marks, t, 7, &Unbounded).unwrap()
    }

    /// Everything flow folded, rendered order-free: every view table's rows sorted, the log's op
    /// set, and what the board derives from them — the lists `nxf next`/`nxf blocked` show.
    fn state(s: &Store) -> Vec<String> {
        // The items as the board reads them, beside the raw views: a table missing from
        // `view_tables` would drop out of BOTH sides of an `image()`-only comparison.
        let mut out: Vec<String> = s
            .list_items()
            .unwrap()
            .iter()
            .map(|i| format!("{i:?}"))
            .collect();
        for table in s.image().unwrap().views {
            let mut rows: Vec<String> = table.rows.iter().map(|r| format!("{r:?}")).collect();
            rows.sort();
            out.push(format!("{} = {rows:?}", table.name));
        }
        let mut ids: Vec<String> = s.export().into_iter().map(|o| o.op_id).collect();
        ids.sort();
        out.push(format!("ops = {ids:?}"));
        let c = s.connection();
        out.push(format!("ready = {:?}", derive::ready(c, NOW).unwrap()));
        out.push(format!("blocked = {:?}", derive::blocked(c).unwrap()));
        out.push(format!(
            "in_progress = {:?}",
            derive::in_progress(c, NOW).unwrap()
        ));
        out.push(format!(
            "next = {:?}",
            derive::next_candidates(c, NOW).unwrap()
        ));
        out
    }

    struct Writer {
        store: Store,
        site: i64,
        prefix: &'static str,
        marks: Watermarks,
        made: usize,
        offline: u64,
    }

    /// One random edit — the kinds of op a board sees: items, their registers, the dependency
    /// OR-set, labels, notes.
    fn edit(w: &mut Writer, rng: &mut Rng) {
        let items: Vec<String> = w
            .store
            .list_items()
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect();
        let author = w.prefix;
        if items.len() < 2 || (w.made < 8 && rng.chance(20)) {
            w.made += 1;
            let id = format!("{}.{}", w.prefix, w.made);
            w.store.create_item(&id, "task", "t", author);
            return;
        }
        let a = rng.pick(&items).clone();
        let b = rng.pick(&items).clone();
        let n = rng.next() % 1000;
        match rng.below(9) {
            0 => w
                .store
                .set_field(&a, "title", Some(format!("title {n}")), author),
            1 => w
                .store
                .set_field(&a, "description", Some(format!("d {n}")), author),
            2 => {
                let status = rng.pick(&["open", "in_progress", "closed"]).to_string();
                w.store.set_field(&a, "status", Some(status), author)
            }
            3 => {
                let priority = rng.pick(&["0", "1", "2", "3", "4"]).to_string();
                w.store.set_field(&a, "priority", Some(priority), author)
            }
            4 | 5 if a != b => w.store.add_edge(&a, &b, EdgeKind::Dep, author),
            6 if a != b => w.store.remove_edge(&a, &b, EdgeKind::Dep, author),
            7 => {
                let label = rng.pick(&["x", "y"]).to_string();
                if rng.chance(50) {
                    w.store.add_label(&a, &label, author)
                } else {
                    w.store.remove_label(&a, &label, author)
                }
            }
            _ => {
                w.store.add_note(&a, &format!("note {n}"), author);
            }
        }
    }

    /// A history with everything the promise has to survive — writers that go offline and push late
    /// (their ops sort into the past), a relay that stores some pushes twice — plus the snapshots
    /// taken along the way: by writers right after a pass, and by a reader at chosen cut points.
    fn history(seed: u64) -> (Relay, Vec<(String, Vec<u8>)>) {
        let relay = Relay::new(seed, 25);
        let mut rng = Rng(seed);
        let mut writers: Vec<Writer> = [(11, "wa"), (22, "wb"), (33, "wc")]
            .into_iter()
            .map(|(site, prefix)| Writer {
                store: Store::open_in_memory(site),
                site,
                prefix,
                marks: Watermarks::default(),
                made: 0,
                offline: 0,
            })
            .collect();
        let mut snapshots = Vec::new();
        for step in 0..80 {
            let w = &mut writers[rng.below(3) as usize];
            edit(w, &mut rng);
            if w.offline > 0 {
                w.offline -= 1;
                continue;
            }
            match rng.below(10) {
                0..=4 => {
                    pass(&mut w.store, w.site, &mut w.marks, &relay);
                    if rng.chance(15) {
                        let bytes = export(&w.store, w.site, &stream(), &w.marks, &relay).unwrap();
                        snapshots.push((format!("writer {} at step {step}", w.prefix), bytes));
                    }
                }
                5 => w.offline = 4 + rng.below(8),
                _ => {}
            }
        }
        for _ in 0..2 {
            for w in &mut writers {
                pass(&mut w.store, w.site, &mut w.marks, &relay);
            }
        }
        let n = relay.len();
        let cuts = [
            0,
            1,
            n / 3,
            n / 2,
            n - 1,
            n,
            rng.below(n as u64 + 1) as usize,
        ];
        for cut in cuts {
            let mut reader = Store::open_in_memory(44);
            let mut marks = Watermarks::default();
            pass(
                &mut reader,
                44,
                &mut marks,
                &Prefix {
                    relay: &relay,
                    len: cut,
                },
            );
            assert_eq!(marks.pulled_through, cut as i64);
            let seen = Prefix {
                relay: &relay,
                len: cut,
            };
            let bytes = export(&reader, 44, &stream(), &marks, &seen).unwrap();
            snapshots.push((format!("reader at position {cut} of {n}"), bytes));
        }
        (relay, snapshots)
    }

    /// Re-encode a snapshot with its image changed — how a test gets one "another engine" wrote.
    fn tampered(bytes: &[u8], change: impl FnOnce(&mut Envelope)) -> Vec<u8> {
        let mut envelope: Envelope =
            serde_json::from_slice(&unpack(bytes, MAX_UNPACKED_BYTES).unwrap()).unwrap();
        change(&mut envelope);
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        serde_json::to_writer(&mut gz, &envelope).unwrap();
        gz.finish().unwrap()
    }

    /// **The promise (6j6v.mxt2's DoD): snapshot plus the rest equals folding the whole history** —
    /// with late ops sorted into the past and duplicated deliveries, for snapshots taken by writers
    /// mid-history and by a reader at every kind of cut point, and for a snapshot whose views
    /// another engine folded.
    #[test]
    fn a_snapshot_plus_the_rest_folds_to_exactly_the_whole_history() {
        let mut compared = 0;
        let mut taken_as_they_were = 0;
        // The hard case, WITNESSED rather than assumed (review of PR #487, Test Quality #4): an op
        // in the rest that sorts BELOW an op of the snapshot touching the same register — it must
        // lose to what the snapshot already folded, exactly as it does in the full fold.
        let mut late_into_the_past = 0;
        for seed in 1..=12u64 {
            let (relay, snapshots) = history(seed * 7919);
            let mut full = Store::open_in_memory(99);
            let mut full_marks = Watermarks::default();
            pass(&mut full, 99, &mut full_marks, &relay);
            let expected = state(&full);
            assert!(
                relay.len() > full.op_count() as usize,
                "seed {seed}: the relay must hold duplicated deliveries for this to test them"
            );

            for (n, (taken_by, bytes)) in snapshots.iter().enumerate() {
                // Every snapshot as it was taken; every third one also as another engine's.
                let variants: &[bool] = if n.is_multiple_of(3) {
                    &[false, true]
                } else {
                    &[false]
                };
                for &other_engine in variants {
                    let bytes = if other_engine {
                        tampered(bytes, |e| e.image.engine = "0.0.1".into())
                    } else {
                        bytes.clone()
                    };
                    let mut fresh = Store::open_in_memory(99);
                    let imported = import(&mut fresh, &bytes, &stream(), &relay)
                        .unwrap_or_else(|e| panic!("seed {seed}, {taken_by}: {e}"));
                    assert_ne!(imported.anchor, Anchor::Mismatch, "seed {seed}, {taken_by}");
                    match (&imported.loaded, other_engine) {
                        (Loaded::Views, false) => taken_as_they_were += 1,
                        (Loaded::Refolded(RefoldReason::OtherEngine { .. }), true) => {}
                        (loaded, _) => panic!("seed {seed}, {taken_by}: {loaded:?}"),
                    }
                    let mut marks = imported.marks;
                    {
                        let log = relay.log.borrow();
                        let cut = imported.header.pulled_through as usize;
                        let mut folded: std::collections::HashMap<(&str, &str), (i64, i64)> =
                            std::collections::HashMap::new();
                        for b in &log[..cut] {
                            let top = folded
                                .entry((b.target_id.as_str(), b.field.as_str()))
                                .or_insert((b.lamport, b.site));
                            *top = (*top).max((b.lamport, b.site));
                        }
                        late_into_the_past += log[cut..]
                            .iter()
                            .filter(|late| {
                                folded
                                    .get(&(late.target_id.as_str(), late.field.as_str()))
                                    .is_some_and(|top| *top > (late.lamport, late.site))
                            })
                            .count();
                    }
                    let rest = pass(&mut fresh, 99, &mut marks, &relay);
                    assert_eq!(
                        rest.pulled,
                        relay.len() - imported.header.pulled_through as usize,
                        "seed {seed}, {taken_by}: it pulls ONLY what came after the snapshot"
                    );
                    assert_eq!(
                        state(&fresh),
                        expected,
                        "seed {seed}, {taken_by} (other engine: {other_engine})"
                    );
                    compared += 1;
                }
            }
        }
        assert!(compared > 120, "{compared} comparisons");
        assert!(
            late_into_the_past > 20,
            "only {late_into_the_past} rest ops sorted below a register the snapshot had folded"
        );
        assert!(
            taken_as_they_were > 90,
            "{taken_as_they_were} snapshots took their views"
        );
    }

    /// A relay whose log was rebuilt — here, deduplicated, as cleaning up a DynamoDB stream
    /// doubled before 6j6v.tm4k would — renumbers every entry after the first duplicate. A
    /// snapshot's position then points somewhere else, and trusting it would skip ops. The anchor
    /// catches it, and the replica pulls from the start instead: slower, never short.
    #[test]
    fn a_position_the_relay_does_not_confirm_is_pulled_from_the_start_instead_of_trusted() {
        let doubled = Relay::new(1, 100);
        let mut writer = Store::open_in_memory(11);
        let mut marks = Watermarks::default();
        writer.create_item("wa.1", "task", "first", "wa");
        pass(&mut writer, 11, &mut marks, &doubled); // stored twice
        let first_ops = writer.op_count() as usize;
        let mut reader = Store::open_in_memory(44);
        let mut reader_marks = Watermarks::default();
        pass(&mut reader, 44, &mut reader_marks, &doubled);
        let snapshot = export(&reader, 44, &stream(), &reader_marks, &doubled).unwrap();
        assert_eq!(reader_marks.pulled_through as usize, 2 * first_ops);

        writer.create_item("wa.2", "task", "second", "wa");
        pass(&mut writer, 11, &mut marks, &doubled);

        // The same stream, rebuilt without its duplicates: every op once, in first-seen order.
        let rebuilt = Relay::new(2, 0);
        let mut seen = std::collections::HashSet::new();
        for op in doubled.log.borrow().iter() {
            if seen.insert(op.op_id.clone()) {
                rebuilt.log.borrow_mut().push(op.clone());
            }
        }
        assert!(
            rebuilt.len() <= 2 * first_ops,
            "precondition: trusting the old position would pull NOTHING from the rebuilt relay"
        );

        let mut fresh = Store::open_in_memory(99);
        let imported = import(&mut fresh, &snapshot, &stream(), &rebuilt).unwrap();
        assert_eq!(imported.anchor, Anchor::Mismatch);
        assert_eq!(
            imported.marks.pulled_through, 0,
            "…so it pulls from the start"
        );
        let mut m = imported.marks;
        pass(&mut fresh, 99, &mut m, &rebuilt);
        assert!(
            fresh.get_item("wa.2").unwrap().is_some(),
            "the op after the snapshot arrived"
        );
        let mut full = Store::open_in_memory(98);
        pass(&mut full, 98, &mut Watermarks::default(), &rebuilt);
        assert_eq!(state(&fresh), state(&full));

        // Against the relay it was taken from, the same snapshot is confirmed and resumes.
        let mut fresh = Store::open_in_memory(97);
        let imported = import(&mut fresh, &snapshot, &stream(), &doubled).unwrap();
        assert_eq!(imported.anchor, Anchor::Confirmed);
        assert_eq!(imported.marks.pulled_through as usize, 2 * first_ops);
    }

    #[test]
    fn a_replica_with_ops_it_has_not_pushed_takes_no_snapshot() {
        let relay = Relay::new(3, 0);
        let mut w = Store::open_in_memory(11);
        let mut marks = Watermarks::default();
        w.create_item("wa.1", "task", "t", "wa");
        pass(&mut w, 11, &mut marks, &relay);
        export(&w, 11, &stream(), &marks, &relay).expect("everything pushed");
        w.set_field("wa.1", "title", Some("unpushed".into()), "wa");
        let err = export(&w, 11, &stream(), &marks, &relay).unwrap_err();
        assert!(matches!(err, SnapshotError::Unpushed { ops: 1 }), "{err}");
        assert!(err.to_string().contains("sync first"), "{err}");
    }

    /// Each refusal is by name, and leaves the store as it was.
    #[test]
    fn a_snapshot_that_cannot_be_read_as_this_replicas_start_is_refused_by_name() {
        let relay = Relay::new(4, 0);
        let mut w = Store::open_in_memory(11);
        let mut marks = Watermarks::default();
        w.create_item("wa.1", "task", "t", "wa");
        pass(&mut w, 11, &mut marks, &relay);
        let good = export(&w, 11, &stream(), &marks, &relay).unwrap();

        let refused = |bytes: &[u8], stream: &StreamId| {
            let mut fresh = Store::open_in_memory(99);
            let err = import(&mut fresh, bytes, stream, &relay).unwrap_err();
            assert_eq!(fresh.op_count(), 0, "nothing written for: {err}");
            err
        };
        let err = refused(&good, &StreamId("another".into()));
        assert!(matches!(err, SnapshotError::WrongStream { .. }), "{err}");

        let newer = tampered(&good, |e| e.header.format = FORMAT + 1);
        let err = refused(&newer, &stream());
        assert!(err.to_string().contains("upgrade nxs"), "{err}");
        assert!(matches!(
            peek(&newer),
            Err(SnapshotError::FormatTooNew { .. })
        ));

        let locked = tampered(&good, |e| {
            e.image.min_compatible = nxs_foundation::schema::SCHEMA_VERSION + 1
        });
        let err = refused(&locked, &stream());
        assert!(err.to_string().contains("upgrade nxs"), "{err}");

        let err = refused(b"not a snapshot", &stream());
        assert!(matches!(err, SnapshotError::Corrupt(_)), "{err}");

        let mut busy = Store::open_in_memory(99);
        busy.create_item("mine.1", "task", "t", "me");
        let err = import(&mut busy, &good, &stream(), &relay).unwrap_err();
        assert!(err.to_string().contains("fresh replica"), "{err}");
    }

    #[test]
    fn peek_reads_what_a_snapshot_says_about_itself() {
        let relay = Relay::new(5, 0);
        let mut w = Store::open_in_memory(11);
        let mut marks = Watermarks::default();
        w.create_item("wa.1", "task", "t", "wa");
        pass(&mut w, 11, &mut marks, &relay);
        let header = peek(&export(&w, 11, &stream(), &marks, &relay).unwrap()).unwrap();
        assert!(
            header.anchor.is_some(),
            "a snapshot past position 0 carries its anchor"
        );
        assert_eq!(
            header,
            Header {
                format: FORMAT_WITH_LOG,
                stream_id: "s".into(),
                pulled_through: marks.pulled_through,
                anchor: header.anchor.clone(),
                engine: nxs_foundation::image::ENGINE_VERSION.into(),
                ops: w.op_count() as usize,
                revisions: BTreeMap::new(),
                base: None,
            },
            "the with-log export still writes format 1, with no base and no revisions"
        );
    }

    fn op(op_id: &str, lamport: i64, site: i64, target: &str, field: &str, value: &str) -> WireOp {
        WireOp {
            envelope_version: crate::wire::ENVELOPE_VERSION,
            op_id: op_id.into(),
            lamport,
            site,
            domain: "task".into(),
            target_kind: "item".into(),
            target_id: target.into(),
            field: field.into(),
            op_type: "set".into(),
            value: Some(value.into()),
            author: "w".into(),
            wall_clock: String::new(),
            extra: Default::default(),
        }
    }

    /// The hard case of the promise, directed (review of PR #487, Test Quality #4): the snapshot
    /// folded `title` at lamport 9; the rest brings `title` at 3, which must LOSE, and
    /// `description` at 4, which must WIN — the rest is folded into the past, not on top.
    #[test]
    fn a_late_op_from_the_past_loses_to_what_the_snapshot_folded_and_a_fresh_one_wins() {
        let relay = Relay::new(6, 0);
        relay.log.borrow_mut().extend([
            op("c1", 1, 2, "x.1", "type", "task"),
            op("c2", 2, 2, "x.1", "title", "first"),
            op("t9", 9, 2, "x.1", "title", "from lamport 9"),
        ]);
        let mut reader = Store::open_in_memory(44);
        let mut marks = Watermarks::default();
        pass(&mut reader, 44, &mut marks, &relay);
        let snapshot = export(&reader, 44, &stream(), &marks, &relay).unwrap();
        relay.log.borrow_mut().extend([
            op("t3", 3, 3, "x.1", "title", "from lamport 3, late"),
            op("d4", 4, 3, "x.1", "description", "from lamport 4, late"),
        ]);

        let mut fresh = Store::open_in_memory(99);
        let mut m = import(&mut fresh, &snapshot, &stream(), &relay)
            .unwrap()
            .marks;
        pass(&mut fresh, 99, &mut m, &relay);
        let item = fresh.get_item("x.1").unwrap().unwrap();
        assert_eq!(item.title.as_deref(), Some("from lamport 9"));
        assert_eq!(item.description.as_deref(), Some("from lamport 4, late"));
        let mut full = Store::open_in_memory(98);
        pass(&mut full, 98, &mut Watermarks::default(), &relay);
        assert_eq!(state(&fresh), state(&full));
    }

    /// A replica that started from a snapshot writes, and its write reaches the relay (review of PR
    /// #487, Test Quality #3): `pushed_through` stands past the imported log and not one op further,
    /// so its first own op is pushed and none of the imported ones are.
    #[test]
    fn a_replica_started_from_a_snapshot_pushes_its_own_first_write_and_nothing_it_imported() {
        let relay = Relay::new(7, 0);
        let mut w = Store::open_in_memory(11);
        let mut wm = Watermarks::default();
        w.create_item("wa.1", "task", "t", "wa");
        pass(&mut w, 11, &mut wm, &relay);
        let snapshot = export(&w, 11, &stream(), &wm, &relay).unwrap();
        let on_relay = relay.len();

        let mut fresh = Store::open_in_memory(99);
        let imported = import(&mut fresh, &snapshot, &stream(), &relay).unwrap();
        assert_eq!(imported.marks.pushed_through, fresh.op_count());
        let mut m = imported.marks;
        fresh.set_field(
            "wa.1",
            "title",
            Some("written after the snapshot".into()),
            "me",
        );
        let out = pass(&mut fresh, 99, &mut m, &relay);
        assert_eq!(out.pushed, 1, "its own write, and only that");
        assert_eq!(relay.len(), on_relay + 1);

        let mut other = Store::open_in_memory(98);
        pass(&mut other, 98, &mut Watermarks::default(), &relay);
        assert_eq!(
            other.get_item("wa.1").unwrap().unwrap().title.as_deref(),
            Some("written after the snapshot")
        );
    }

    /// The anchor is the op AT the position, not any op of the snapshot (review of PR #487, Code
    /// Quality #3): a relay holding the same ops in another order puts a different one there, and
    /// the replica pulls from the start although the op it finds is one the snapshot holds.
    #[test]
    fn a_relay_that_holds_another_op_at_the_position_is_not_trusted_even_one_the_snapshot_has() {
        let relay = Relay::new(8, 0);
        relay.log.borrow_mut().extend([
            op("a", 1, 2, "x.1", "type", "task"),
            op("b", 2, 2, "x.1", "title", "b"),
            op("c", 3, 2, "x.1", "title", "c"),
        ]);
        let mut reader = Store::open_in_memory(44);
        let mut marks = Watermarks::default();
        pass(&mut reader, 44, &mut marks, &relay);
        let snapshot = export(&reader, 44, &stream(), &marks, &relay).unwrap();
        assert_eq!(peek(&snapshot).unwrap().anchor.as_deref(), Some("c"));

        let reordered = Relay::new(9, 0);
        reordered.log.borrow_mut().extend([
            op("a", 1, 2, "x.1", "type", "task"),
            op("c", 3, 2, "x.1", "title", "c"),
            op("b", 2, 2, "x.1", "title", "b"),
            op("d", 4, 2, "x.1", "description", "d"),
        ]);
        let mut fresh = Store::open_in_memory(99);
        let imported = import(&mut fresh, &snapshot, &stream(), &reordered).unwrap();
        assert_eq!(
            imported.anchor,
            Anchor::Mismatch,
            "b sits at 3, the snapshot saw c"
        );
        assert_eq!(imported.marks.pulled_through, 0);

        // A relay shorter than the position holds nothing there at all.
        let short = Relay::new(10, 0);
        short
            .log
            .borrow_mut()
            .push(op("a", 1, 2, "x.1", "type", "task"));
        let mut fresh = Store::open_in_memory(97);
        let imported = import(&mut fresh, &snapshot, &stream(), &short).unwrap();
        assert_eq!(imported.anchor, Anchor::Mismatch);
    }

    /// A transport that fails.
    struct Down;
    impl Transport for Down {
        fn push(&self, _: &StreamId, _: &[WireOp]) -> Result<(), TransportError> {
            Err(TransportError("down".into()))
        }
        fn pull(
            &self,
            _: &StreamId,
            _: Cursor,
            _: usize,
        ) -> Result<(Vec<WireOp>, Cursor), TransportError> {
            Err(TransportError("down".into()))
        }
        fn register(
            &self,
            _: &StreamId,
            _: &str,
            _: &str,
        ) -> Result<RegisterOutcome, TransportError> {
            Err(TransportError("down".into()))
        }
    }

    /// The position is confirmed before a single row is written: a relay that cannot be asked
    /// leaves the store as it was (review of PR #487, Test Quality #2) — and `export` does not take
    /// a snapshot whose marks it cannot back.
    #[test]
    fn nothing_is_written_when_the_relay_cannot_confirm_and_no_snapshot_is_taken_unbacked() {
        let relay = Relay::new(11, 0);
        let mut w = Store::open_in_memory(11);
        let mut wm = Watermarks::default();
        w.create_item("wa.1", "task", "t", "wa");
        pass(&mut w, 11, &mut wm, &relay);
        let snapshot = export(&w, 11, &stream(), &wm, &relay).unwrap();

        let mut fresh = Store::open_in_memory(99);
        let err = import(&mut fresh, &snapshot, &stream(), &Down).unwrap_err();
        assert!(matches!(err, SnapshotError::Transport(_)), "{err}");
        assert_eq!(fresh.op_count(), 0);

        assert!(matches!(
            export(&w, 11, &stream(), &wm, &Down),
            Err(SnapshotError::Transport(_))
        ));
        let elsewhere = Relay::new(12, 0);
        let err = export(&w, 11, &stream(), &wm, &elsewhere).unwrap_err();
        assert!(matches!(err, SnapshotError::Unbacked { .. }), "{err}");
    }

    /// What the header promises has to be what the body is, and the unpacked size is bounded.
    #[test]
    fn a_header_that_does_not_match_its_body_and_an_oversized_snapshot_are_refused() {
        let relay = Relay::new(13, 0);
        let mut w = Store::open_in_memory(11);
        let mut wm = Watermarks::default();
        w.create_item("wa.1", "task", "t", "wa");
        pass(&mut w, 11, &mut wm, &relay);
        let good = export(&w, 11, &stream(), &wm, &relay).unwrap();
        for (what, bytes) in [
            (
                "a negative position",
                tampered(&good, |e| e.header.pulled_through = -5),
            ),
            ("a wrong op count", tampered(&good, |e| e.header.ops += 1)),
            (
                "an anchor not in the log",
                tampered(&good, |e| e.header.anchor = Some("nope".into())),
            ),
            (
                "no anchor past position 0",
                tampered(&good, |e| e.header.anchor = None),
            ),
        ] {
            let mut fresh = Store::open_in_memory(99);
            let err = import(&mut fresh, &bytes, &stream(), &relay).expect_err(what);
            assert!(matches!(err, SnapshotError::Corrupt(_)), "{what}: {err}");
            assert_eq!(fresh.op_count(), 0);
        }
        let mut fresh = Store::open_in_memory(99);
        let err = import_bounded(&mut fresh, &good, &stream(), &relay, 64).unwrap_err();
        assert!(err.to_string().contains("more than 64 bytes"), "{err}");
    }

    /// A newer format is refused AS newer even when its header changed shape.
    #[test]
    fn a_newer_format_with_another_header_is_refused_as_newer_not_as_unreadable() {
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        serde_json::to_writer(
            &mut gz,
            &serde_json::json!({"format": FORMAT + 1, "stream": {"id": "s"}, "image": {}}),
        )
        .unwrap();
        let bytes = gz.finish().unwrap();
        assert!(matches!(
            peek(&bytes),
            Err(SnapshotError::FormatTooNew { .. })
        ));
        let mut fresh = Store::open_in_memory(99);
        assert!(matches!(
            import(&mut fresh, &bytes, &stream(), &Relay::new(14, 0)),
            Err(SnapshotError::FormatTooNew { .. })
        ));
    }

    #[test]
    fn an_op_too_large_reaches_a_snapshot_start_as_a_transport_failure_naming_it() {
        let err = SyncError::OpTooLarge {
            op_id: "op-1".into(),
            bytes: 10,
            limit: Some(5),
            outcome: crate::engine::SyncOutcome {
                pushed: 0,
                pulled: 0,
                pull_ceiling_hit: false,
                budget_exhausted: false,
                pull_pages: 0,
                pull_empty_pages: 0,
                signatures_stripped: 0,
                unsigned_from_signers: 0,
            },
        };
        match SnapshotError::from(err) {
            SnapshotError::Transport(m) => assert!(m.contains("op-1"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    // ── Format 2: a snapshot with a BASE instead of the ops below its position (6j6v.0cfa) ──

    /// The one product a flow store folds. The platform's three are [`PLATFORM`]; a store with only
    /// the task reducer can carry only this one, and saying so is what [`export_based`] checks.
    const FLOW_ONLY: &[Product] = &[Product {
        of: "flow",
        domain: "task",
        keep_log: false,
    }];

    /// A writer that pulled `ops` of the relay and nothing of its own.
    fn caught_up(relay: &dyn Transport, site: i64) -> (Store, Watermarks) {
        let mut store = Store::open_in_memory(site);
        let mut marks = Watermarks::default();
        loop {
            let before = marks.pulled_through;
            engine::sync(&mut store, site, &stream(), &mut marks, relay, 500, &Unbounded).unwrap();
            if marks.pulled_through == before {
                return (store, marks);
            }
        }
    }

    /// A small stream on the relay double, written by one replica and pushed.
    fn written(relay: &Relay, items: usize) -> i64 {
        let mut w = Store::open_in_memory(11);
        let mut marks = Watermarks::default();
        for n in 0..items {
            w.create_item(&format!("wa.{n}"), "task", "t", "wa");
            w.set_field(&format!("wa.{n}"), "status", Some("open".into()), "wa");
        }
        pass(&mut w, 11, &mut marks, relay);
        marks.pulled_through
    }

    /// Unpack a snapshot's JSON, hand it to `tamper`, and pack it again — for the checks that must
    /// refuse a file whose halves disagree, which no honest writer produces.
    fn retamper(bytes: &[u8], tamper: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let json = unpack(bytes, MAX_UNPACKED_BYTES).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        tamper(&mut value);
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        serde_json::to_writer(&mut gz, &value).unwrap();
        gz.finish().unwrap()
    }

    /// A based snapshot carries the state and NOT the ops the state stands in for — and its header
    /// says exactly that, readable without unpacking the body.
    #[test]
    fn a_based_snapshot_carries_the_state_and_not_the_log_below_its_position() {
        let relay = Relay::new(1, 0);
        let p = written(&relay, 20);
        let (writer, marks) = caught_up(&relay, 12);
        let bytes =
            export_based(&writer, 12, &stream(), &marks, &relay, FLOW_ONLY).unwrap();
        let header = peek(&bytes).unwrap();
        assert_eq!(header.format, FORMAT_BASED);
        assert_eq!(header.pulled_through, p);
        assert_eq!(header.ops, 0, "every op was folded, so none is carried");
        let base = header.base.unwrap();
        assert_eq!(base.ops_through_p, writer.op_count() as usize);
        assert_eq!(base.residue, 0);
        assert!(base.kept_domains.is_empty());
        assert_eq!(base.clock, writer.clock());
        assert_eq!(header.revisions, BTreeMap::from([("task".to_string(), 0)]));
        // And a with-log snapshot of the same replica still carries all of it.
        let with_log = export(&writer, 12, &stream(), &marks, &relay).unwrap();
        assert_eq!(peek(&with_log).unwrap().ops, writer.op_count() as usize);
        assert!(bytes.len() < with_log.len());
    }

    /// A based snapshot plus the tail equals the whole history — held here on the smallest stream
    /// that has a tail at all. The acceptance-scale differential across three products lives in
    /// `nxs-fold-ddb`'s `based_differential`, which is where the generator and the server half are.
    #[test]
    fn a_based_snapshot_plus_the_tail_folds_to_exactly_the_whole_history() {
        let relay = Relay::new(2, 0);
        let p = written(&relay, 12);
        let (writer, marks) = caught_up(&relay, 12);
        let bytes =
            export_based(&writer, 12, &stream(), &marks, &relay, FLOW_ONLY).unwrap();
        // The stream grows past the snapshot.
        let mut later = Store::open_in_memory(13);
        let mut later_marks = Watermarks::default();
        pass(&mut later, 13, &mut later_marks, &relay);
        later.create_item("wb.1", "task", "after", "wb");
        later.set_field("wa.3", "status", Some("closed".into()), "wb");
        later.add_edge("wb.1", "wa.3", EdgeKind::Dep, "wb");
        pass(&mut later, 13, &mut later_marks, &relay);
        assert!(relay.len() as i64 > p);

        let mut based = Store::open_in_memory(14);
        let imported = import(&mut based, &bytes, &stream(), &relay).unwrap();
        assert_eq!(imported.anchor, Anchor::Confirmed);
        assert_eq!(imported.marks.pulled_through, p);
        let mut marks = imported.marks;
        pass(&mut based, 14, &mut marks, &relay);

        let (replayed, _) = caught_up(&relay, 15);
        assert_eq!(state(&based), state(&replayed));
    }

    /// A based replica's first write beats the state it imported (spec §3, L1) — the clock floor
    /// travels in the snapshot, and the local store has no ops to read it off.
    #[test]
    fn a_based_replica_mints_above_the_clock_the_snapshot_carried() {
        let relay = Relay::new(3, 0);
        written(&relay, 8);
        let (writer, marks) = caught_up(&relay, 12);
        let bytes =
            export_based(&writer, 12, &stream(), &marks, &relay, FLOW_ONLY).unwrap();
        let mut based = Store::open_in_memory(14);
        import(&mut based, &bytes, &stream(), &relay).unwrap();
        assert_eq!(based.op_count(), 0, "no log to read a clock off");
        let before: String = based
            .connection()
            .query_row("SELECT status FROM items WHERE id = 'wa.1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, "open", "the snapshot brought the register");
        based.set_field("wa.1", "status", Some("closed".into()), "me");
        let after: String = based
            .connection()
            .query_row("SELECT status FROM items WHERE id = 'wa.1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            after, "closed",
            "the local write won, so it was minted above the register the snapshot brought"
        );
        assert!(based.clock() >= writer.clock());
    }

    /// A position the relay does not confirm: a with-log snapshot pulls from the start instead, a
    /// based one cannot — it holds nothing below its position — so it refuses by name and the
    /// caller falls back.
    #[test]
    fn a_based_import_refuses_a_position_the_relay_does_not_confirm() {
        let relay = Relay::new(4, 0);
        written(&relay, 6);
        let (writer, marks) = caught_up(&relay, 12);
        let bytes =
            export_based(&writer, 12, &stream(), &marks, &relay, FLOW_ONLY).unwrap();

        // Another relay, holding another stream's ops at the same positions.
        let other = Relay::new(5, 0);
        written(&other, 6);
        let mut based = Store::open_in_memory(14);
        let err = import(&mut based, &bytes, &stream(), &other).unwrap_err();
        assert!(
            matches!(err, SnapshotError::AnchorMismatch { .. }),
            "{err}"
        );
        assert_eq!(based.op_count(), 0, "a refusal writes nothing");
        assert!(based.list_items().unwrap().is_empty());

        // The with-log snapshot of the same replica takes the other road, unchanged.
        let with_log = export(&writer, 12, &stream(), &marks, &relay).unwrap();
        let mut fallback = Store::open_in_memory(15);
        let imported = import(&mut fallback, &with_log, &stream(), &other).unwrap();
        assert_eq!(imported.anchor, Anchor::Mismatch);
        assert_eq!(imported.marks.pulled_through, 0);
    }

    /// A product whose reducer is not registered has no views in the image to carry, so the export
    /// refuses rather than writing a snapshot that leaves a product out.
    #[test]
    fn export_based_refuses_a_product_whose_reducer_is_not_registered() {
        let relay = Relay::new(6, 0);
        written(&relay, 4);
        let (writer, marks) = caught_up(&relay, 12);
        let err = export_based(&writer, 12, &stream(), &marks, &relay, PLATFORM).unwrap_err();
        assert!(
            matches!(&err, SnapshotError::NotFolded { domain } if domain == "message"),
            "{err}"
        );
    }

    /// Everything [`export`] refuses, [`export_based`] refuses too.
    #[test]
    fn export_based_refuses_unpushed_ops_and_a_position_this_relay_does_not_back() {
        let relay = Relay::new(7, 0);
        written(&relay, 4);
        let (mut writer, marks) = caught_up(&relay, 12);
        writer.create_item("mine.1", "task", "t", "me");
        let err = export_based(&writer, 12, &stream(), &marks, &relay, FLOW_ONLY).unwrap_err();
        assert!(matches!(err, SnapshotError::Unpushed { ops: 1 }), "{err}");

        let (clean, mut marks) = caught_up(&relay, 13);
        marks.pulled_through += 5; // a position this relay does not hold
        let err = export_based(&clean, 13, &stream(), &marks, &relay, FLOW_ONLY).unwrap_err();
        assert!(matches!(err, SnapshotError::Unbacked { .. }), "{err}");
    }

    /// A snapshot whose header and body disagree about its base is refused, not believed in
    /// whichever half a reader happens to look at.
    #[test]
    fn a_snapshot_whose_halves_disagree_about_its_base_is_refused() {
        let relay = Relay::new(8, 0);
        written(&relay, 6);
        let (writer, marks) = caught_up(&relay, 12);
        let good = export_based(&writer, 12, &stream(), &marks, &relay, FLOW_ONLY).unwrap();

        let tampered: Vec<(&str, Vec<u8>)> = vec![
            (
                "a header clock the base does not carry",
                retamper(&good, |v| v["base"]["clock"] = serde_json::json!(1)),
            ),
            (
                "a residue count the base does not carry",
                retamper(&good, |v| v["base"]["residue"] = serde_json::json!(3)),
            ),
            (
                "kept domains the base does not carry",
                retamper(&good, |v| {
                    v["base"]["kept_domains"] = serde_json::json!(["message"])
                }),
            ),
            (
                "a revision the views were not folded at",
                retamper(&good, |v| {
                    v["revisions"] = serde_json::json!({ "task": 9 })
                }),
            ),
            (
                "a base position its header does not claim",
                retamper(&good, |v| {
                    v["image"]["base"]["relay_position"] = serde_json::json!(2)
                }),
            ),
            (
                "format 2 with no base at all",
                retamper(&good, |v| {
                    v.as_object_mut().unwrap().remove("base");
                    v["image"].as_object_mut().unwrap().remove("base");
                }),
            ),
            (
                "a base under a format-1 header",
                retamper(&good, |v| v["format"] = serde_json::json!(1)),
            ),
        ];
        for (what, bytes) in tampered {
            let mut store = Store::open_in_memory(14);
            let err = import(&mut store, &bytes, &stream(), &relay).unwrap_err();
            assert!(
                matches!(err, SnapshotError::Corrupt(_)),
                "{what}: {err}"
            );
            assert_eq!(store.op_count(), 0, "{what}: a refusal writes nothing");
        }
        // The untampered one still loads, so the tampering is what the refusals are about.
        let mut store = Store::open_in_memory(15);
        import(&mut store, &good, &stream(), &relay).unwrap();
    }

    /// A format newer than this build reads is refused as NEWER, from the header alone — the base
    /// does not change that, and a reader must not try to interpret a shape it does not know.
    #[test]
    fn a_format_newer_than_this_build_is_refused_as_newer() {
        let relay = Relay::new(9, 0);
        written(&relay, 4);
        let (writer, marks) = caught_up(&relay, 12);
        let good = export_based(&writer, 12, &stream(), &marks, &relay, FLOW_ONLY).unwrap();
        let newer = retamper(&good, |v| v["format"] = serde_json::json!(FORMAT + 1));
        let mut store = Store::open_in_memory(14);
        assert!(matches!(
            import(&mut store, &newer, &stream(), &relay).unwrap_err(),
            SnapshotError::FormatTooNew { found, supported }
                if found == FORMAT + 1 && supported == FORMAT
        ));
        assert!(matches!(
            peek(&newer).unwrap_err(),
            SnapshotError::FormatTooNew { .. }
        ));
    }

    /// The digest over the dropped prefix is order-free (the two sides enumerate differently) and
    /// changes when the set does (a missing op is what it exists to catch).
    #[test]
    fn the_digest_is_order_free_and_notices_a_missing_op() {
        let relay = Relay::new(10, 0);
        written(&relay, 10);
        let (full, _) = caught_up(&relay, 12);
        let (count, whole) = digest(&full).unwrap();
        assert_eq!(count, full.op_count() as usize);

        // The same ops, taken in in another order: the same digest.
        let mut other = Store::open_in_memory(13);
        let mut ops = full.export();
        ops.reverse();
        other.apply(&ops);
        assert_eq!(digest(&other).unwrap(), (count, whole.clone()));

        // One op short: a different digest.
        let mut short = Store::open_in_memory(14);
        short.apply(&ops[1..]);
        let (fewer, missing_one) = digest(&short).unwrap();
        assert_eq!(fewer, count - 1);
        assert_ne!(missing_one, whole);
    }

    /// The envelope opened for a reader that folds the views elsewhere — with the same refusals as
    /// an import, so the two readers of one format cannot disagree about what a snapshot is.
    #[test]
    fn unpack_image_hands_back_the_header_and_the_image_it_describes() {
        let relay = Relay::new(11, 0);
        let p = written(&relay, 7);
        let (writer, marks) = caught_up(&relay, 12);
        let bytes = export_based(&writer, 12, &stream(), &marks, &relay, FLOW_ONLY).unwrap();
        let (header, image) = unpack_image(&bytes).unwrap();
        assert_eq!(header.pulled_through, p);
        assert!(image.based());
        assert_eq!(image.base.as_ref().unwrap().relay_position, p);
        use nxs_foundation::reducer::Reducer;
        assert_eq!(
            image.views.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            nexus_flow_core::task_reducer::TaskReducer
                .view_tables()
                .to_vec()
        );
        let broken = retamper(&bytes, |v| v["base"]["clock"] = serde_json::json!(7));
        assert!(matches!(
            unpack_image(&broken).unwrap_err(),
            SnapshotError::Corrupt(_)
        ));
    }

    /// The platform list is the one description of the platform, and chat is the one product that
    /// keeps its log (spec D9).
    #[test]
    fn the_platform_list_keeps_chats_log_and_drops_the_others() {
        assert_eq!(
            PLATFORM
                .iter()
                .map(|p| (p.of, p.domain, p.keep_log))
                .collect::<Vec<_>>(),
            vec![
                ("flow", "task", false),
                ("chat", "message", true),
                ("memory", "fact", false),
            ]
        );
    }
}
