//! Shared protocol vocabulary: the stream identity, the server-assigned cursor, and
//! the HTTP request/response bodies. Both the relay and the client engine speak these,
//! so they live in the always-on part of the crate (no `engine` feature required).

use crate::wire::WireOp;
use serde::{Deserialize, Serialize};

/// The sync unit = one workspace, addressed by a durable id (spec §4.2). Opaque string;
/// the relay keys its op log by it and never interprets it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamId(pub String);

impl StreamId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The server-assigned, per-stream, monotone sequence used as the pull watermark
/// (spec §5). Total order, maps to one indexed column / sort key. `Cursor(0)` is the
/// "from the beginning" position; the first appended op gets `Cursor(1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Cursor(pub i64);

impl Cursor {
    pub const BEGINNING: Cursor = Cursor(0);
}

/// `POST /streams/{id}/ops` body: a batch of opaque ops to append in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRequest {
    pub ops: Vec<WireOp>,
}

/// `POST /streams/{id}/ops` response: how many ops the relay durably appended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushResponse {
    pub appended: usize,
}

/// `GET /streams/{id}/ops?since=&limit=` body: one page of ops plus the cursor to
/// resume from. `next == since` with an empty `ops` means the stream is exhausted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullResponse {
    pub ops: Vec<WireOp>,
    pub next: Cursor,
}

/// `POST /streams/{id}/register` body (bab/§6): a replica claims a display `prefix` for
/// the stream, identified by its durable, globally-unique `replica_uuid`. The uuid — NOT
/// the prefix — is the identity anchor that tells two replicas with a colliding prefix
/// apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub prefix: String,
    pub replica_uuid: String,
}

/// `POST /streams/{id}/register` response (bab/§6). Either the requested prefix is now
/// this replica's in the stream, or it collided with another replica and the relay
/// assigned a fresh `new_prefix` the replica must remap to (§8) before merging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RegisterOutcome {
    Registered,
    Reassigned { new_prefix: String },
}

/// `POST /streams/{id}/machines` body (6j6v.f0b5): a machine's background service says it is here.
///
/// Sent by the SERVICE after every pass that synced the stream — never by a manual
/// `nxs sync run`, because "online" is a promise that something on that machine is attending the
/// stream, and only the service keeps it. `interval_secs` is that promise: the service's cadence,
/// never shorter than its retry backoff, so one failed pass stays inside the reader's window. The
/// reader judges a sighting against it (see [`crate::presence`]), which is why it travels with the
/// announcement instead of being assumed.
///
/// What this puts on a relay that authenticates nobody is deliberately small: an id minted on the
/// machine (a ULID — random but for its first ten characters, which say when it was minted; never
/// a hardware id, never the update channel's id), a name its owner chose or can change, and the
/// cadence. No path, no replica identity, no user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineHello {
    pub machine_id: String,
    pub name: String,
    pub interval_secs: u64,
}

/// One machine as the relay last heard from it (6j6v.f0b5). `last_seen` is Unix seconds on the
/// RELAY's clock and `age_secs` is how long ago that was, measured by the relay when it answered —
/// so a reader whose own clock is off still judges correctly, because it only ever uses the age.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSeen {
    pub machine_id: String,
    pub name: String,
    pub last_seen: i64,
    pub age_secs: u64,
    pub interval_secs: u64,
}

/// `GET /streams/{id}/machines` response (6j6v.f0b5): the machines that announced themselves for
/// the stream within [`crate::presence::RETENTION_SECS`], most recently seen first, at most
/// [`crate::presence::MAX_LISTED_MACHINES`]. The relay judges nothing — whether one is online is
/// [`crate::presence::judge`]'s to say.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachinesResponse {
    pub machines: Vec<MachineSeen>,
    /// The relay knows more machines for the stream than it returned. Never true for an honest
    /// stream; it is what a flood of invented machine ids looks like from the outside, and a reader
    /// says so instead of presenting the list as complete. Defaulted, so a response without it reads
    /// as "not truncated".
    #[serde(default)]
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_hello_round_trips() {
        let hello = MachineHello {
            machine_id: "01j8machine000000000000000".into(),
            name: "Mac mini".into(),
            interval_secs: 300,
        };
        let json = serde_json::to_string(&hello).unwrap();
        assert_eq!(serde_json::from_str::<MachineHello>(&json).unwrap(), hello);
    }

    #[test]
    fn a_machines_response_round_trips() {
        let resp = MachinesResponse {
            machines: vec![MachineSeen {
                machine_id: "m1".into(),
                name: "MacBook".into(),
                last_seen: 1_790_000_000,
                age_secs: 12,
                interval_secs: 300,
            }],
            truncated: true,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(
            serde_json::from_str::<MachinesResponse>(&json).unwrap(),
            resp
        );
    }

    #[test]
    fn a_field_a_newer_relay_adds_to_a_sighting_is_ignored_not_refused() {
        // Both directions of the compatibility contract (6j6v.f0b5) rest on serde's default of
        // ignoring what it does not know: a newer relay may say more about a machine, and a client
        // built today must still read the list.
        let json = r#"{"machines":[{"machine_id":"m1","name":"MacBook","last_seen":5,
                       "age_secs":1,"interval_secs":300,"capabilities":["claude"]}]}"#;
        let resp: MachinesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.machines[0].machine_id, "m1");
        assert!(
            !resp.truncated,
            "a relay that says nothing about truncation did not truncate"
        );
    }

    #[test]
    fn register_request_round_trips() {
        let req = RegisterRequest {
            prefix: "aaaa".into(),
            replica_uuid: "01HRUUIDEXAMPLE0000000000".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(serde_json::from_str::<RegisterRequest>(&json).unwrap(), req);
    }

    #[test]
    fn register_outcome_variants_round_trip() {
        for outcome in [
            RegisterOutcome::Registered,
            RegisterOutcome::Reassigned {
                new_prefix: "kp3z".into(),
            },
        ] {
            let json = serde_json::to_string(&outcome).unwrap();
            assert_eq!(
                serde_json::from_str::<RegisterOutcome>(&json).unwrap(),
                outcome
            );
        }
    }

    #[test]
    fn cursor_orders_and_round_trips() {
        assert!(Cursor(1) > Cursor::BEGINNING);
        let json = serde_json::to_string(&Cursor(5)).unwrap();
        assert_eq!(serde_json::from_str::<Cursor>(&json).unwrap(), Cursor(5));
    }

    #[test]
    fn pull_response_round_trips() {
        let resp = PullResponse {
            ops: vec![],
            next: Cursor(3),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert_eq!(serde_json::from_str::<PullResponse>(&json).unwrap(), resp);
    }
}
