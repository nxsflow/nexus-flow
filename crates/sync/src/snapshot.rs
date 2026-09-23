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
use std::io::Read;

/// The snapshot format this build writes, and the newest it reads.
pub const FORMAT: u32 = 1;

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
            SnapshotError::Image(e) => write!(f, "{e}"),
            SnapshotError::Storage(m) => write!(f, "snapshot storage: {m}"),
            SnapshotError::Transport(m) => write!(f, "snapshot transport: {m}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<ImageError> for SnapshotError {
    fn from(e: ImageError) -> SnapshotError {
        SnapshotError::Image(e)
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
    let image = store.image()?;
    let anchor = if marks.pulled_through > 0 {
        match op_at(transport, stream, marks.pulled_through)? {
            Some(id) if image.op_ids().contains(id.as_str()) => Some(id),
            _ => {
                return Err(SnapshotError::Unbacked {
                    position: marks.pulled_through,
                })
            }
        }
    } else {
        None
    };
    let envelope = Envelope {
        header: Header {
            format: FORMAT,
            stream_id: stream.as_str().to_string(),
            pulled_through: marks.pulled_through,
            anchor,
            engine: image.engine.clone(),
            ops: image.op_count(),
        },
        image,
    };
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    serde_json::to_writer(&mut gz, &envelope)
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
        (Some(anchor), p) if p > 0 && envelope.image.op_ids().contains(anchor.as_str()) => {}
        _ => {
            return Err(SnapshotError::Corrupt(
                "its anchor does not match its position and log".to_string(),
            ))
        }
    }
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
    /// (6j6v.4qjw), one that stores a push TWICE `twice_pct` percent of the time.
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

    /// A relay whose log was rebuilt — here, deduplicated, as fixing 6j6v.4qjw would do to a
    /// DynamoDB stream — renumbers every entry after the first duplicate. A snapshot's position
    /// then points somewhere else, and trusting it would skip ops. The anchor catches it, and the
    /// replica pulls from the start instead: slower, never short.
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
                format: FORMAT,
                stream_id: "s".into(),
                pulled_through: marks.pulled_through,
                anchor: header.anchor.clone(),
                engine: nxs_foundation::image::ENGINE_VERSION.into(),
                ops: w.op_count() as usize,
            }
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
}
