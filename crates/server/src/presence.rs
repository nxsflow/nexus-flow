//! [`PresenceStore`]: which machines the relay has heard from, per stream (nxf 6j6v.f0b5).
//!
//! The relay's third piece of state, beside the op log and the prefix claims — and unlike the op
//! log it is NOT history. One row per `(stream, machine)`, overwritten by every announcement:
//! presence is how things are now, so it lives beside the registry instead of in the log, where a
//! heartbeat per machine per pass would grow the history of every stream forever. Nothing here is
//! ever synced, folded or replayed.
//!
//! Like the other stores the trait is SQL-free. It is an OPTIONAL capability of a
//! [`PrefixRegistry`](crate::registry::PrefixRegistry)
//! ([`PrefixRegistry::presence`](crate::registry::PrefixRegistry::presence)), not a requirement: all three in-repo
//! backends implement it on the struct that already keeps the prefix claims, in the same database
//! — SQLite and Postgres add a `machine_presence` table, DynamoDB adds a third item kind
//! (`sk = machine#<id>`) to the registry table it already has, so a deployment provisions nothing
//! new — while a registry an embedder wrote before presence existed still compiles, and the relay
//! then answers the presence route exactly like a relay that predates it.
//!
//! **Bounded, because nobody is authenticated** (review of PR #485). Every row expires
//! [`RETENTION_SECS`](nxs_sync::presence::RETENTION_SECS) after its machine was last heard from: it is no longer listed, and SQLite and
//! Postgres delete a stream's expired rows whenever a machine announces itself for it (DynamoDB
//! stamps an `expires_at` a deployment MAY hand to its TTL, and filters on read either way). A read
//! returns at most [`MAX_LISTED_MACHINES`](nxs_sync::presence::MAX_LISTED_MACHINES) and says when
//! there were more. None of that stops a
//! flood from inside the window — only relay authentication (6j6v.6aza) can — but it makes the
//! cost bounded and the damage visible.
//!
//! The relay judges nothing: it stores what a machine announced plus its own clock's time, and
//! [`sighting`] turns a row into the age a reader judges by (`nxs_sync::presence`). What it
//! exposes, and why that is acceptable on a relay without authentication, is written down once, at
//! [`nxs_sync::presence`].

use nxs_sync::protocol::{MachineHello, MachineSeen, StreamId};

use crate::store::StoreResult;

/// One stored announcement: what the machine said, and when (Unix seconds, the relay's clock).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineRecord {
    pub machine_id: String,
    pub name: String,
    pub last_seen: i64,
    pub interval_secs: u64,
}

/// One read of a stream's machines: the most recently seen first, and whether there were more than
/// the read returned.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MachinesPage {
    pub records: Vec<MachineRecord>,
    pub truncated: bool,
}

/// The relay's presence board. Two operations, both per stream, both handed the relay's clock by
/// the caller so every backend is provable without waiting.
pub trait PresenceStore {
    /// Record that `hello`'s machine was seen at `now`: insert its row, or overwrite name, cadence
    /// and time if it has one. A backend that can do so cheaply also forgets the stream's rows last
    /// seen before `forget_before` in the same call.
    fn announce(
        &self,
        stream: &StreamId,
        hello: &MachineHello,
        now: i64,
        forget_before: i64,
    ) -> StoreResult<()>;

    /// The stream's machines last seen at or after `seen_since`, most recently seen first (ties by
    /// id in byte order, so the order is total and the backends agree value for value), at most
    /// `limit`, with `truncated` set when there were more.
    fn machines(
        &self,
        stream: &StreamId,
        seen_since: i64,
        limit: usize,
    ) -> StoreResult<MachinesPage>;
}

/// A stored row as a reader receives it: with its age at `now`. A relay clock that stepped
/// backwards past a stored time reads as age zero rather than as a negative number no reader can
/// judge.
pub fn sighting(record: &MachineRecord, now: i64) -> MachineSeen {
    MachineSeen {
        machine_id: record.machine_id.clone(),
        name: record.name.clone(),
        last_seen: record.last_seen,
        age_secs: now.saturating_sub(record.last_seen).max(0) as u64,
        interval_secs: record.interval_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sighting_carries_the_age_at_the_moment_of_asking() {
        let record = MachineRecord {
            machine_id: "m1".into(),
            name: "MacBook".into(),
            last_seen: 1_000,
            interval_secs: 300,
        };
        let seen = sighting(&record, 1_042);
        assert_eq!(seen.age_secs, 42);
        assert_eq!(seen.last_seen, 1_000);
        assert_eq!(sighting(&record, 900).age_secs, 0, "a clock stepping back");
    }
}
