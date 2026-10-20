//! `nxf-relay purge` on DynamoDB Local (6j6v.f6yb): every DoD line DynamoDB can show, each run as
//! a real `nxf-relay purge` process against tables the server's own DynamoDB stores wrote. Keyed
//! on `NXF_RELAY_DDB_ENDPOINT` like the other DynamoDB suite (CI: a hard failure when missing).
//!
//! Seeds are small here (tens of ops, page 7, so the plan and the batches still cross many
//! boundaries) so that every case can have a fresh pair of tables; the 5,000-op run with real
//! workspaces is `purge_acceptance.rs`.
#![cfg(feature = "dynamodb")]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use aws_sdk_dynamodb::Client;
use serde_json::json;

use nxs_server::presence::PresenceStore;
use nxs_server::registry::PrefixRegistry;
use nxs_server::store::OpStore;
use nxs_server::store_ddb::{DynamoDbOpStore, DynamoDbPrefixRegistry};
use nxs_sync::protocol::{MachineHello, StreamId};
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

#[path = "support/purge.rs"]
mod support;
use support::{purge, Purged, C9_KEY};

#[path = "support/ddb.rs"]
mod ddb;

const H: &str = "00ff00ff00ff00ff00ff00ff00ff00ff";

fn target() -> String {
    format!("n-{H}-1")
}
fn neighbour() -> String {
    format!("n-{H}-10")
}
fn prefix_neighbour() -> String {
    format!("n-{H}-1-x")
}

fn op(op_id: &str, value: &str, wall_clock: &str) -> WireOp {
    WireOp {
        envelope_version: ENVELOPE_VERSION,
        op_id: op_id.into(),
        lamport: 1,
        site: 1,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "ab12.0001".into(),
        field: "title".into(),
        op_type: "set".into(),
        value: Some(value.into()),
        author: "seed".into(),
        wall_clock: wall_clock.into(),
        extra: Default::default(),
    }
}

/// One pair of relay tables on DynamoDB Local.
struct Fixture {
    endpoint: String,
    client: Client,
    ops: String,
    registry: String,
    /// Where the purge processes are pointed (the endpoint, or a recording proxy before it).
    purge_endpoint: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        ddb::delete_table(&self.client, &self.ops);
        ddb::delete_table(&self.client, &self.registry);
    }
}

impl Fixture {
    fn new(test: &str) -> Option<Fixture> {
        let endpoint = ddb::endpoint(test)?;
        let client = ddb::client(&endpoint);
        let ops = ddb::ops_table(&client);
        let registry = ddb::registry_table(&client);
        Some(Fixture {
            purge_endpoint: endpoint.clone(),
            endpoint,
            client,
            ops,
            registry,
        })
    }

    /// Target and neighbours with `ops` ops each, 2 prefixes and `machines` machines, plus the
    /// raw-seeded `#due` items of target and neighbour and the global `#sweep/lease`.
    fn seeded(test: &str, ops: usize, machines: usize) -> Option<Fixture> {
        let f = Fixture::new(test)?;
        for s in [target(), neighbour(), prefix_neighbour()] {
            f.seed(&s, ops, machines);
        }
        ddb::put_raw(
            &f.client,
            &f.registry,
            &[("stream_id", "#due"), ("sk", &target()), ("due_at", "#N:1")],
        );
        ddb::put_raw(
            &f.client,
            &f.registry,
            &[
                ("stream_id", "#due"),
                ("sk", &neighbour()),
                ("due_at", "#N:2"),
            ],
        );
        ddb::put_raw(
            &f.client,
            &f.registry,
            &[
                ("stream_id", "#sweep"),
                ("sk", "lease"),
                ("holder", "the-sweep"),
                ("expires_at", "#N:4102444800"),
            ],
        );
        Some(f)
    }

    fn op_store(&self) -> DynamoDbOpStore {
        DynamoDbOpStore::new(
            self.ops.clone(),
            Some(self.endpoint.clone()),
            Some(ddb::REGION.into()),
        )
    }

    fn registry_store(&self) -> DynamoDbPrefixRegistry {
        DynamoDbPrefixRegistry::new(
            self.registry.clone(),
            Some(self.endpoint.clone()),
            Some(ddb::REGION.into()),
        )
    }

    fn seed(&self, stream: &str, ops: usize, machines: usize) {
        let s = StreamId(stream.to_string());
        let list: Vec<WireOp> = (1..=ops)
            .map(|i| {
                op(
                    &format!("01JDDB{stream}{i:05}"),
                    &format!("MARKERddb<{stream}>{i}"),
                    &format!("2026-10-18T20:{:02}:{:02}Z", i / 60 % 60, i % 60),
                )
            })
            .collect();
        self.op_store()
            .append_batch(&s, &list, Instant::now() + Duration::from_secs(600))
            .unwrap();
        let reg = self.registry_store();
        for (i, prefix) in ["ab12", "cd34"].iter().enumerate() {
            reg.register(&s, prefix, &format!("uuid-{stream}-{i}"))
                .unwrap();
        }
        for i in 0..machines {
            reg.announce(
                &s,
                &MachineHello {
                    machine_id: format!("m-{stream}-{i}"),
                    name: format!("laptop {i}"),
                    interval_secs: 300,
                },
                4_000_000_000 + i as i64,
                3_000_000_000,
            )
            .unwrap();
        }
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("NXF_RELAY_BACKEND", "dynamodb".into()),
            ("NXF_RELAY_DDB_TABLE", self.ops.clone()),
            ("NXF_RELAY_DDB_REGISTRY_TABLE", self.registry.clone()),
            ("NXF_RELAY_DDB_REGION", ddb::REGION.into()),
            ("NXF_RELAY_DDB_ENDPOINT", self.purge_endpoint.clone()),
            ("AWS_ACCESS_KEY_ID", "test".into()),
            ("AWS_SECRET_ACCESS_KEY", "test".into()),
            ("NXS_LOG_HMAC_KEY", C9_KEY.into()),
            ("NXF_RELAY_TEST_PURGE_PAGE", "7".into()),
        ]
    }

    fn run(&self, args: &[&str], extra: &[(&str, &str)]) -> Purged {
        let mut env = self.env();
        for (k, v) in extra {
            env.retain(|(name, _)| name != k);
            env.push((k, v.to_string()));
        }
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        purge(args, &env)
    }

    fn dry(&self, stream: &str) -> Purged {
        self.run(&[stream, "--dry-run"], &[])
    }

    fn confirm(&self, stream: &str) -> Purged {
        self.run(&[stream, "--dry-run", "--expect-empty"], &[])
    }

    fn yes(&self, stream: &str, digest: &str) -> Purged {
        self.run(&[stream, "--yes", "--plan", digest], &[])
    }

    /// Both tables in full.
    fn dump(&self) -> Vec<String> {
        let mut rows: Vec<String> = ddb::dump(&self.client, &self.ops)
            .into_iter()
            .map(|r| format!("ops|{r}"))
            .collect();
        rows.extend(
            ddb::dump(&self.client, &self.registry)
                .into_iter()
                .map(|r| format!("reg|{r}")),
        );
        rows
    }

    /// Both tables without anything of the target: its two partitions, its `#due` item and its
    /// stream lease.
    fn outside(&self) -> Vec<String> {
        let t = target();
        self.dump()
            .into_iter()
            .filter(|row| {
                !row.contains(&format!("\"stream_id\": \"S:{t}\""))
                    && !(row.contains("\"stream_id\": \"S:#due\"")
                        && row.contains(&format!("\"sk\": \"S:{t}\"")))
                    && !row.contains(&format!("\"sk\": \"S:stream#{t}\""))
            })
            .collect()
    }

    /// Items of the target anywhere: its partitions, its `#due`, its lease.
    fn target_items(&self) -> usize {
        self.dump().len() - self.outside().len()
    }

    fn lease_item(&self) -> Option<String> {
        let t = target();
        self.dump()
            .into_iter()
            .find(|r| r.contains(&format!("\"sk\": \"S:stream#{t}\"")))
    }
}

macro_rules! fixture {
    ($test:expr, $ops:expr, $machines:expr) => {
        match Fixture::seeded($test, $ops, $machines) {
            Some(f) => f,
            None => return,
        }
    };
}

/// What a fresh dry run, a bound --yes and a confirm must give after any interruption.
fn finish(f: &Fixture, outside: &[String], what: &str, extra: &[(&str, &str)]) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let dry = f.run(&[&target(), "--dry-run"], extra);
        dry.expect(0, &format!("{what}: fresh dry run"));
        let yes = f.run(&[&target(), "--yes", "--plan", &dry.digest()], extra);
        // A lease left by a crashed run is waited out (exit 2 while it is unexpired).
        if yes.code == Some(2) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
        yes.expect(0, &format!("{what}: bound --yes"));
        break;
    }
    f.run(&[&target(), "--dry-run", "--expect-empty"], extra)
        .expect(0, &format!("{what}: confirm"));
    assert_eq!(f.target_items(), 0, "{what}");
    assert_eq!(
        f.outside(),
        outside,
        "{what}: everything outside the target"
    );
}

// ----- the cycle ---------------------------------------------------------------------------------

#[test]
fn a_dry_run_a_bound_yes_and_a_confirm_remove_the_target_and_only_it() {
    let f = fixture!("purge_ddb_cycle", 40, 30);
    let all = f.dump();
    let outside = f.outside();
    let dry = f.dry(&target());
    dry.expect(0, "dry run");
    assert_eq!(f.dump(), all, "a dry run deletes nothing");
    assert_eq!(dry.found("ops"), 40);
    assert_eq!(dry.found("claims"), 40);
    assert_eq!(dry.found("counter"), 1);
    assert_eq!(dry.found("registry"), 2 + 2 + 30);
    assert_eq!(dry.found("due"), 1);
    assert_eq!(
        dry.kind("registry")["by_prefix"],
        json!({"machine#": 30, "prefix#": 2, "uuid#": 2})
    );
    assert_eq!(dry.kind("ops")["seq_ranges"], json!([[1, 40]]));
    assert_eq!(dry.kind("ops")["max_seq"], 40);
    assert_eq!(dry.kind("ops")["value_bytes"], serde_json::Value::Null);
    assert_eq!(dry.report["lease"]["found"], false);
    assert_eq!(
        dry.report["present"],
        json!({"counter": true, "registry": true})
    );
    let yes = f.yes(&target(), &dry.digest());
    yes.expect(0, "--yes");
    for kind in ["ops", "claims", "counter", "registry", "due"] {
        assert_eq!(yes.deleted(kind), dry.found(kind), "{kind}");
    }
    assert_eq!(yes.report["physical"], "provider");
    assert_eq!(yes.report["lease"]["taken"], true);
    assert_eq!(yes.report["lease"]["released"], true);
    assert_eq!(f.target_items(), 0, "no item of the target anywhere");
    assert_eq!(
        f.outside(),
        outside,
        "neighbours, #due of n-h-10 and #sweep/lease identical"
    );
    f.confirm(&target()).expect(0, "confirm");
}

#[test]
fn with_200kb_values_plan_pages_come_back_short_and_the_purge_still_completes() {
    // MF-12: about 200 ops of 200 KB (not 5,000), so a Query page stops at 1 MB before `Limit`.
    let Some(f) = Fixture::new("purge_ddb_short_pages") else {
        return;
    };
    let big = "z".repeat(200 * 1024);
    let s = StreamId(target());
    let list: Vec<WireOp> = (1..=200)
        .map(|i| op(&format!("big-{i}"), &big, ""))
        .collect();
    f.op_store()
        .append_batch(&s, &list, Instant::now() + Duration::from_secs(900))
        .unwrap();
    f.seed(&neighbour(), 3, 1);
    let outside = f.outside();
    let dry = f.run(
        &[&target(), "--dry-run"],
        &[("NXF_RELAY_TEST_PURGE_PAGE", "1000")],
    );
    dry.expect(0, "dry run");
    assert_eq!(dry.found("ops"), 200);
    let yes = f.run(
        &[&target(), "--yes", "--plan", &dry.digest()],
        &[("NXF_RELAY_TEST_PURGE_PAGE", "1000")],
    );
    yes.expect(0, "--yes");
    assert_eq!(yes.deleted("ops"), 200);
    f.confirm(&target()).expect(0, "confirm");
    assert_eq!(f.outside(), outside);
}

// ----- the digest --------------------------------------------------------------------------------

#[test]
fn the_digest_is_stable_changes_with_a_push_and_ignores_leases_heartbeats_and_expired_items() {
    let f = fixture!("purge_ddb_digest", 9, 3);
    // An item already past its TTL: announced "long ago".
    f.registry_store()
        .announce(
            &StreamId(target()),
            &MachineHello {
                machine_id: "m-expired".into(),
                name: "gone".into(),
                interval_secs: 300,
            },
            1_000,
            900,
        )
        .unwrap();
    let first = f.dry(&target());
    let d = first.digest();
    assert_eq!(first.kind("registry")["expired"], 1);
    assert_eq!(f.dry(&target()).digest(), d, "stable");
    // A lease item appears and goes.
    ddb::put_raw(
        &f.client,
        &f.registry,
        &[
            ("stream_id", "#sweep"),
            ("sk", &format!("stream#{}", target())),
            ("holder", "x"),
            ("expires_at", "#N:1"),
        ],
    );
    assert_eq!(f.dry(&target()).digest(), d, "a lease is not in the digest");
    ddb::delete_raw(
        &f.client,
        &f.registry,
        "#sweep",
        &format!("stream#{}", target()),
    );
    // A heartbeat overwrites a machine's last_seen.
    f.registry_store()
        .announce(
            &StreamId(target()),
            &MachineHello {
                machine_id: format!("m-{}-0", target()),
                name: "renamed".into(),
                interval_secs: 60,
            },
            4_100_000_000,
            3_000_000_000,
        )
        .unwrap();
    assert_eq!(
        f.dry(&target()).digest(),
        d,
        "last_seen is not in the digest"
    );
    // DynamoDB's TTL removes the expired item between two dry runs.
    ddb::delete_raw(&f.client, &f.registry, &target(), "machine#m-expired");
    assert_eq!(
        f.dry(&target()).digest(),
        d,
        "an expired item going away changes nothing"
    );
    // Put it back: --yes still deletes it and counts it.
    f.registry_store()
        .announce(
            &StreamId(target()),
            &MachineHello {
                machine_id: "m-expired".into(),
                name: "gone".into(),
                interval_secs: 300,
            },
            1_000,
            900,
        )
        .unwrap();
    let yes = f.yes(&target(), &d);
    yes.expect(0, "--yes with an expired item");
    assert_eq!(yes.kind("registry")["expired"], 1);
    assert_eq!(f.target_items(), 0);
    // And one more op changes the digest.
    f.seed(&target(), 1, 0);
    let again = f.dry(&target()).digest();
    f.op_store()
        .append(&StreamId(target()), &op("pushed", "p", ""))
        .unwrap();
    assert_ne!(f.dry(&target()).digest(), again);
}

#[test]
fn a_stale_plan_and_another_streams_digest_are_exit_4_with_nothing_deleted() {
    let f = fixture!("purge_ddb_stale", 8, 2);
    let dry = f.dry(&target());
    let n = f.dry(&neighbour()).digest();
    let before = f.dump();
    f.yes(&target(), &n).expect(4, "the neighbour's digest");
    assert_eq!(f.dump(), before);
    f.op_store()
        .append(&StreamId(target()), &op("late", "late", ""))
        .unwrap();
    let before = f.dump();
    f.yes(&target(), &dry.digest())
        .expect(4, "a push since the dry run");
    assert_eq!(f.dump(), before, "nothing deleted, and the lease released");
}

#[test]
fn a_confirm_finds_anything_left_a_lease_included_and_passes_on_a_never_existing_stream() {
    let f = fixture!("purge_ddb_confirm", 3, 1);
    f.confirm(&target()).expect(3, "the stream is there");
    let never = f.confirm(&format!("n-{H}-404"));
    never.expect(0, "never existed");
    ddb::put_raw(
        &f.client,
        &f.registry,
        &[
            ("stream_id", "#sweep"),
            ("sk", &format!("stream#n-{H}-404")),
            ("holder", "x"),
            ("expires_at", "#N:1"),
        ],
    );
    f.confirm(&format!("n-{H}-404"))
        .expect(3, "a lease item of the target is something left");
}

// ----- the store ---------------------------------------------------------------------------------

#[test]
fn the_identity_gate_refuses_every_wrong_pair_after_describe_and_before_any_query() {
    let f = fixture!("purge_ddb_identity", 2, 0);
    // A board-fold-shaped table (pk/sk + GSIs), a right-keyed table with a GSI, a missing table.
    let fold = ddb::unique("nxf_purge_fold");
    rt_fold_table(&f.client, &fold);
    let gsi = ddb::unique("nxf_purge_gsi");
    ddb::create(
        &f.client,
        &gsi,
        "stream_id",
        "sk",
        aws_sdk_dynamodb::types::ScalarAttributeType::S,
        true,
    );
    let proxy = ddb::Proxy::start(&f.endpoint);
    let cases: [(String, String, &str); 5] = [
        (f.registry.clone(), f.ops.clone(), "swapped"),
        (f.ops.clone(), f.ops.clone(), "the same table twice"),
        (
            f.ops.clone(),
            fold.clone(),
            "the board-fold table as the registry",
        ),
        (f.ops.clone(), gsi.clone(), "a GSI"),
        (
            f.ops.clone(),
            "nxf_purge_missing_table".into(),
            "a missing table",
        ),
    ];
    for (ops, registry, what) in &cases {
        for args in [
            vec![target(), "--dry-run".into()],
            vec![target(), "--dry-run".into(), "--expect-empty".into()],
            vec![target(), "--yes".into(), "--plan".into(), "00".repeat(32)],
        ] {
            proxy.clear();
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = f.run(
                &args,
                &[
                    ("NXF_RELAY_DDB_TABLE", ops),
                    ("NXF_RELAY_DDB_REGISTRY_TABLE", registry),
                    ("NXF_RELAY_DDB_ENDPOINT", &proxy.url),
                ],
            );
            out.expect(78, what);
            assert_eq!(out.report["error"]["kind"], "NotARelayStore", "{what}");
            let ops_seen = proxy.ops();
            assert!(
                ops_seen.iter().all(|o| o == "DescribeTable"),
                "{what}: only DescribeTable, saw {ops_seen:?}"
            );
        }
    }
    ddb::delete_table(&f.client, &fold);
    ddb::delete_table(&f.client, &gsi);
}

/// The board-fold table, as `nxs_fold_ddb::ddb::create_table` makes it: `pk`/`sk` strings and
/// global secondary indexes.
fn rt_fold_table(c: &Client, table: &str) {
    ddb::rt().block_on(async {
        nxs_fold_ddb::ddb::create_table(c, table)
            .send()
            .await
            .expect("create the board-fold table");
    });
    ddb::wait_active(c, table);
}

#[test]
fn a_recreated_table_is_another_store() {
    // MF-8, answered by CI: DynamoDB Local returns TableArn and CreationDateTime but no TableId,
    // so the TableId rules are tested on constructed descriptions (purge::ddb unit tests) and a
    // re-created table is told apart here by its creation time alone. MF-12: real AWS reports
    // CreationDateTime in seconds, so the re-create waits more than one.
    let f = fixture!("purge_ddb_recreate", 2, 0);
    let first = f.dry(&target());
    let identity = first.report["store"]["identity"].clone();
    assert!(identity["ops_table_arn"].is_string(), "{identity}");
    assert!(
        identity["ops_created"].is_string(),
        "CreationDateTime: {identity}"
    );
    eprintln!(
        "DynamoDB Local TableId: {} (null means the TableId rules rest on the unit tests)",
        identity["ops_table_id"]
    );
    ddb::delete_table(&f.client, &f.ops);
    std::thread::sleep(Duration::from_millis(1_500));
    ddb::create(
        &f.client,
        &f.ops,
        "stream_id",
        "seq",
        aws_sdk_dynamodb::types::ScalarAttributeType::N,
        false,
    );
    f.seed(&neighbour(), 1, 0);
    let again = f.dry(&target());
    again.expect(0, "after the re-create");
    assert_ne!(
        again.report["store"]["identity"]["ops_created"],
        identity["ops_created"]
    );
    assert_ne!(again.identity_digest(), first.identity_digest());
}

#[test]
fn a_foreign_table_with_the_registry_keys_is_refused_with_zzz_items_and_untouched_without() {
    let f = fixture!("purge_ddb_foreign", 3, 0);
    let foreign = ddb::registry_table(&f.client);
    ddb::put_raw(
        &f.client,
        &foreign,
        &[("stream_id", &target()), ("sk", "zzz#1"), ("v", "kept")],
    );
    ddb::put_raw(
        &f.client,
        &foreign,
        &[("stream_id", "other"), ("sk", "zzz#2"), ("v", "kept")],
    );
    let before_ops = ddb::dump(&f.client, &f.ops);
    let before_foreign = ddb::dump(&f.client, &foreign);
    for args in [
        vec![target(), "--dry-run".into()],
        vec![target(), "--dry-run".into(), "--expect-empty".into()],
        vec![target(), "--yes".into(), "--plan".into(), "00".repeat(32)],
    ] {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = f.run(&args, &[("NXF_RELAY_DDB_REGISTRY_TABLE", &foreign)]);
        out.expect(78, "zzz# in the target's partition");
        assert!(out.report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("zzz#"));
    }
    assert_eq!(ddb::dump(&f.client, &f.ops), before_ops);
    assert_eq!(ddb::dump(&f.client, &foreign), before_foreign);
    // Nothing of the target in it: the run proceeds and the foreign table stays identical.
    ddb::delete_raw(&f.client, &foreign, &target(), "zzz#1");
    let before_foreign = ddb::dump(&f.client, &foreign);
    let env = [("NXF_RELAY_DDB_REGISTRY_TABLE", foreign.as_str())];
    let dry = f.run(&[&target(), "--dry-run"], &env);
    dry.expect(0, "dry run against the foreign table");
    f.run(&[&target(), "--yes", "--plan", &dry.digest()], &env)
        .expect(0, "--yes against the foreign table");
    assert_eq!(ddb::dump(&f.client, &foreign), before_foreign);
    ddb::delete_table(&f.client, &foreign);
}

#[test]
fn an_unknown_kind_in_the_targets_partition_refuses_every_mode() {
    let f = fixture!("purge_ddb_unknown", 3, 1);
    ddb::put_raw(
        &f.client,
        &f.registry,
        &[("stream_id", &target()), ("sk", "zzz#1")],
    );
    let before = f.dump();
    for args in [
        vec![target(), "--dry-run".into()],
        vec![target(), "--dry-run".into(), "--expect-empty".into()],
        vec![target(), "--yes".into(), "--plan".into(), "00".repeat(32)],
    ] {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = f.run(&args, &[]);
        out.expect(78, "zzz#1");
        assert!(out.report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("zzz#"));
    }
    assert_eq!(f.dump(), before, "nothing deleted, no lease left");
}

#[test]
fn every_required_variable_is_named_when_missing() {
    let Some(f) = Fixture::new("purge_ddb_env") else {
        return;
    };
    for missing in [
        "NXF_RELAY_BACKEND",
        "NXF_RELAY_DDB_TABLE",
        "NXF_RELAY_DDB_REGISTRY_TABLE",
        "NXF_RELAY_DDB_REGION",
    ] {
        let mut env = f.env();
        env.retain(|(k, _)| *k != missing);
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let out = purge(&[&target(), "--dry-run"], &env);
        out.expect(78, missing);
        assert!(out.report["error"]["message"]
            .as_str()
            .unwrap()
            .contains(missing));
    }
}

#[test]
fn fresh_tables_need_the_pin_and_the_last_streams_confirm_passes_with_it() {
    let Some(f) = Fixture::new("purge_ddb_empty") else {
        return;
    };
    let dry = f.dry(&target());
    dry.expect(78, "fresh tables");
    assert_eq!(dry.report["error"]["kind"], "EmptyStore");
    f.confirm(&target()).expect(78, "confirm, fresh tables");
    f.yes(&target(), &"00".repeat(32))
        .expect(78, "--yes, fresh tables");
    let pin = dry.identity_digest();
    f.run(
        &[
            &target(),
            "--dry-run",
            "--expect-empty",
            "--expect-identity",
            &pin,
        ],
        &[],
    )
    .expect(0, "confirm with the pin");
    f.run(
        &[
            &target(),
            "--dry-run",
            "--expect-identity",
            &"ab".repeat(32),
        ],
        &[],
    )
    .expect(78, "a wrong pin");
}

#[test]
fn the_present_field_says_where_the_stream_was_found() {
    let Some(f) = Fixture::new("purge_ddb_present") else {
        return;
    };
    f.op_store()
        .append(&StreamId(format!("n-{H}-4")), &op("only-op", "x", ""))
        .unwrap();
    f.registry_store()
        .register(&StreamId(format!("n-{H}-3")), "ef56", "uuid-only")
        .unwrap();
    for (stream, counter, registry) in [
        (format!("n-{H}-4"), true, false),
        (format!("n-{H}-3"), false, true),
        (format!("n-{H}-5"), false, false),
    ] {
        let dry = f.dry(&stream);
        dry.expect(0, &stream);
        assert_eq!(
            dry.report["present"],
            json!({"counter": counter, "registry": registry}),
            "{stream}"
        );
    }
}

#[test]
fn the_identity_is_what_describe_table_says() {
    let f = fixture!("purge_ddb_identity_values", 1, 0);
    let dry = f.dry(&target());
    let describe = |table: &str| {
        ddb::rt()
            .block_on(f.client.describe_table().table_name(table).send())
            .unwrap()
            .table()
            .cloned()
            .unwrap()
    };
    let (ops, reg) = (describe(&f.ops), describe(&f.registry));
    let identity = &dry.report["store"]["identity"];
    assert_eq!(identity["ops_table_arn"], json!(ops.table_arn()));
    assert_eq!(identity["ops_table_id"], json!(ops.table_id()));
    assert_eq!(identity["registry_table_arn"], json!(reg.table_arn()));
    assert_eq!(identity["registry_table_id"], json!(reg.table_id()));
    let created = |d: &aws_sdk_dynamodb::types::TableDescription| {
        d.creation_date_time().map(|t| {
            t.fmt(aws_sdk_dynamodb::primitives::DateTimeFormat::DateTime)
                .unwrap()
        })
    };
    assert_eq!(identity["ops_created"], json!(created(&ops)));
    assert_eq!(identity["registry_created"], json!(created(&reg)));
}

// ----- crashes, injections and the lease ------------------------------------------------------------

#[test]
fn a_run_aborted_at_every_dynamodb_point_completes_on_rerun() {
    // 60 ops: D1 and D2 have three batches each (25, 25, 10).
    for point in [
        "L0",
        "D1-batch:1",
        "D1-batch:2",
        "D1",
        "D2-batch:1",
        "D2-batch:3",
        "D2",
        "D3",
        "D4-batch:1",
        "D4",
        "D5",
        "V0",
        "L9",
    ] {
        let f = fixture!("purge_ddb_crash", 60, 2);
        let outside = f.outside();
        let lease = [("NXF_RELAY_TEST_PURGE_LEASE_SECS", "5")];
        let dry = f.run(&[&target(), "--dry-run"], &lease);
        let crashed = f.run(
            &[&target(), "--yes", "--plan", &dry.digest()],
            &[("NXF_RELAY_TEST_PURGE_STOP_AFTER", point), lease[0]],
        );
        assert_ne!(crashed.code, Some(0), "{point}: the abort happened");
        if point != "L9" {
            assert!(
                f.lease_item().is_some(),
                "{point}: the crash left its lease"
            );
            f.run(&[&target(), "--dry-run", "--expect-empty"], &lease)
                .expect(3, &format!("{point}: the confirm finds the lease"));
        }
        finish(&f, &outside, point, &lease);
    }
}

#[test]
fn each_injection_gives_exactly_its_exit_and_residue() {
    let r = |pairs: &[(&str, u64)]| -> BTreeMap<String, u64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    };
    let cases: [(&str, i32, BTreeMap<String, u64>); 11] = [
        ("L0:op", 4, r(&[])),
        ("L0:machine", 4, r(&[])),
        ("L0:prefix", 4, r(&[])),
        ("D1-batch:1:op", 3, r(&[("ops", 1), ("claims", 1)])),
        ("D2:op", 3, r(&[("ops", 1), ("claims", 1)])),
        ("D3:op", 3, r(&[("ops", 1), ("claims", 1), ("counter", 1)])),
        ("V0:op", 3, r(&[("ops", 1), ("claims", 1), ("counter", 1)])),
        ("D1-batch:2:machine", 3, r(&[("registry", 1)])),
        ("V0:machine", 3, r(&[("registry", 1)])),
        ("D4:prefix", 3, r(&[("registry", 2)])),
        ("L9:op", 0, r(&[])),
    ];
    for (point_write, code, residue) in cases {
        let f = fixture!("purge_ddb_inject", 60, 2);
        let outside = f.outside();
        let dry = f.dry(&target());
        let out = f.run(
            &[&target(), "--yes", "--plan", &dry.digest()],
            &[("NXF_RELAY_TEST_PURGE_INJECT_AT", point_write)],
        );
        out.expect(code, point_write);
        assert_eq!(out.residue(), residue, "{point_write}");
        if point_write == "D4:prefix" {
            let entry = &out.report["residue"].as_array().unwrap()[0];
            assert_eq!(entry["by_prefix"], json!({"prefix#": 1, "uuid#": 1}));
        }
        if code == 4 {
            assert_eq!(f.lease_item(), None, "{point_write}: released");
        }
        if point_write == "L9:op" {
            let c = f.confirm(&target());
            c.expect(3, "the confirm after L9");
            assert_eq!(c.found("ops"), 1);
            assert_eq!(c.found("claims"), 1);
            assert_eq!(c.found("counter"), 1);
        }
        assert_eq!(f.outside(), outside, "{point_write}");
        finish(&f, &outside, point_write, &[]);
    }
}

#[test]
fn a_steal_mid_run_is_exit_2_with_the_batches_before_it_counted() {
    let f = fixture!("purge_ddb_steal", 60, 2);
    let outside = f.outside();
    let dry = f.dry(&target());
    let out = f.run(
        &[&target(), "--yes", "--plan", &dry.digest()],
        // The steal lands after batch 1; the pause lets less than half the ttl remain, so the
        // renewal before batch 2 runs, finds another holder and stops the run (design §4: "the
        // renewal before the next batch fails" — a renewal happens only below half the ttl).
        &[
            ("NXF_RELAY_TEST_PURGE_INJECT_AT", "D1-batch:1:steal"),
            ("NXF_RELAY_TEST_PURGE_PAUSE_AT", "D1-batch:1:12"),
            ("NXF_RELAY_TEST_PURGE_LEASE_SECS", "20"),
        ],
    );
    out.expect(2, "steal");
    assert_eq!(out.report["error"]["kind"], "LeaseLost");
    assert_eq!(out.deleted("ops"), 25, "exactly the batch before the steal");
    assert!(
        f.lease_item()
            .is_some_and(|l| l.contains("test-seam-thief")),
        "the foreign lease is left alone"
    );
    ddb::delete_raw(
        &f.client,
        &f.registry,
        "#sweep",
        &format!("stream#{}", target()),
    );
    finish(&f, &outside, "steal", &[]);
}

#[test]
fn the_lease_rule_by_the_clock_stops_a_paused_run_with_exactly_two_batches_deleted() {
    let f = fixture!("purge_ddb_pause", 60, 2);
    let dry = f.dry(&target());
    let out = f.run(
        &[&target(), "--yes", "--plan", &dry.digest()],
        &[
            ("NXF_RELAY_TEST_PURGE_LEASE_SECS", "8"),
            ("NXF_RELAY_TEST_PURGE_PAUSE_AT", "D1-batch:2:10"),
        ],
    );
    out.expect(2, "lease lost by the clock");
    assert_eq!(out.report["error"]["kind"], "LeaseLost");
    assert_eq!(out.deleted("ops"), 50);
}

#[test]
fn a_foreign_lease_holds_a_yes_off_and_an_expired_one_is_taken_over() {
    let f = fixture!("purge_ddb_lease", 5, 1);
    let sk = format!("stream#{}", target());
    ddb::put_raw(
        &f.client,
        &f.registry,
        &[
            ("stream_id", "#sweep"),
            ("sk", &sk),
            ("holder", "sweep-run"),
            ("expires_at", "#N:4102444800"),
        ],
    );
    let dry = f.dry(&target());
    dry.expect(0, "dry run reports it");
    assert_eq!(
        dry.report["lease"],
        json!({"found": true, "expired": false, "taken": null, "renewals": null, "released": null})
    );
    f.confirm(&target()).expect(3, "confirm finds it");
    let before = f.dump();
    let held = f.yes(&target(), &dry.digest());
    held.expect(2, "held");
    assert_eq!(held.report["error"]["kind"], "LeaseHeld");
    assert_eq!(
        f.dump(),
        before,
        "nothing deleted, the foreign lease untouched"
    );
    // Expired: taken over and released.
    ddb::put_raw(
        &f.client,
        &f.registry,
        &[
            ("stream_id", "#sweep"),
            ("sk", &sk),
            ("holder", "crashed"),
            ("expires_at", "#N:1"),
        ],
    );
    let yes = f.yes(&target(), &dry.digest());
    yes.expect(0, "an expired lease is taken over");
    assert_eq!(yes.report["lease"]["found"], true);
    assert_eq!(yes.report["lease"]["expired"], true);
    assert_eq!(yes.report["lease"]["released"], true);
    assert_eq!(f.lease_item(), None);
}

#[test]
fn no_lease_of_the_target_is_left_after_exits_0_1_3_4_and_78() {
    let global = |f: &Fixture| -> Vec<String> {
        f.dump()
            .into_iter()
            .filter(|r| r.contains("\"sk\": \"S:lease\""))
            .collect()
    };
    for (what, code) in [("0", 0), ("1", 1), ("3", 3), ("4", 4), ("78", 78)] {
        let f = fixture!("purge_ddb_release", 30, 1);
        let before_global = global(&f);
        let dry = f.dry(&target());
        let out = match what {
            "1" => f.run(
                &[&target(), "--yes", "--plan", &dry.digest()],
                &[("NXF_RELAY_TEST_PURGE_FAIL_AT", "D1-batch:1")],
            ),
            "3" => f.run(
                &[&target(), "--yes", "--plan", &dry.digest()],
                &[("NXF_RELAY_TEST_PURGE_INJECT_AT", "V0:op")],
            ),
            "4" => f.run(
                &[&target(), "--yes", "--plan", &dry.digest()],
                &[("NXF_RELAY_TEST_PURGE_INJECT_AT", "L0:op")],
            ),
            "78" => {
                ddb::put_raw(
                    &f.client,
                    &f.registry,
                    &[("stream_id", &target()), ("sk", "zzz#1")],
                );
                f.yes(&target(), &dry.digest())
            }
            _ => f.yes(&target(), &dry.digest()),
        };
        out.expect(code, what);
        assert_eq!(f.lease_item(), None, "exit {what}: the lease was released");
        if what != "78" {
            assert_eq!(out.report["lease"]["released"], true, "exit {what}");
        }
        assert_eq!(
            global(&f),
            before_global,
            "exit {what}: #sweep/lease untouched"
        );
        if what == "1" {
            // `deleted` counts exactly what went, and a fresh dry run and a bound --yes finish.
            assert_eq!(out.deleted("ops"), 25, "the one batch before the failure");
            let outside = f.outside();
            finish(&f, &outside, "after exit 1", &[]);
        }
    }
}

#[test]
fn a_renewal_that_fails_for_another_reason_is_exit_1_and_still_releases() {
    // MF-14: FAIL_AT=renew; a 20-second ttl and a 12-second pause after batch 1 leave less than
    // half the ttl (but more than the 5-second margin), so the run renews before batch 2.
    let f = fixture!("purge_ddb_renew_fail", 60, 1);
    let dry = f.dry(&target());
    let out = f.run(
        &[&target(), "--yes", "--plan", &dry.digest()],
        &[
            ("NXF_RELAY_TEST_PURGE_FAIL_AT", "renew"),
            ("NXF_RELAY_TEST_PURGE_LEASE_SECS", "20"),
            ("NXF_RELAY_TEST_PURGE_PAUSE_AT", "D1-batch:1:12"),
        ],
    );
    out.expect(1, "renewal failed");
    assert_eq!(out.report["lease"]["released"], true);
    assert_eq!(f.lease_item(), None);
}

// ----- the request budget ----------------------------------------------------------------------------

fn count(ops: &[String], name: &str) -> usize {
    ops.iter().filter(|o| *o == name).count()
}

#[test]
fn a_delete_run_stays_inside_its_request_budget_and_the_delete_is_the_plan() {
    let f = fixture!("purge_ddb_budget", 60, 4);
    let proxy = ddb::Proxy::start(&f.endpoint);
    let dry = f.dry(&target());
    let (n, c, r, p) = (60usize, 60usize, 2 + 2 + 4usize, 7usize);
    // An op injected after every D1 batch, sent past the proxy so the log is the purge's alone.
    let out = f.run(
        &[&target(), "--yes", "--plan", &dry.digest()],
        &[
            ("NXF_RELAY_DDB_ENDPOINT", &proxy.url),
            ("NXF_RELAY_TEST_PURGE_INJECT_AT", "D1-batch:*:op"),
            ("NXF_RELAY_TEST_PURGE_INJECT_ENDPOINT", &f.endpoint),
        ],
    );
    out.expect(3, "injected ops are residue, never deleted");
    let log = proxy.ops();
    assert_eq!(count(&log, "DescribeTable"), 2, "{log:?}");
    assert_eq!(count(&log, "Scan"), 0);
    assert_eq!(count(&log, "GetItem"), 0);
    let renewals = out.report["lease"]["renewals"].as_u64().unwrap() as usize;
    assert_eq!(count(&log, "PutItem"), 1 + renewals, "{log:?}");
    assert_eq!(count(&log, "DeleteItem"), 1, "the release");
    // The delete phase: exactly the planned batches, and no Query among them.
    let first = log.iter().position(|o| o == "BatchWriteItem").unwrap();
    let last = log.iter().rposition(|o| o == "BatchWriteItem").unwrap();
    assert!(
        log[first..=last].iter().all(|o| o == "BatchWriteItem"),
        "{log:?}"
    );
    let batches = n.div_ceil(25) + c.div_ceil(25) + 1 + r.div_ceil(25) + 1;
    assert_eq!(count(&log, "BatchWriteItem"), batches, "{log:?}");
    // Each plan (the plan and the verification) within its Query bound.
    let per_plan = n.div_ceil(p) + c.div_ceil(p) + r.div_ceil(p) + 3 + 3;
    let before = log[..first].iter().filter(|o| *o == "Query").count();
    assert!(before <= per_plan, "plan: {before} Queries > {per_plan}");
    let after = log[last..].iter().filter(|o| *o == "Query").count();
    assert!(after <= per_plan + 3, "verification: {after} Queries");
    // No injected op was deleted: three D1 batches, three injected ops left.
    assert_eq!(out.residue().get("ops"), Some(&3));
}

#[test]
fn a_shipped_ttl_run_renews_no_more_often_than_once_per_450_seconds() {
    let f = fixture!("purge_ddb_ttl", 120, 2);
    let dry = f.run(
        &[&target(), "--dry-run"],
        &[("NXF_RELAY_TEST_PURGE_PAGE", "1000")],
    );
    let started = Instant::now();
    let yes = f.run(
        &[&target(), "--yes", "--plan", &dry.digest()],
        &[("NXF_RELAY_TEST_PURGE_PAGE", "1000")],
    );
    yes.expect(0, "--yes");
    let renewals = yes.report["lease"]["renewals"].as_u64().unwrap();
    let allowed = started.elapsed().as_secs() / 450;
    assert!(
        renewals <= allowed,
        "{renewals} renewals in {:?}",
        started.elapsed()
    );
}

// ----- the report ----------------------------------------------------------------------------------

#[test]
fn the_report_names_ids_only_by_token_and_an_aws_error_by_its_code() {
    let f = fixture!("purge_ddb_report", 5, 2);
    let dry = f.dry(&target());
    let yes = f.yes(&target(), &dry.digest());
    let confirm = f.confirm(&target());
    for out in [&dry, &yes, &confirm] {
        for needle in [
            target(),
            neighbour(),
            prefix_neighbour(),
            format!("01JDDB{}", target()),
            "MARKERddb".into(),
            format!("uuid-{}", target()),
            format!("m-{}", target()),
        ] {
            assert!(!out.stdout.contains(&needle), "{needle} in stdout");
            assert!(!out.stderr.contains(&needle), "{needle} in stderr");
        }
    }
    // An AWS error is its code only.
    let out = f.run(
        &[&target(), "--dry-run"],
        &[("NXF_RELAY_DDB_TABLE", "bad name!")],
    );
    assert_eq!(
        out.report["error"]["message"], "op store: dynamodb error: ValidationException",
        "{}",
        out.stdout
    );
}
