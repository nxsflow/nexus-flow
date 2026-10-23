//! The board-fold purge's two copies of the relay's rules, held equal to the relay (6j6v.dbmd
//! design MF-1, f6yb MF-13): `purge::validate_purge_id` refuses exactly the ids
//! `nxs_server::purge::validate_purge_id` refuses, with the same rule; `purge::LogKey` parses exactly
//! the keys `nxs_server::log_id::LogKey` parses, refuses the rest with the same rule, and gives every
//! id the same token. The copies stay copies — the library never depends on the relay — and this is
//! what keeps them from drifting.

use nxs_fold_ddb::purge::{self, Kind, LogKey, PurgeError};
use nxs_server::log_id as relay_log;
use nxs_sync::protocol::StreamId;

fn ids() -> Vec<String> {
    let mut ids: Vec<String> = [
        "",
        "#",
        "#due",
        "#sweep",
        "a#b",
        "a/b",
        "/",
        "n-00ff00ff00ff00ff00ff00ff00ff00ff-1",
        "u-00ff00ff00ff00ff00ff00ff00ff00ff",
        "6j6v",
        "-odd",
        " lead",
        "trail ",
        "tab\tin",
        "nl\n",
        "del\u{7f}",
        "c1\u{85}",
        "é-ü",
        "日本",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for n in [999, 1_000, 1_001, 4_096] {
        ids.push("x".repeat(n));
    }
    // A multi-byte id right at the byte limit and one past it.
    ids.push("é".repeat(500));
    ids.push(format!("{}x", "é".repeat(500)));
    ids
}

#[test]
fn validate_purge_id_refuses_exactly_what_the_relay_refuses_with_the_same_rule() {
    for id in ids() {
        let ours = purge::validate_purge_id(&id);
        let relay = nxs_server::purge::validate_purge_id(&StreamId(id.clone()));
        let rule = |r: Option<&'static str>| r;
        let ours_rule = match &ours {
            Ok(()) => None,
            Err(PurgeError::InvalidStreamId { rule }) => Some(*rule),
            Err(other) => panic!("{id:?}: {other:?}"),
        };
        let relay_rule = match &relay {
            Ok(()) => None,
            Err(nxs_server::purge::PurgeError::InvalidStreamId { rule }) => Some(*rule),
            Err(other) => panic!("{id:?}: {other:?}"),
        };
        assert_eq!(rule(ours_rule), rule(relay_rule), "{id:?}");
    }
    assert_eq!(
        purge::MAX_PURGE_ID_BYTES,
        nxs_server::purge::MAX_PURGE_ID_BYTES
    );
}

const C9_KEY: &str = "t1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

fn keys() -> Vec<String> {
    let good = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    let mut keys = vec![
        C9_KEY.to_string(),
        format!("20261101abcdef99:{good}"),
        format!("t1:{}", &good[..42]),
        format!("t1:{good}A"),
        format!("t1:{}=", &good[..42]),
        format!("t1:{}+", &good[..42]),
        format!("t1:{}/", &good[..42]),
        format!(":{good}"),
        format!("T1:{good}"),
        format!("{}:{good}", "a".repeat(17)),
        format!("t1:{good}:x"),
        format!("t1:x:{good}"),
        good.to_string(),
        String::new(),
        format!("t1:{good}\n"),
        format!(" t1:{good}"),
        format!("t1:{}9", &good[..42]),
    ];
    // Every last character: the decode rule for the 2 trailing bits.
    for c in "AQgw9_-".chars() {
        keys.push(format!("t1:{}{c}", &good[..42]));
    }
    keys
}

#[test]
fn the_log_key_copy_parses_and_refuses_exactly_as_the_relay_does() {
    for value in keys() {
        let ours = LogKey::parse(&value).map_err(|e| format!("{e:?}"));
        let relay = relay_log::LogKey::parse(&value).map_err(|e| format!("{e:?}"));
        assert_eq!(ours.is_ok(), relay.is_ok(), "{value:?}");
        if let (Err(a), Err(b)) = (&ours, &relay) {
            assert_eq!(a, b, "{value:?}: the same rule");
            let ours_message = LogKey::parse(&value).unwrap_err().to_string();
            let relay_message = relay_log::LogKey::parse(&value).unwrap_err().to_string();
            assert_eq!(ours_message, relay_message);
        }
    }
}

#[test]
fn the_log_key_copy_gives_every_id_the_relays_token() {
    for value in [
        C9_KEY.to_string(),
        "20261101abcdef99:_-_-AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxw".to_string(),
    ] {
        let ours = LogKey::parse(&value).unwrap();
        let relay = relay_log::LogKey::parse(&value).unwrap();
        for (kind, relay_kind) in [
            (Kind::Sid, relay_log::Kind::Sid),
            (Kind::Op, relay_log::Kind::Op),
            (Kind::Rid, relay_log::Kind::Rid),
            (Kind::Mid, relay_log::Kind::Mid),
            (Kind::Uid, relay_log::Kind::Uid),
        ] {
            for id in ids() {
                assert_eq!(
                    ours.token(kind, &id),
                    relay.token(relay_kind, &id),
                    "{id:?}"
                );
            }
        }
    }
}

#[test]
fn the_five_c9_vectors_hold_for_the_copy() {
    let key = LogKey::parse(C9_KEY).unwrap();
    for (kind, id, token) in [
        (Kind::Sid, "6j6v", "sid=h1:t1:d5bfa71653806fdbcfb1052f"),
        (
            Kind::Op,
            "01J0DEMOOP00000000000000A",
            "op=h1:t1:ab7c22dbcefc68c0fc2f34d4",
        ),
        (
            Kind::Sid,
            "01J0DEMOOP00000000000000A",
            "sid=h1:t1:332d1e2cee79a5b45589a4e6",
        ),
        (
            Kind::Rid,
            "3f1c2a9e-0000-4000-8000-000000000001",
            "rid=h1:t1:ca8c97c1c46d0dc5c92131fc",
        ),
        (
            Kind::Mid,
            "m-laptop-1",
            "mid=h1:t1:232916555290733ba960135f",
        ),
    ] {
        assert_eq!(key.token(kind, id), token);
    }
}
