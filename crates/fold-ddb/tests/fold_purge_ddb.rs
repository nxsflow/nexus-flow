//! `nxf-fold-purge` on DynamoDB Local (6j6v.dbmd): every DoD line DynamoDB can show, each run as a
//! real `nxf-fold-purge` process against a board-fold table made by `ddb::create_table`, beside the
//! relay's ops and registry tables and a deny table made by `ddb::create_deny_table`. Keyed on
//! `NXF_RELAY_DDB_ENDPOINT` like the other DynamoDB suites (CI: a hard failure when missing).
//!
//! Every partition is folded by `Folder` from the platform fixture (`common::purge`). The target
//! `n-<h>-1` has two neighbours of the same account that an inexact key would reach (`n-<h>-10`,
//! `n-<h>-1-x`) and a fourth stream of another account (`n-<h2>-1`). Every purge process runs with
//! `NXS_LOG_HMAC_KEY` = log-id v1's C9 key and plan pages of 7 unless a test says otherwise, and
//! every report and every stderr line it produces is searched for the fixture's markers, sort keys
//! and stream ids ([`Fixture::run`]).
#![cfg(feature = "dynamodb")]

mod common;
#[path = "support/ddb.rs"]
mod ddb;

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use aws_sdk_dynamodb::config::interceptors::BeforeTransmitInterceptorContextRef;
use aws_sdk_dynamodb::config::{ConfigBag, Intercept, RuntimeComponents};
use aws_sdk_dynamodb::error::BoxError;
use aws_sdk_dynamodb::types::{ScalarAttributeType, TableStatus};
use aws_sdk_dynamodb::Client;
use common::purge::{
    fixture_ops, late_ops, markers, neighbour, other, prefix_neighbour, target, C9_KEY, H, H2,
};
use nxs_fold_ddb::ddb::{create_deny_table, create_table, DynamoDbDenyList, DynamoDbTable};
use nxs_fold_ddb::deny::{admission, Admission, DenyList};
use nxs_fold_ddb::fold::Folder;
use nxs_fold_ddb::layout::WATERMARK_KEY;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::purge::injected_ops;
use nxs_foundation::model::Op;
use serde_json::Value;

const SEAM_VARS: &[&str] = &[
    "NXF_FOLD_TEST_PURGE_STOP_AFTER",
    "NXF_FOLD_TEST_PURGE_INJECT_AT",
    "NXF_FOLD_TEST_PURGE_PAGE",
    "NXF_FOLD_TEST_PURGE_INJECT_ENDPOINT",
];

/// The streams of the fixture and the tag of their markers.
fn streams() -> [(String, &'static str); 4] {
    [
        (target(), "T"),
        (neighbour(), "N"),
        (prefix_neighbour(), "P"),
        (other(), "O"),
    ]
}

/// Whatever no report or stderr line may carry: every stream id of the fixture, both account hashes,
/// every marker, every sort key a fold of the fixture writes but the watermark's (the layout's own
/// name, in `covers`).
fn forbidden() -> &'static Vec<String> {
    static FORBIDDEN: OnceLock<Vec<String>> = OnceLock::new();
    FORBIDDEN.get_or_init(|| {
        let mut words = vec![H.to_string(), H2.to_string(), "team-board-7".to_string()];
        for (stream, tag) in streams() {
            words.push(stream.clone());
            words.extend(markers(tag));
            let t = MemTable::new(&stream);
            ready(folder().fold_batch(&t, &fixture_ops(tag))).unwrap();
            words.extend(t.rows().into_keys().filter(|k| k != WATERMARK_KEY));
        }
        words.sort();
        words.dedup();
        words
    })
}

fn folder() -> Folder {
    Folder::platform().unwrap()
}

/// One finished `nxf-fold-purge` process.
#[derive(Debug)]
struct Purged {
    /// `None` when the process was aborted (a stop seam).
    code: Option<i32>,
    report: Value,
    stdout: String,
    stderr: String,
}

impl Purged {
    fn digest(&self) -> String {
        self.report["plan_digest"]
            .as_str()
            .unwrap_or_else(|| panic!("no plan_digest in {}", self.stdout))
            .to_string()
    }

    fn identity_digest(&self) -> String {
        self.report["store"]["identity_digest"]
            .as_str()
            .unwrap_or_else(|| panic!("no identity_digest in {}", self.stdout))
            .to_string()
    }

    fn kind(&self, kind: &str) -> Value {
        self.report["kinds"]
            .as_array()
            .and_then(|ks| ks.iter().find(|k| k["kind"] == kind).cloned())
            .unwrap_or(Value::Null)
    }

    fn found(&self, kind: &str) -> u64 {
        self.kind(kind)["found"].as_u64().unwrap_or(0)
    }

    fn deleted(&self, kind: &str) -> u64 {
        self.kind(kind)["deleted"].as_u64().unwrap_or(0)
    }

    fn error(&self) -> String {
        self.report["error"]["kind"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }

    fn total_found(&self) -> u64 {
        ["rows", "entries", "watermark"]
            .iter()
            .map(|k| self.found(k))
            .sum()
    }
}

/// A board-fold table, the relay's two tables and a deny table on DynamoDB Local.
struct Fixture {
    endpoint: String,
    client: Client,
    fold: String,
    ops: String,
    registry: String,
    deny: String,
    /// Where the purge processes are pointed (the endpoint, or a recording proxy before it).
    purge_endpoint: String,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for t in [&self.fold, &self.ops, &self.registry, &self.deny] {
            ddb::delete_table(&self.client, t);
        }
    }
}

fn wait_created(client: &Client, table: &str) {
    ddb::wait_active(client, table);
}

impl Fixture {
    /// Empty tables.
    fn new(test: &str) -> Option<Fixture> {
        let endpoint = ddb::endpoint(test)?;
        let client = ddb::client(&endpoint);
        let fold = ddb::unique("nxf_fold_purge");
        let deny = ddb::unique("nxf_deny");
        ddb::rt().block_on(async {
            create_table(&client, &fold)
                .send()
                .await
                .expect("fold table");
            create_deny_table(&client, &deny)
                .send()
                .await
                .expect("deny table");
        });
        wait_created(&client, &fold);
        wait_created(&client, &deny);
        let ops = ddb::unique("nxf_relay_ops");
        ddb::relay_table(&client, &ops, "seq", ScalarAttributeType::N);
        let registry = ddb::unique("nxf_relay_reg");
        ddb::relay_table(&client, &registry, "sk", ScalarAttributeType::S);
        for (stream, _) in streams() {
            ddb::put_raw(
                &client,
                &ops,
                &[
                    ("stream_id", &stream),
                    ("seq", "#N:1"),
                    ("value", "relay op"),
                ],
            );
            ddb::put_raw(
                &client,
                &registry,
                &[
                    ("stream_id", &stream),
                    ("sk", "prefix#ab12"),
                    ("uuid", "u-1"),
                ],
            );
        }
        Some(Fixture {
            purge_endpoint: endpoint.clone(),
            endpoint,
            client,
            fold,
            ops,
            registry,
            deny,
        })
    }

    /// The four streams folded from the fixture, and the target's account on the deny list.
    fn seeded(test: &str) -> Option<Fixture> {
        let f = Fixture::new(test)?;
        for (stream, tag) in streams() {
            f.fold_ops(&stream, &fixture_ops(tag));
        }
        f.deny_account(H);
        Some(f)
    }

    fn table(&self, stream: &str) -> DynamoDbTable {
        DynamoDbTable::new(self.client.clone(), &self.fold, stream)
    }

    fn fold_ops(&self, stream: &str, ops: &[(i64, Op)]) {
        let table = self.table(stream);
        ddb::rt()
            .block_on(folder().fold_batch(&table, ops))
            .expect("fold");
    }

    fn deny_account(&self, h: &str) {
        ddb::put_raw(
            &self.client,
            &self.deny,
            &[("h", h), ("v", "#N:1"), ("since", "2026-10-23T10:00:00Z")],
        );
    }

    fn env(&self) -> Vec<(String, String)> {
        [
            ("NXF_FOLD_DDB_TABLE", self.fold.as_str()),
            ("NXF_FOLD_DDB_REGION", ddb::REGION),
            ("NXF_FOLD_DDB_ENDPOINT", self.purge_endpoint.as_str()),
            ("NXS_DENY_TABLE", self.deny.as_str()),
            ("NXS_LOG_HMAC_KEY", C9_KEY),
            ("NXF_FOLD_TEST_PURGE_PAGE", "7"),
            ("AWS_ACCESS_KEY_ID", "test"),
            ("AWS_SECRET_ACCESS_KEY", "test"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    /// One process: `set` overrides the base environment, `unset` removes from it. Its report and
    /// stderr are searched for everything [`forbidden`] lists.
    fn run(&self, args: &[&str], set: &[(&str, &str)], unset: &[&str]) -> Purged {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_nxf-fold-purge"));
        for var in SEAM_VARS.iter().chain([
            &"NXS_DENY_TABLE",
            &"NXS_LOG_HMAC_KEY",
            &"NXF_FOLD_DDB_TABLE",
            &"NXF_FOLD_DDB_REGION",
            &"NXF_FOLD_DDB_ENDPOINT",
        ]) {
            cmd.env_remove(var);
        }
        for (k, v) in self.env() {
            if !unset.contains(&k.as_str()) {
                cmd.env(k, v);
            }
        }
        for (k, v) in set {
            cmd.env(k, v);
        }
        let out = cmd.args(args).output().expect("run nxf-fold-purge");
        let stdout = String::from_utf8(out.stdout).unwrap();
        let stderr = String::from_utf8(out.stderr).unwrap();
        for word in forbidden() {
            assert!(
                !stdout.contains(word.as_str()) && !stderr.contains(word.as_str()),
                "{args:?} printed {word:?}:\n{stdout}\n{stderr}"
            );
        }
        let report = if out.status.code().is_some() {
            serde_json::from_str(&stdout)
                .unwrap_or_else(|e| panic!("one JSON report on stdout ({e}):\n{stdout}\n{stderr}"))
        } else {
            Value::Null
        };
        Purged {
            code: out.status.code(),
            report,
            stdout,
            stderr,
        }
    }

    fn dry(&self, stream: &str) -> Purged {
        self.run(&[stream, "--dry-run"], &[], &[])
    }

    fn confirm(&self, stream: &str) -> Purged {
        self.run(&[stream, "--dry-run", "--expect-empty"], &[], &[])
    }

    fn yes(&self, stream: &str, digest: &str) -> Purged {
        self.run(&[stream, "--yes", "--plan", digest], &[], &[])
    }

    fn yes_with(&self, stream: &str, digest: &str, set: &[(&str, &str)]) -> Purged {
        self.run(&[stream, "--yes", "--plan", digest], set, &[])
    }

    /// A fresh dry run, a bound `--yes` and a confirm, each exit 0.
    fn finish(&self, stream: &str) {
        let dry = self.dry(stream);
        assert_eq!(dry.code, Some(0), "{}", dry.stdout);
        let yes = self.yes(stream, &dry.digest());
        assert_eq!(yes.code, Some(0), "{}\n{}", yes.stdout, yes.stderr);
        let confirm = self.confirm(stream);
        assert_eq!(confirm.code, Some(0), "{}", confirm.stdout);
        assert!(self.keys(stream).is_empty());
    }

    fn keys(&self, stream: &str) -> BTreeSet<String> {
        ddb::partition_keys(&self.client, &self.fold, stream)
    }

    /// Everything outside the target, as bytes: the fold table without it, and the relay's tables.
    fn outside(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        (
            ddb::dump_without(&self.client, &self.fold, &target()),
            ddb::dump(&self.client, &self.ops),
            ddb::dump(&self.client, &self.registry),
        )
    }

    /// The index partitions a stream's items sit in: active, closed, archived and each type.
    fn index_counts(&self, stream: &str) -> Vec<usize> {
        let c = |index: &str, attribute: &str, value: String| {
            ddb::index_count(&self.client, &self.fold, index, attribute, &value)
        };
        vec![
            c("active", "nxf_active", stream.to_string()),
            c("dated", "nxf_dated", format!("{stream}#closed")),
            c("dated", "nxf_dated", format!("{stream}#archived")),
            c("typed", "nxf_typed", format!("{stream}#task")),
            c("typed", "nxf_typed", format!("{stream}#epic")),
            c("typed", "nxf_typed", format!("{stream}#note")),
        ]
    }

    fn with_proxy(mut self) -> (Fixture, ddb::Proxy) {
        let proxy = ddb::Proxy::start(&self.endpoint);
        self.purge_endpoint = proxy.url.clone();
        (self, proxy)
    }
}

/// The index partitions eventually catch up; poll a little before asserting on them.
fn eventually(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..50 {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

// ----- the dry run and the plan ------------------------------------------------------------------

#[test]
fn a_dry_run_counts_the_partition_deletes_nothing_and_binds_its_digest() {
    let Some(f) = Fixture::seeded("a_dry_run_counts_the_partition") else {
        return;
    };
    let before = ddb::dump(&f.client, &f.fold);
    let dry = f.dry(&target());
    assert_eq!(dry.code, Some(0), "{}", dry.stdout);
    assert_eq!(
        ddb::dump(&f.client, &f.fold),
        before,
        "a dry run deletes nothing"
    );

    let keys = f.keys(&target());
    let count = |prefix: &str| keys.iter().filter(|k| k.starts_with(prefix)).count() as u64;
    let entry_prefixes = [".adj#", ".ladj#", ".tadj#", ".cadj#", ".radj#", ".rsrc#"];
    let entries: u64 = entry_prefixes.iter().map(|p| count(p)).sum();
    assert_eq!(dry.found("entries"), entries);
    assert_eq!(dry.found("watermark"), 1);
    assert_eq!(dry.found("rows"), keys.len() as u64 - entries - 1);
    for p in entry_prefixes {
        assert_eq!(dry.kind("entries")["by_prefix"][p], count(p), "{p}");
    }
    assert!(count(".radj#") > 0 && count(".rsrc#") > 0);
    for (table, n) in dry.kind("rows")["by_table"].as_object().unwrap() {
        assert_eq!(n.as_u64().unwrap(), count(&format!("{table}#")), "{table}");
    }
    let mark = ddb::rt()
        .block_on(Folder::watermark(&f.table(&target())))
        .unwrap()
        .unwrap();
    assert_eq!(dry.report["folded_through"], mark.folded_through);
    assert_eq!(dry.report["fence"], "denied");
    assert_eq!(dry.report["deny"]["table"], f.deny);

    let keys_flag = f.run(&[&target(), "--dry-run", "--keys"], &[], &[]);
    assert_eq!(keys_flag.code, Some(64));

    let again = f.dry(&target());
    assert_eq!(again.digest(), dry.digest(), "stable across two dry runs");
    f.fold_ops(&target(), &late_ops("T", 1));
    assert_ne!(
        f.dry(&target()).digest(),
        dry.digest(),
        "one more op folded"
    );
}

// ----- the delete --------------------------------------------------------------------------------

#[test]
fn a_bound_delete_leaves_nothing_of_the_target_and_everything_else_byte_identical() {
    let Some(f) = Fixture::seeded("a_bound_delete_leaves_nothing") else {
        return;
    };
    let outside = f.outside();
    let neighbour_indexes: Vec<Vec<usize>> = [neighbour(), prefix_neighbour(), other()]
        .iter()
        .map(|s| f.index_counts(s))
        .collect();
    assert!(
        eventually(|| f.index_counts(&target()).iter().all(|n| *n > 0)),
        "the target sits in every index partition before: {:?}",
        f.index_counts(&target())
    );
    let dry = f.dry(&target());
    let yes = f.yes(&target(), &dry.digest());
    assert_eq!(yes.code, Some(0), "{}\n{}", yes.stdout, yes.stderr);
    for kind in ["rows", "entries", "watermark"] {
        assert_eq!(yes.deleted(kind), dry.found(kind), "{kind}");
    }
    assert!(ddb::dump_of(&f.client, &f.fold, &target()).is_empty());
    assert!(
        eventually(|| f.index_counts(&target()).iter().all(|n| *n == 0)),
        "{:?}",
        f.index_counts(&target())
    );
    let all = ddb::dump(&f.client, &f.fold).join("\n");
    for marker in markers("T") {
        assert!(!all.contains(&marker), "{marker} is still in the table");
    }
    for marker in markers("N") {
        assert!(all.contains(&marker), "the neighbour's {marker} is gone");
    }
    assert_eq!(f.outside(), outside, "everything outside the target");
    let after: Vec<Vec<usize>> = [neighbour(), prefix_neighbour(), other()]
        .iter()
        .map(|s| f.index_counts(s))
        .collect();
    assert_eq!(after, neighbour_indexes);
    let confirm = f.confirm(&target());
    assert_eq!(confirm.code, Some(0), "{}", confirm.stdout);
}

#[test]
fn a_delete_after_one_more_folded_op_exits_4_and_deletes_nothing() {
    let Some(f) = Fixture::seeded("a_delete_after_one_more_folded_op") else {
        return;
    };
    let dry = f.dry(&target());
    f.fold_ops(&target(), &late_ops("T", 1));
    let before = ddb::dump(&f.client, &f.fold);
    let yes = f.yes(&target(), &dry.digest());
    assert_eq!(yes.code, Some(4), "{}", yes.stdout);
    assert_eq!(ddb::dump(&f.client, &f.fold), before);
}

/// Rows near 300 KB, so DynamoDB cuts plan pages at 1 MB well below the page of 7.
#[test]
fn a_partition_of_large_rows_is_planned_across_short_pages_and_deleted() {
    let Some(f) = Fixture::new("a_partition_of_large_rows") else {
        return;
    };
    for (stream, tag) in streams().into_iter().skip(1) {
        f.fold_ops(&stream, &fixture_ops(tag));
    }
    f.deny_account(H);
    let mut s = nexus_flow_core::store::Store::open_in_memory(3);
    let big = "x".repeat(300 * 1024);
    for i in 0..200 {
        s.create_item(&format!("big0.{i:04}"), "task", &big, "u");
    }
    let ops: Vec<(i64, Op)> = s
        .export()
        .into_iter()
        .enumerate()
        .map(|(i, op)| (i as i64 + 1, op))
        .collect();
    f.fold_ops(&target(), &ops);
    let outside = f.outside();
    let (f, proxy) = f.with_proxy();
    let dry = f.dry(&target());
    let k = dry.total_found();
    assert!(k > 200, "{k}");
    let plan_queries = proxy.ops().iter().filter(|o| *o == "Query").count() as u64;
    assert!(
        plan_queries > k.div_ceil(7) + 1,
        "the pages came back short: {plan_queries} Queries for {k} keys"
    );
    proxy.clear();
    let yes = f.yes(&target(), &dry.digest());
    assert_eq!(yes.code, Some(0), "{}\n{}", yes.stdout, yes.stderr);
    assert_eq!(yes.total_found(), k);
    let deleted: u64 = ["rows", "entries", "watermark"]
        .iter()
        .map(|kind| yes.deleted(kind))
        .sum();
    assert_eq!(deleted, k);
    let log = proxy.ops();
    assert_delete_phase(&log, k);
    assert!(f.keys(&target()).is_empty());
    assert_eq!(f.outside(), outside);
    let confirm = f.confirm(&target());
    assert_eq!(confirm.code, Some(0), "{}", confirm.stdout);
}

/// The delete phase of a request log: exactly `ceil((K-1)/25) + 1` BatchWriteItem calls, and no
/// Query between the first and the last of them.
fn assert_delete_phase(log: &[String], k: u64) {
    let first = log
        .iter()
        .position(|o| o == "BatchWriteItem")
        .expect("a delete");
    let last = log.iter().rposition(|o| o == "BatchWriteItem").unwrap();
    assert!(
        !log[first..=last].iter().any(|o| o == "Query"),
        "a Query in the delete phase: {log:?}"
    );
    let batches = log.iter().filter(|o| *o == "BatchWriteItem").count() as u64;
    assert_eq!(batches, (k - 1).div_ceil(25) + 1, "{log:?}");
}

// ----- crash, injection, the plan as the delete --------------------------------------------------

fn batches_of(dry: &Purged) -> u64 {
    (dry.found("rows") + dry.found("entries")).div_ceil(25)
}

#[test]
fn a_run_aborted_at_any_point_is_finished_by_a_fresh_dry_run_and_a_bound_delete() {
    let Some(f) = Fixture::seeded("a_run_aborted_at_any_point") else {
        return;
    };
    let outside = f.outside();
    let batches = batches_of(&f.dry(&target()));
    assert!(batches >= 2, "the fixture needs two F1 batches: {batches}");
    let points = [
        "RowsBatch:1".to_string(),
        format!("RowsBatch:{batches}"),
        "Rows".into(),
        "Watermark".into(),
        "V0".into(),
    ];
    for point in points {
        // Each point on a freshly folded target.
        f.fold_ops(&target(), &fixture_ops("T"));
        let dry = f.dry(&target());
        let aborted = f.yes_with(
            &target(),
            &dry.digest(),
            &[("NXF_FOLD_TEST_PURGE_STOP_AFTER", &point)],
        );
        assert_eq!(aborted.code, None, "{point}: the stop seam aborts");
        assert!(
            aborted.stderr.contains(&format!("stop at {point}")),
            "{point}"
        );
        f.finish(&target());
        assert_eq!(f.outside(), outside, "{point}");
    }
}

/// The keys the `n`-th injection folds to, folded into an empty partition by the same folder.
fn control_keys(n: u64) -> BTreeSet<String> {
    let control = MemTable::new(&target());
    let ops: Vec<(i64, Op)> = injected_ops(n).into_iter().map(|op| (1, op)).collect();
    ready(folder().fold_batch(&control, &ops)).unwrap();
    control.rows().into_keys().collect()
}

#[test]
fn an_injection_at_any_point_leaves_exactly_the_control_fold_as_residue() {
    let Some(f) = Fixture::seeded("an_injection_at_any_point") else {
        return;
    };
    let outside = f.outside();
    let batches = batches_of(&f.dry(&target()));
    for (point, with_watermark) in [
        ("RowsBatch:1".to_string(), false),
        (format!("RowsBatch:{batches}"), false),
        ("Rows".into(), false),
        ("Watermark".into(), true),
        ("V0".into(), true),
    ] {
        f.fold_ops(&target(), &fixture_ops("T"));
        let dry = f.dry(&target());
        let yes = f.yes_with(
            &target(),
            &dry.digest(),
            &[("NXF_FOLD_TEST_PURGE_INJECT_AT", &point)],
        );
        assert_eq!(yes.code, Some(3), "{point}: {}", yes.stdout);
        let mut expected = control_keys(1);
        if !with_watermark {
            expected.remove(WATERMARK_KEY);
        }
        assert_eq!(f.keys(&target()), expected, "{point}");
        assert_eq!(f.outside(), outside, "{point}");
        f.finish(&target());
    }
}

#[test]
fn the_delete_is_the_plan_and_reaches_no_row_written_after_it() {
    let Some(f) = Fixture::seeded("the_delete_is_the_plan") else {
        return;
    };
    let endpoint = f.endpoint.clone();
    let (f, proxy) = f.with_proxy();
    let dry = f.dry(&target());
    let k = dry.total_found();
    let batches = batches_of(&dry);
    proxy.clear();
    // The injections go straight to DynamoDB Local, past the proxy, so the log is the purge's own.
    let yes = f.yes_with(
        &target(),
        &dry.digest(),
        &[
            ("NXF_FOLD_TEST_PURGE_INJECT_AT", "RowsBatch"),
            ("NXF_FOLD_TEST_PURGE_INJECT_ENDPOINT", &endpoint),
        ],
    );
    assert_eq!(yes.code, Some(3), "{}", yes.stdout);
    assert_delete_phase(&proxy.ops(), k);
    let mut injected = BTreeSet::new();
    for n in 1..=batches {
        injected.extend(
            control_keys(n)
                .into_iter()
                .filter(|key| key != WATERMARK_KEY),
        );
    }
    assert_eq!(f.keys(&target()), injected, "no injected row was deleted");
}

// ----- late records and re-creation --------------------------------------------------------------

#[test]
fn a_late_record_after_a_complete_purge_fails_the_confirm_until_it_is_purged_too() {
    let Some(f) = Fixture::seeded("a_late_record") else {
        return;
    };
    f.finish(&target());
    f.fold_ops(&target(), &late_ops("T", 1));
    let before = ddb::dump(&f.client, &f.fold);
    let confirm = f.confirm(&target());
    assert_eq!(confirm.code, Some(3), "{}", confirm.stdout);
    assert!(confirm.total_found() > 0);
    assert_eq!(
        ddb::dump(&f.client, &f.fold),
        before,
        "the confirm deletes nothing"
    );
    f.finish(&target());
}

#[test]
fn a_partition_re_created_by_a_rescue_fails_the_confirm() {
    let Some(f) = Fixture::seeded("a_partition_re_created") else {
        return;
    };
    f.finish(&target());
    // The shape a rescue or backfill of a still-admitted stream leaves: a watermark, no ops.
    ddb::rt()
        .block_on(folder().advance(&f.table(&target()), 40))
        .unwrap();
    let confirm = f.confirm(&target());
    assert_eq!(confirm.code, Some(3), "{}", confirm.stdout);
    assert_eq!(confirm.found("watermark"), 1);
}

// ----- unknown kinds, partitions -----------------------------------------------------------------

#[test]
fn an_unknown_kind_or_a_newer_watermark_refuses_with_nothing_deleted() {
    let Some(f) = Fixture::seeded("an_unknown_kind") else {
        return;
    };
    ddb::put_raw(
        &f.client,
        &f.fold,
        &[("stream_id", &target()), ("sk", "zzz#1"), ("v", "x")],
    );
    let before = ddb::dump(&f.client, &f.fold);
    for run in [f.dry(&target()), f.yes(&target(), &"0".repeat(64))] {
        assert_eq!(run.code, Some(78), "{}", run.stdout);
        assert_eq!(run.error(), "UnknownKind");
        assert!(!run.stdout.contains("zzz#1"));
    }
    assert_eq!(ddb::dump(&f.client, &f.fold), before);

    let Some(g) = Fixture::seeded("a_newer_watermark") else {
        return;
    };
    ddb::rt().block_on(async {
        g.client
            .update_item()
            .table_name(&g.fold)
            .key(
                "stream_id",
                aws_sdk_dynamodb::types::AttributeValue::S(target()),
            )
            .key(
                "sk",
                aws_sdk_dynamodb::types::AttributeValue::S(WATERMARK_KEY.into()),
            )
            .update_expression("SET revision_index = :r")
            .expression_attribute_values(
                ":r",
                aws_sdk_dynamodb::types::AttributeValue::N(
                    (nxs_fold_ddb::fold::INDEX_REVISION + 1).to_string(),
                ),
            )
            .send()
            .await
            .unwrap();
    });
    let before = ddb::dump(&g.client, &g.fold);
    for run in [g.dry(&target()), g.yes(&target(), &"0".repeat(64))] {
        assert_eq!(run.code, Some(78), "{}", run.stdout);
        assert_eq!(run.error(), "UnknownKind");
    }
    assert_eq!(ddb::dump(&g.client, &g.fold), before);
}

#[test]
fn a_fold_into_two_streams_writes_each_only_into_its_own_partition() {
    let Some(f) = Fixture::new("a_fold_into_two_streams") else {
        return;
    };
    f.fold_ops(&target(), &fixture_ops("T"));
    f.fold_ops(&other(), &fixture_ops("O"));
    let by_stream: BTreeMap<String, BTreeSet<String>> = [target(), other()]
        .into_iter()
        .map(|s| (s.clone(), f.keys(&s)))
        .collect();
    // Every item of the table is in one of the two partitions: nothing was written elsewhere.
    let total: usize = by_stream.values().map(BTreeSet::len).sum();
    assert_eq!(ddb::dump(&f.client, &f.fold).len(), total);
    for (stream, tag) in [(target(), "T"), (other(), "O")] {
        let control = MemTable::new(&stream);
        ready(folder().fold_batch(&control, &fixture_ops(tag))).unwrap();
        let expected: BTreeSet<String> = control.rows().into_keys().collect();
        assert_eq!(by_stream[&stream], expected, "{stream}");
    }
}

// ----- the table gate ----------------------------------------------------------------------------

const MODES: &[&[&str]] = &[
    &["--dry-run"],
    &["--dry-run", "--expect-empty"],
    &[
        "--yes",
        "--plan",
        "0000000000000000000000000000000000000000000000000000000000000000",
    ],
];

fn with_mode<'a>(stream: &'a str, mode: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![stream];
    args.extend_from_slice(mode);
    args
}

#[test]
fn every_configuration_refusal_exits_78_in_every_mode_before_any_query() {
    let Some(f) = Fixture::seeded("every_configuration_refusal") else {
        return;
    };
    let before = ddb::dump(&f.client, &f.fold);
    // A fold table from before `typed`: the keys and the `active` and `dated` indexes only.
    let untyped = ddb::unique("nxf_fold_untyped");
    ddb::untyped_fold_table(&f.client, &untyped);
    ddb::wait_active(&f.client, &untyped);
    let (f, proxy) = f.with_proxy();
    let good = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    let bad_keys = [
        format!("t1:{}", &good[..42]),
        format!("t1:{good}A"),
        format!("t1:{}=", &good[..42]),
        format!("t1:{}+", &good[..42]),
        format!("t1:{}/", &good[..42]),
        format!(":{good}"),
        format!("T1:{good}"),
        format!("{}:{good}", "a".repeat(17)),
        format!("t1:{good}:x"),
    ];
    let missing = "nxf_no_such_table".to_string();
    type Case = (String, Vec<(String, String)>, Vec<&'static str>, bool);
    let mut cases: Vec<Case> = vec![
        (
            "table unset".into(),
            vec![],
            vec!["NXF_FOLD_DDB_TABLE"],
            false,
        ),
        (
            "region unset".into(),
            vec![],
            vec!["NXF_FOLD_DDB_REGION"],
            false,
        ),
        ("deny unset".into(), vec![], vec!["NXS_DENY_TABLE"], false),
        (
            "deny empty".into(),
            vec![("NXS_DENY_TABLE".into(), String::new())],
            vec![],
            false,
        ),
        (
            "missing table".into(),
            vec![("NXF_FOLD_DDB_TABLE".into(), missing.clone())],
            vec![],
            true,
        ),
        (
            "relay ops table".into(),
            vec![("NXF_FOLD_DDB_TABLE".into(), f.ops.clone())],
            vec![],
            true,
        ),
        (
            "relay registry table".into(),
            vec![("NXF_FOLD_DDB_TABLE".into(), f.registry.clone())],
            vec![],
            true,
        ),
        (
            "fold table without typed".into(),
            vec![("NXF_FOLD_DDB_TABLE".into(), untyped.clone())],
            vec![],
            true,
        ),
        (
            "fold table as deny table".into(),
            vec![("NXS_DENY_TABLE".into(), f.fold.clone())],
            vec![],
            true,
        ),
        (
            "relay ops table as deny table".into(),
            vec![("NXS_DENY_TABLE".into(), f.ops.clone())],
            vec![],
            true,
        ),
        (
            "missing deny table".into(),
            vec![("NXS_DENY_TABLE".into(), missing.clone())],
            vec![],
            true,
        ),
    ];
    for key in &bad_keys {
        cases.push((
            "malformed key".into(),
            vec![("NXS_LOG_HMAC_KEY".into(), key.clone())],
            vec![],
            false,
        ));
    }
    for (name, set, unset, describes) in &cases {
        for mode in MODES {
            proxy.clear();
            let set: Vec<(&str, &str)> =
                set.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let run = f.run(&with_mode(&target(), mode), &set, unset);
            assert_eq!(run.code, Some(78), "{name} {mode:?}: {}", run.stdout);
            let log = proxy.ops();
            assert!(!log.iter().any(|o| o == "Query"), "{name}: {log:?}");
            if *describes {
                assert_eq!(
                    log.first().map(String::as_str),
                    Some("DescribeTable"),
                    "{name}"
                );
            } else {
                assert!(log.is_empty(), "{name}: no request at all: {log:?}");
            }
            for (_, value) in &set {
                if value.contains(good) {
                    assert!(!run.stdout.contains(good) && !run.stderr.contains(good));
                }
            }
            if name.starts_with("deny") {
                assert!(run.stdout.contains("NXS_DENY_TABLE"), "{name}: names it");
            }
            if name.contains("as deny table") || name == "missing deny table" {
                assert_eq!(run.error(), "NotADenyTable", "{name}");
            }
        }
    }
    assert_eq!(ddb::dump(&f.client, &f.fold), before, "nothing deleted");
    // Nothing created: the one table a refusal could have made is the missing one it was named.
    // (Other tests create and drop tables in parallel, so the whole list is no evidence.)
    assert!(
        !ddb::table_names(&f.client).contains(&missing),
        "nothing created"
    );
    ddb::delete_table(&f.client, &untyped);
}

#[test]
fn the_store_identity_is_describe_tables_and_changes_with_a_re_created_table() {
    let Some(f) = Fixture::seeded("the_store_identity") else {
        return;
    };
    let dry = f.dry(&target());
    let desc = ddb::rt().block_on(async {
        f.client
            .describe_table()
            .table_name(&f.fold)
            .send()
            .await
            .unwrap()
            .table()
            .cloned()
            .unwrap()
    });
    let identity = &dry.report["store"]["identity"];
    assert_eq!(identity["table_arn"], desc.table_arn().unwrap());
    assert_eq!(identity["table_id"], serde_json::json!(desc.table_id()));
    let created = desc
        .creation_date_time()
        .unwrap()
        .fmt(aws_sdk_dynamodb::primitives::DateTimeFormat::DateTime)
        .unwrap();
    assert_eq!(identity["created"], created);
    // MF-3: DynamoDB Local answers CreationDateTime but no TableId; the identity still changes with
    // a re-creation through the creation time — after more than a second (MF-4).
    ddb::delete_table(&f.client, &f.fold);
    std::thread::sleep(Duration::from_millis(1_500));
    ddb::rt().block_on(async { create_table(&f.client, &f.fold).send().await.unwrap() });
    ddb::wait_active(&f.client, &f.fold);
    f.fold_ops(&target(), &fixture_ops("T"));
    let again = f.dry(&target());
    assert_ne!(again.identity_digest(), dry.identity_digest());
}

#[test]
fn the_pin_refuses_another_table_and_lets_an_emptied_table_be_confirmed() {
    let Some(f) = Fixture::new("the_pin") else {
        return;
    };
    f.deny_account(H);
    // An existing but empty board-fold table.
    let empty = f.confirm(&target());
    assert_eq!(empty.code, Some(78), "{}", empty.stdout);
    assert_eq!(empty.error(), "EmptyStore");
    let pin = empty.identity_digest();
    let pinned = f.run(
        &[
            &target(),
            "--dry-run",
            "--expect-empty",
            "--expect-identity",
            &pin,
        ],
        &[],
        &[],
    );
    assert_eq!(pinned.code, Some(0), "{}", pinned.stdout);

    // A table holding only the target: the --yes that empties it exits 0.
    f.fold_ops(&target(), &fixture_ops("T"));
    let dry = f.dry(&target());
    let yes = f.yes(&target(), &dry.digest());
    assert_eq!(yes.code, Some(0), "{}", yes.stdout);
    let confirm = f.confirm(&target());
    // ItemCount lags (refreshed about every six hours): on DynamoDB Local it is current, so the
    // emptied table reads as empty — and is refused without the pin, confirmed with it.
    assert_eq!(confirm.code, Some(78), "{}", confirm.stdout);
    let pinned = f.run(
        &[
            &target(),
            "--dry-run",
            "--expect-empty",
            "--expect-identity",
            &pin,
        ],
        &[],
        &[],
    );
    assert_eq!(pinned.code, Some(0), "{}", pinned.stdout);

    let (f, proxy) = f.with_proxy();
    f.fold_ops(&target(), &fixture_ops("T"));
    let wrong = "1".repeat(64);
    for mode in MODES {
        proxy.clear();
        let t = target();
        let mut args = with_mode(&t, mode);
        args.extend(["--expect-identity", &wrong]);
        let run = f.run(&args, &[], &[]);
        assert_eq!(run.code, Some(78), "{mode:?}");
        assert_eq!(run.error(), "IdentityMismatch");
        assert!(
            !proxy.ops().iter().any(|o| o == "Query"),
            "{:?}",
            proxy.ops()
        );
    }
}

#[test]
fn every_other_invocation_and_every_refused_id_exits_64_reading_nothing() {
    let Some(f) = Fixture::seeded("every_other_invocation") else {
        return;
    };
    let (f, proxy) = f.with_proxy();
    let d = "0".repeat(64);
    let t = target();
    for args in [
        vec![t.as_str()],
        vec![t.as_str(), "--dry-run", "--yes"],
        vec![t.as_str(), "--yes"],
        vec![t.as_str(), "--plan", &d],
        vec![t.as_str(), "--dry-run", "--plan", &d],
        vec![t.as_str(), "--yes", "--plan", &d, "--expect-empty"],
        vec![t.as_str(), "--expect-empty"],
        vec![t.as_str(), "--dry-run", "--keys"],
        vec![t.as_str(), "--dry-run", "--dry-run"],
    ] {
        proxy.clear();
        let run = f.run(&args, &[], &[]);
        assert_eq!(run.code, Some(64), "{args:?}");
        assert!(proxy.ops().is_empty(), "{args:?} read {:?}", proxy.ops());
    }
    let long = "x".repeat(1_001);
    for id in ["", "#due", "a/b", "a\u{7}b", long.as_str()] {
        proxy.clear();
        let run = f.run(&["--dry-run", "--", id], &[], &[]);
        assert_eq!(run.code, Some(64), "{id:?}");
        let server =
            nxs_server::purge::validate_purge_id(&nxs_sync::protocol::StreamId(id.to_string()));
        let Err(nxs_server::purge::PurgeError::InvalidStreamId { rule }) = server else {
            panic!("the relay refuses {id:?} too");
        };
        assert_eq!(run.report["refused"]["rule"], rule, "{id:?}");
        assert!(proxy.ops().is_empty());
    }
}

#[test]
fn a_confirm_of_a_never_folded_stream_exits_0_and_an_empty_plan_digest_names_no_stream() {
    let Some(f) = Fixture::seeded("a_confirm_of_a_never_folded_stream") else {
        return;
    };
    let never = format!("n-{H}-77");
    let confirm = f.confirm(&never);
    assert_eq!(confirm.code, Some(0), "{}", confirm.stdout);
    assert_eq!(confirm.total_found(), 0);
    f.finish(&target());
    assert_eq!(f.dry(&target()).digest(), f.dry(&never).digest());
}

// ----- ids in output -----------------------------------------------------------------------------

#[test]
fn the_sid_is_the_relays_log_hash_and_a_dash_without_a_key() {
    let Some(f) = Fixture::seeded("the_sid") else {
        return;
    };
    let dry = f.dry(&target());
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = nxs_server::log_id::log_hash_command(
        &["sid".to_string(), target()],
        Some(C9_KEY),
        &mut out,
        &mut err,
    );
    assert_eq!(code, 0);
    let token = String::from_utf8(out).unwrap();
    assert_eq!(
        format!("sid={}\n", dry.report["sid"].as_str().unwrap()),
        token
    );
    for line in dry.stderr.lines() {
        assert!(
            line.starts_with(&format!("nxf-fold-purge: {}", token.trim())),
            "{line}"
        );
    }
    let off = f.run(&[&target(), "--dry-run"], &[], &["NXS_LOG_HMAC_KEY"]);
    assert_eq!(off.code, Some(0));
    assert_eq!(off.report["sid"], "-");
    assert_eq!(
        off.stderr
            .lines()
            .filter(|l| l.contains("log ids are off"))
            .count(),
        1,
        "{}",
        off.stderr
    );
    // An AWS error is its code only.
    let gone = f.run(
        &[&target(), "--dry-run"],
        &[("NXF_FOLD_DDB_TABLE", "nxf_no_such_table")],
        &[],
    );
    assert_eq!(gone.code, Some(78));
    assert!(
        !gone.stdout.contains("Requested resource not found"),
        "{}",
        gone.stdout
    );
}

// ----- the request budget ------------------------------------------------------------------------

#[test]
fn a_delete_run_stays_within_its_request_budget() {
    let Some(f) = Fixture::seeded("a_delete_run_stays_within_its_request_budget") else {
        return;
    };
    let (f, proxy) = f.with_proxy();
    let dry = f.dry(&target());
    let k = dry.total_found();
    proxy.clear();
    let yes = f.yes(&target(), &dry.digest());
    assert_eq!(yes.code, Some(0), "{}", yes.stdout);
    let log = proxy.ops();
    let count = |op: &str| log.iter().filter(|o| *o == op).count() as u64;
    assert_eq!(count("DescribeTable"), 2, "{log:?}");
    assert_eq!(count("GetItem"), 1, "the one deny read: {log:?}");
    for op in [
        "Scan",
        "PutItem",
        "DeleteItem",
        "UpdateItem",
        "BatchGetItem",
    ] {
        assert_eq!(count(op), 0, "{op}: {log:?}");
    }
    let first = log.iter().position(|o| o == "BatchWriteItem").unwrap();
    let last = log.iter().rposition(|o| o == "BatchWriteItem").unwrap();
    let plan = log[..first].iter().filter(|o| *o == "Query").count() as u64;
    let verification = log[last..].iter().filter(|o| *o == "Query").count() as u64;
    assert!(plan <= k.div_ceil(7) + 1, "{plan} Queries for {k} keys");
    assert!(verification <= 1 + 1, "an empty partition: {verification}");
    assert_delete_phase(&log, k);
    assert_eq!(
        log.iter()
            .filter(
                |o| !["DescribeTable", "GetItem", "Query", "BatchWriteItem"].contains(&o.as_str())
            )
            .count(),
        0,
        "{log:?}"
    );
}

// ----- a racing fold -----------------------------------------------------------------------------

#[test]
fn a_fold_racing_the_delete_never_touches_the_neighbours() {
    let Some(f) = Fixture::seeded("a_fold_racing_the_delete") else {
        return;
    };
    let outside = f.outside();
    let dry = f.dry(&target());
    let stop = Arc::new(AtomicBool::new(false));
    let racer = {
        let (stop, table) = (stop.clone(), f.table(&target()));
        std::thread::spawn(move || {
            let mut n = 100;
            while !stop.load(Ordering::Relaxed) {
                ddb::rt()
                    .block_on(folder().fold_batch(&table, &late_ops("T", n)))
                    .unwrap();
                n += 1;
            }
        })
    };
    let yes = f.yes(&target(), &dry.digest());
    stop.store(true, Ordering::Relaxed);
    racer.join().unwrap();
    assert!(
        matches!(yes.code, Some(0) | Some(3) | Some(4)),
        "{}",
        yes.stdout
    );
    assert_eq!(f.outside(), outside, "the neighbours are byte-identical");
}

// ----- the fence ---------------------------------------------------------------------------------

#[test]
fn without_the_deny_item_only_the_dry_run_runs() {
    let Some(f) = Fixture::new("without_the_deny_item") else {
        return;
    };
    for (stream, tag) in streams() {
        f.fold_ops(&stream, &fixture_ops(tag));
    }
    f.fold_ops("team-board-7", &fixture_ops("U"));
    let before = ddb::dump(&f.client, &f.fold);
    let (f, proxy) = f.with_proxy();
    for (stream, fence) in [
        (target(), "not_denied"),
        ("team-board-7".to_string(), "unscoped"),
    ] {
        let dry = f.dry(&stream);
        assert_eq!(dry.code, Some(0), "{}", dry.stdout);
        assert_eq!(dry.report["fence"], fence);
        for run in [f.yes(&stream, &dry.digest()), f.confirm(&stream)] {
            assert_eq!(run.code, Some(78), "{}", run.stdout);
            assert_eq!(run.error(), "NotFenced");
            assert_eq!(run.report["fence"], fence);
        }
        proxy.clear();
        let run = f.confirm(&stream);
        assert_eq!(run.code, Some(78));
        assert!(
            !proxy.ops().iter().any(|o| o == "Query"),
            "{:?}",
            proxy.ops()
        );
    }
    assert_eq!(ddb::dump(&f.client, &f.fold), before);
}

// ----- the deny reader and a board-fold-shaped loop ----------------------------------------------

/// Every GetItem's body, as the SDK sends it.
#[derive(Debug, Clone, Default)]
struct Bodies(Arc<Mutex<Vec<(String, Value)>>>);

impl Intercept for Bodies {
    fn name(&self) -> &'static str {
        "nxf-deny-recorder"
    }

    fn read_before_transmit(
        &self,
        context: &BeforeTransmitInterceptorContextRef<'_>,
        _: &RuntimeComponents,
        _: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let request = context.request();
        let target = request.headers().get("x-amz-target").unwrap_or("unknown");
        let op = target.rsplit('.').next().unwrap_or(target).to_string();
        let body = request
            .body()
            .bytes()
            .and_then(|b| serde_json::from_slice(b).ok())
            .unwrap_or(Value::Null);
        self.0.lock().unwrap().push((op, body));
        Ok(())
    }
}

fn recording_client(endpoint: &str, bodies: &Bodies) -> Client {
    let config = aws_sdk_dynamodb::Config::builder()
        .behavior_version(aws_sdk_dynamodb::config::BehaviorVersion::latest())
        .region(aws_sdk_dynamodb::config::Region::new(ddb::REGION))
        .endpoint_url(endpoint)
        .credentials_provider(aws_sdk_dynamodb::config::Credentials::new(
            "test",
            "test",
            None,
            None,
            "dynamodb-local",
        ))
        .interceptor(bodies.clone())
        .build();
    Client::from_conf(config)
}

#[test]
fn the_deny_reader_judges_presence_with_one_consistent_get_and_no_cache() {
    let Some(f) = Fixture::new("the_deny_reader") else {
        return;
    };
    let bodies = Bodies::default();
    let client = recording_client(&f.endpoint, &bodies);
    for (table, why) in [
        (None, "unset"),
        (Some(""), "empty"),
        (Some("nxf_no_such_table"), "missing"),
        (Some(f.fold.as_str()), "the board-fold table"),
        (Some(f.ops.as_str()), "the relay's ops table"),
    ] {
        let opened = ddb::rt().block_on(DynamoDbDenyList::open(client.clone(), table));
        assert!(opened.is_err(), "{why}");
    }
    assert!(!ddb::table_names(&f.client).contains(&"nxf_no_such_table".to_string()));
    let list = ddb::rt()
        .block_on(DynamoDbDenyList::open(client.clone(), Some(&f.deny)))
        .unwrap();
    let account = |h: &str| nxs_fold_ddb::deny::AccountHash::parse(h).unwrap();
    f.deny_account(H);
    ddb::put_raw(
        &f.client,
        &f.deny,
        &[
            ("h", H2),
            ("v", "#N:2"),
            ("since", "2026-10-23T10:00:00Z"),
            ("note", "x"),
        ],
    );
    let third = "22dd22dd22dd22dd22dd22dd22dd22dd";
    bodies.0.lock().unwrap().clear();
    let read = |h: &str| ddb::rt().block_on(list.is_denied(&account(h))).unwrap();
    assert!(read(H), "an item {{h, v: 1, since}} is a denial");
    assert!(
        read(H2),
        "an item {{h, v: 2}} with another attribute is a denial"
    );
    assert!(!read(third), "another <h> is not denied");
    assert!(!read(third), "a second read of a miss");
    f.deny_account(third);
    assert!(
        read(third),
        "an item put between two reads is seen by the second: no cache"
    );
    let calls = bodies.0.lock().unwrap().clone();
    assert_eq!(calls.len(), 5, "one GetItem a read: {calls:?}");
    for (op, body) in &calls {
        assert_eq!(op, "GetItem");
        assert_eq!(body["ConsistentRead"], true, "{body}");
        assert_eq!(body["ProjectionExpression"], "#h", "{body}");
        assert_eq!(
            body["ExpressionAttributeNames"],
            serde_json::json!({"#h": "h"})
        );
    }
    assert_eq!(
        list.table().map(|t| t.table_arn.is_some()),
        Some(true),
        "the list reports its TableArn"
    );
}

#[test]
fn a_board_fold_shaped_loop_stops_writing_a_denied_account_at_its_next_chunk() {
    let Some(f) = Fixture::new("a_board_fold_shaped_loop") else {
        return;
    };
    let list = ddb::rt()
        .block_on(DynamoDbDenyList::open(f.client.clone(), Some(&f.deny)))
        .unwrap();
    let chunks = |stream_tag: &str| -> Vec<Vec<(i64, Op)>> {
        let ops = fixture_ops(stream_tag);
        let half = ops.len() / 2;
        vec![
            ops[..half].to_vec(),
            ops[half..].to_vec(),
            late_ops(stream_tag, 3),
            late_ops(stream_tag, 4),
            late_ops(stream_tag, 5),
        ]
    };
    let (t_chunks, o_chunks) = (chunks("T"), chunks("O"));
    let mut target_after = Vec::new();
    let mut other_after = Vec::new();
    for i in 0..5 {
        if i == 2 {
            f.deny_account(H);
        }
        for (stream, chunk) in [(target(), &t_chunks[i]), (other(), &o_chunks[i])] {
            let admitted = ddb::rt().block_on(admission(&list, &stream)).unwrap();
            if admitted.writes() {
                f.fold_ops(&stream, chunk);
            }
        }
        target_after.push(f.keys(&target()));
        other_after.push(f.keys(&other()));
    }
    assert_eq!(
        target_after[4], target_after[1],
        "frozen after the deny item"
    );
    assert!(!target_after[1].is_empty());
    for i in 2..5 {
        assert!(
            other_after[i].len() > other_after[i - 1].len(),
            "the other account gains rows in chunk {}",
            i + 1
        );
    }

    // Option (b): with the deny table, a chunk for an unscoped id writes nothing.
    let before = ddb::dump(&f.client, &f.fold);
    let unscoped = ddb::rt()
        .block_on(admission(&list, "team-board-7"))
        .unwrap();
    assert_eq!(unscoped, Admission::Unscoped);
    if unscoped.writes() {
        f.fold_ops("team-board-7", &fixture_ops("U"));
    }
    assert_eq!(ddb::dump(&f.client, &f.fold), before);

    // A deny read that fails — the table gone after the list was opened — admits nothing.
    ddb::delete_table(&f.client, &f.deny);
    let failed = ddb::rt().block_on(admission(&list, &other()));
    assert!(failed.is_err(), "{failed:?}");
    let before = ddb::dump(&f.client, &f.fold);
    if failed.is_ok_and(Admission::writes) {
        f.fold_ops(&other(), &late_ops("O", 9));
    }
    assert_eq!(ddb::dump(&f.client, &f.fold), before);
}

#[test]
fn the_deny_table_made_by_create_deny_table_is_active_and_keyed_by_h() {
    let Some(f) = Fixture::new("the_deny_table_shape") else {
        return;
    };
    let desc = ddb::rt().block_on(async {
        f.client
            .describe_table()
            .table_name(&f.deny)
            .send()
            .await
            .unwrap()
            .table()
            .cloned()
            .unwrap()
    });
    assert_eq!(desc.table_status(), Some(&TableStatus::Active));
    assert_eq!(nxs_fold_ddb::ddb::deny_table_shape(&desc), Ok(()));
}
