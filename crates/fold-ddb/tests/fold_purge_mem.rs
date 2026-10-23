//! The board-fold purge's rules on [`MemTable`] (6j6v.dbmd), in the default `cargo test` — no
//! `dynamodb` feature, no endpoint. Every rule the purge has that does not depend on DynamoDB is
//! proved here: the plan and its digest, the bound delete, the fence, the table's identity, unknown
//! kinds, the residue after an injection at every point against a control fold, `UnprocessedItems`,
//! and the exit codes. `fold_purge_ddb.rs` runs the same tool as a process against DynamoDB Local.

mod common;

use common::purge::{fixture_ops, late_ops, markers, target, C9_KEY, H, H2};
use nxs_fold_ddb::deny::{AccountHash, DenyList, MemDenyList};
use nxs_fold_ddb::fold::{Folder, INDEX_REVISION};
use nxs_fold_ddb::layout::WATERMARK_KEY;
use nxs_fold_ddb::mem::{ready, MemTable};
use nxs_fold_ddb::purge::{
    exit_code, injected_ops, purge_stream_with, IdentityDigest, LogKey, PurgeError, PurgeMode,
    PurgeOptions, PurgeOutcome, PurgeSeams, Sha256Hex, StoreIdentity,
};
use nxs_fold_ddb::table::{Dated, KeyRow, Outcome, Row, Table};
use nxs_fold_ddb::write::Write;
use nxs_foundation::change::Cell;
use nxs_foundation::model::Op;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

fn folder() -> Folder {
    Folder::platform().expect("the platform folder")
}

fn fold(t: &MemTable, ops: &[(i64, Op)]) {
    ready(folder().fold_batch(t, ops)).expect("fold");
}

/// The target, folded from the fixture.
fn folded(stream: &str, tag: &str) -> MemTable {
    let t = MemTable::new(stream);
    fold(&t, &fixture_ops(tag));
    t
}

fn identity(populated: bool) -> StoreIdentity {
    StoreIdentity {
        backend: "memory",
        fold_table: "board_fold".into(),
        region: "eu-central-1".into(),
        table_arn: "arn:aws:dynamodb:eu-central-1:123456789012:table/board_fold".into(),
        table_id: Some("1b2c3d4e-0000-4000-8000-000000000001".into()),
        created: Some("2026-09-01T10:00:00Z".into()),
        populated,
    }
}

fn denied() -> MemDenyList {
    let list = MemDenyList::new();
    list.deny(&AccountHash::parse(H).unwrap());
    list
}

fn c9() -> LogKey {
    LogKey::parse(C9_KEY).unwrap()
}

/// One run: the outcome, its exit code, its report as JSON and its progress lines.
struct Run {
    outcome: PurgeOutcome,
    code: u8,
    report: Value,
    stderr: Vec<String>,
}

impl Run {
    fn digest(&self) -> Sha256Hex {
        Sha256Hex::parse(self.report["plan_digest"].as_str().expect("a plan_digest"))
            .expect("64 hex")
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

    fn error(&self) -> &str {
        self.report["error"]["kind"].as_str().unwrap_or("")
    }
}

struct Purge<'a, T: Table, D: DenyList> {
    table: &'a T,
    deny: &'a D,
    identity: StoreIdentity,
    pin: Option<IdentityDigest>,
    key: Option<LogKey>,
    seams: PurgeSeams,
}

impl<'a, T: Table, D: DenyList> Purge<'a, T, D> {
    fn new(table: &'a T, deny: &'a D) -> Self {
        Purge {
            table,
            deny,
            identity: identity(true),
            pin: None,
            key: Some(c9()),
            seams: PurgeSeams::shipped(),
        }
    }

    fn seams(mut self, vars: &[(&str, &str)]) -> Self {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.seams = PurgeSeams::from_vars(|name| vars.get(name).cloned());
        self
    }

    fn run(&self, mode: PurgeMode) -> Run {
        let opts = PurgeOptions {
            expect_identity: self.pin,
            log_key: self.key.clone(),
        };
        let clock = || SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_000_000);
        let lines = Mutex::new(Vec::new());
        let mut progress = |line: &str| lines.lock().unwrap().push(line.to_string());
        let outcome = ready(purge_stream_with(
            self.table,
            self.table,
            &self.identity,
            self.deny,
            mode,
            &opts,
            &self.seams,
            &clock,
            &mut progress,
        ));
        let code = exit_code(&outcome);
        let report = serde_json::to_value(&outcome.report).unwrap();
        Run {
            outcome,
            code,
            report,
            stderr: lines.into_inner().unwrap(),
        }
    }

    fn dry(&self) -> Run {
        self.run(PurgeMode::DryRun {
            expect_empty: false,
        })
    }

    fn confirm(&self) -> Run {
        self.run(PurgeMode::DryRun { expect_empty: true })
    }

    fn yes(&self, plan: Sha256Hex) -> Run {
        self.run(PurgeMode::Delete { plan })
    }
}

fn keys(t: &MemTable) -> BTreeSet<String> {
    t.rows().into_keys().collect()
}

fn count_prefix(t: &MemTable, prefix: &str) -> u64 {
    keys(t).iter().filter(|k| k.starts_with(prefix)).count() as u64
}

// ----- the plan --------------------------------------------------------------------------------

#[test]
fn a_dry_run_counts_what_the_partition_holds_and_deletes_nothing() {
    let t = folded(&target(), "T");
    let before = t.rows();
    let run = Purge::new(&t, &denied()).dry();
    assert_eq!(run.code, 0, "{}", run.report);
    assert_eq!(t.rows(), before, "a dry run deletes nothing");

    let all = keys(&t);
    let entries: Vec<&str> = vec![".adj#", ".ladj#", ".tadj#", ".cadj#", ".radj#", ".rsrc#"];
    let entry_count: u64 = entries.iter().map(|p| count_prefix(&t, p)).sum();
    assert_eq!(run.found("entries"), entry_count);
    assert_eq!(run.found("watermark"), 1);
    assert_eq!(
        run.found("rows"),
        all.len() as u64 - entry_count - 1,
        "every other key is a view row"
    );
    for prefix in entries {
        let by_prefix = run.kind("entries")["by_prefix"][prefix]
            .as_u64()
            .unwrap_or(0);
        assert_eq!(by_prefix, count_prefix(&t, prefix), "{prefix}");
        assert!(by_prefix > 0, "the fixture leaves {prefix} rows");
    }
    let by_table = run.kind("rows")["by_table"].as_object().unwrap().clone();
    for (table, n) in &by_table {
        assert_eq!(
            n.as_u64().unwrap(),
            count_prefix(&t, &format!("{table}#")),
            "{table}"
        );
    }
    let mark = ready(Folder::watermark(&t)).unwrap().unwrap();
    assert_eq!(run.report["folded_through"], mark.folded_through);
    assert_eq!(run.report["fence"], "denied");
    assert_eq!(run.report["mode"], "dry_run");
    assert!(run.report["kinds"][0]["deleted"].is_null());
}

#[test]
fn the_plan_digest_is_stable_and_moves_with_one_more_folded_op() {
    let t = folded(&target(), "T");
    let deny = denied();
    let first = Purge::new(&t, &deny).dry();
    let second = Purge::new(&t, &deny).dry();
    assert_eq!(first.digest(), second.digest());
    fold(&t, &late_ops("T", 1));
    let third = Purge::new(&t, &deny).dry();
    assert_ne!(first.digest(), third.digest());
}

#[test]
fn the_plan_digest_binds_every_key_and_the_watermark_position() {
    let deny = denied();
    let one = |sk: &str| {
        let t = MemTable::new(&target());
        raw(&t, sk, vec![("id", Cell::Text("x".into()))]);
        ready(folder().advance(&t, 5)).unwrap();
        t
    };
    // Equal counts, equal by_table, equal folded_through: only the keys differ.
    let (a, b) = (one("items#ab12.0001"), one("items#ab12.0002"));
    let (da, db) = (Purge::new(&a, &deny).dry(), Purge::new(&b, &deny).dry());
    assert_eq!(da.report["kinds"], db.report["kinds"]);
    assert_ne!(da.digest(), db.digest(), "the digest binds the keys");
    // Equal keys: only the watermark's position differs.
    ready(folder().advance(&b, 9)).unwrap();
    let c = one("items#ab12.0002");
    let (dc, db) = (Purge::new(&c, &deny).dry(), Purge::new(&b, &deny).dry());
    assert_ne!(dc.digest(), db.digest(), "the digest binds folded_through");
}

// ----- the delete ------------------------------------------------------------------------------

#[test]
fn a_bound_delete_empties_the_partition_and_a_confirm_proves_it() {
    let t = folded(&target(), "T");
    let deny = denied();
    let p = Purge::new(&t, &deny);
    let dry = p.dry();
    let yes = p.yes(dry.digest());
    assert_eq!(yes.code, 0, "{}", yes.report);
    assert!(t.rows().is_empty(), "{:?}", keys(&t));
    for kind in ["rows", "entries", "watermark"] {
        assert_eq!(yes.deleted(kind), dry.found(kind), "{kind}");
    }
    assert_eq!(yes.report["complete"], true);
    assert_eq!(yes.report["residue"], serde_json::json!([]));
    assert_eq!(yes.report["physical"], "provider");
    let confirm = p.confirm();
    assert_eq!(confirm.code, 0, "{}", confirm.report);
    assert_eq!(confirm.report["mode"], "confirm");
}

#[test]
fn a_delete_after_one_more_folded_op_exits_4_and_deletes_nothing() {
    let t = folded(&target(), "T");
    let deny = denied();
    let p = Purge::new(&t, &deny);
    let dry = p.dry();
    fold(&t, &late_ops("T", 1));
    let before = t.rows();
    let yes = p.yes(dry.digest());
    assert_eq!(yes.code, 4, "{}", yes.report);
    assert_eq!(yes.error(), "PlanChanged");
    assert_eq!(t.rows(), before);
}

#[test]
fn the_watermark_goes_last() {
    let t = folded(&target(), "T");
    let deny = denied();
    let p = Purge::new(&t, &deny).seams(&[("NXF_FOLD_TEST_PURGE_PAGE", "7")]);
    let dry = p.dry();
    // Stop the delete after F1 by failing F2's call: the rows are gone, the watermark stays.
    let failing = FailOn {
        inner: &t,
        sk: WATERMARK_KEY,
    };
    let run = Purge::new(&failing, &deny).yes(dry.digest());
    assert_eq!(run.code, 1, "{}", run.report);
    assert_eq!(keys(&t), BTreeSet::from([WATERMARK_KEY.to_string()]));
    assert_eq!(run.deleted("watermark"), 0);
    assert_eq!(run.deleted("rows"), dry.found("rows"));
}

// ----- the late record and the re-created partition --------------------------------------------

#[test]
fn a_late_record_after_a_complete_purge_is_found_by_the_confirm() {
    let t = folded(&target(), "T");
    let deny = denied();
    let p = Purge::new(&t, &deny);
    assert_eq!(p.yes(p.dry().digest()).code, 0);
    fold(&t, &late_ops("T", 1));
    let before = t.rows();
    let confirm = p.confirm();
    assert_eq!(confirm.code, 3, "{}", confirm.report);
    assert!(confirm.found("rows") > 0);
    assert_eq!(t.rows(), before, "the confirm deletes nothing");
    let again = p.yes(p.dry().digest());
    assert_eq!(again.code, 0, "{}", again.report);
    assert_eq!(p.confirm().code, 0);
}

#[test]
fn a_partition_re_created_by_a_rescue_is_found_by_the_confirm() {
    let t = folded(&target(), "T");
    let deny = denied();
    let p = Purge::new(&t, &deny);
    assert_eq!(p.yes(p.dry().digest()).code, 0);
    // What a rescue or backfill of a stream still admitted leaves: a watermark, no ops.
    ready(folder().advance(&t, 40)).unwrap();
    let confirm = p.confirm();
    assert_eq!(confirm.code, 3, "{}", confirm.report);
    assert_eq!(confirm.found("watermark"), 1);
}

// ----- the fence -------------------------------------------------------------------------------

#[test]
fn without_the_deny_item_the_dry_run_reports_it_and_delete_and_confirm_refuse() {
    let t = folded(&target(), "T");
    let open = MemDenyList::new();
    let p = Purge::new(&t, &open);
    let dry = p.dry();
    assert_eq!(dry.code, 0, "{}", dry.report);
    assert_eq!(dry.report["fence"], "not_denied");
    let before = t.rows();
    let queries = t.requests().queries;
    for run in [p.yes(dry.digest()), p.confirm()] {
        assert_eq!(run.code, 78, "{}", run.report);
        assert_eq!(run.error(), "NotFenced");
        assert_eq!(run.report["fence"], "not_denied");
    }
    assert_eq!(t.rows(), before);
    assert_eq!(
        t.requests().queries,
        queries,
        "NotFenced is decided before the partition is read"
    );
}

#[test]
fn another_accounts_deny_item_does_not_fence_the_target() {
    let t = folded(&target(), "T");
    let list = MemDenyList::new();
    list.deny(&AccountHash::parse(H2).unwrap());
    let p = Purge::new(&t, &list);
    assert_eq!(p.dry().report["fence"], "not_denied");
    assert_eq!(p.confirm().error(), "NotFenced");
}

#[test]
fn an_unscoped_id_is_never_fenced() {
    let t = folded("team-board-7", "U");
    let deny = denied();
    let p = Purge::new(&t, &deny);
    let dry = p.dry();
    assert_eq!(dry.code, 0);
    assert_eq!(dry.report["fence"], "unscoped");
    let before = t.rows();
    for run in [p.yes(dry.digest()), p.confirm()] {
        assert_eq!(run.code, 78, "{}", run.report);
        assert_eq!(run.error(), "NotFenced");
        assert_eq!(run.report["fence"], "unscoped");
    }
    assert_eq!(t.rows(), before);
}

#[test]
fn a_deny_read_that_fails_stops_the_run_with_exit_1_before_the_partition_is_read() {
    let t = folded(&target(), "T");
    let queries = t.requests().queries;
    for mode in [
        PurgeMode::DryRun {
            expect_empty: false,
        },
        PurgeMode::DryRun { expect_empty: true },
    ] {
        let run = Purge::new(&t, &Unreachable).run(mode);
        assert_eq!(run.code, 1, "{}", run.report);
        assert_eq!(run.error(), "Table");
        assert!(run.report["fence"].is_null());
    }
    assert_eq!(t.requests().queries, queries);
}

// ----- identity and the empty table ------------------------------------------------------------

#[test]
fn a_pin_that_does_not_match_refuses_every_mode_before_the_partition_is_read() {
    let t = folded(&target(), "T");
    let deny = denied();
    let mut p = Purge::new(&t, &deny);
    let digest = p.dry().digest();
    let queries = t.requests().queries;
    p.pin = Some(Sha256Hex([7; 32]));
    for run in [p.dry(), p.confirm(), p.yes(digest)] {
        assert_eq!(run.code, 78, "{}", run.report);
        assert_eq!(run.error(), "IdentityMismatch");
    }
    assert_eq!(t.requests().queries, queries);
}

#[test]
fn an_empty_table_is_refused_without_the_pin_and_confirmed_with_it() {
    let t = MemTable::new(&target());
    let deny = denied();
    let mut p = Purge::new(&t, &deny);
    p.identity = identity(false);
    for run in [p.dry(), p.confirm()] {
        assert_eq!(run.code, 78, "{}", run.report);
        assert_eq!(run.error(), "EmptyStore");
    }
    p.pin = Some(p.identity.digest());
    assert_eq!(p.confirm().code, 0);
}

#[test]
fn a_delete_that_empties_a_table_holding_only_the_target_exits_0() {
    let t = folded(&target(), "T");
    let deny = denied();
    let mut p = Purge::new(&t, &deny);
    p.identity = identity(false);
    let yes = p.yes(p.dry().digest());
    assert_eq!(yes.code, 0, "{}", yes.report);
    assert_eq!(p.confirm().error(), "EmptyStore");
    p.pin = Some(p.identity.digest());
    assert_eq!(p.confirm().code, 0);
}

#[test]
fn a_confirm_of_a_never_folded_stream_of_a_populated_table_exits_0() {
    let t = MemTable::new(&format!("n-{H}-77"));
    let deny = denied();
    let run = Purge::new(&t, &deny).confirm();
    assert_eq!(run.code, 0, "{}", run.report);
    for kind in ["rows", "entries", "watermark"] {
        assert_eq!(run.found(kind), 0);
    }
}

#[test]
fn an_empty_plan_has_the_same_digest_for_every_stream() {
    let t = folded(&target(), "T");
    let deny = denied();
    let p = Purge::new(&t, &deny);
    assert_eq!(p.yes(p.dry().digest()).code, 0);
    let after = p.dry().digest();
    let never = MemTable::new(&format!("n-{H}-77"));
    assert_eq!(Purge::new(&never, &deny).dry().digest(), after);
}

// ----- unknown kinds ---------------------------------------------------------------------------

fn raw(t: &MemTable, sk: &str, set: Vec<(&str, Cell)>) {
    let outcome = ready(t.write(&Write {
        sk: sk.to_string(),
        set: set.into_iter().map(|(a, c)| (a.to_string(), c)).collect(),
        remove: Vec::new(),
        condition: None,
    }))
    .unwrap();
    assert_eq!(outcome, Outcome::Written);
}

#[test]
fn an_unknown_sort_key_kind_refuses_before_anything_is_deleted() {
    let t = folded(&target(), "T");
    raw(&t, "zzz#1", vec![("v", Cell::Text("x".into()))]);
    let deny = denied();
    let p = Purge::new(&t, &deny);
    let before = t.rows();
    let dry = p.dry();
    assert_eq!(dry.code, 78, "{}", dry.report);
    assert_eq!(dry.error(), "UnknownKind");
    let yes = p.yes(Sha256Hex([0; 32]));
    assert_eq!(yes.code, 78);
    assert_eq!(t.rows(), before);
    for run in [&dry, &yes] {
        assert!(!run.report.to_string().contains("zzz#1"), "{}", run.report);
    }
}

#[test]
fn a_watermark_of_a_newer_index_revision_refuses() {
    let t = folded(&target(), "T");
    raw(
        &t,
        WATERMARK_KEY,
        vec![("revision_index", Cell::Int(INDEX_REVISION + 1))],
    );
    let deny = denied();
    let p = Purge::new(&t, &deny);
    let before = t.rows();
    assert_eq!(p.dry().code, 78);
    assert_eq!(p.dry().error(), "UnknownKind");
    assert_eq!(p.yes(Sha256Hex([0; 32])).code, 78);
    assert_eq!(t.rows(), before);
}

#[test]
fn an_unknown_prefix_is_named_only_when_it_conforms() {
    use nxs_fold_ddb::purge::name_unknown_prefix;
    assert_eq!(name_unknown_prefix("zzz#1"), "zzz#");
    assert_eq!(name_unknown_prefix(".meta#other"), ".meta#");
    assert_eq!(
        name_unknown_prefix("Secret Label#1"),
        "<nonconforming, 14 bytes>"
    );
    assert_eq!(name_unknown_prefix("nohash"), "<nonconforming, 6 bytes>");
}

// ----- injection at every point, against a control fold ----------------------------------------

/// The keys the `n`-th injection folds to, folded into an empty partition by the same folder.
fn control_keys(n: u64) -> BTreeSet<String> {
    let control = MemTable::new(&target());
    let ops: Vec<(i64, Op)> = injected_ops(n).into_iter().map(|op| (1, op)).collect();
    fold(&control, &ops);
    keys(&control)
}

#[test]
fn an_injection_at_every_point_leaves_exactly_the_control_fold_as_residue() {
    for (point, with_watermark) in [
        ("RowsBatch:1", false),
        ("RowsBatch:2", false),
        ("Rows", false),
        ("Watermark", true),
        ("V0", true),
    ] {
        let t = folded(&target(), "T");
        let deny = denied();
        let p = Purge::new(&t, &deny).seams(&[
            ("NXF_FOLD_TEST_PURGE_INJECT_AT", point),
            ("NXF_FOLD_TEST_PURGE_PAGE", "7"),
        ]);
        let dry = p.dry();
        assert!(
            dry.found("rows") + dry.found("entries") > 25,
            "the fixture needs at least two F1 batches"
        );
        let yes = p.yes(dry.digest());
        assert_eq!(yes.code, 3, "{point}: {}", yes.report);
        assert_eq!(yes.report["complete"], false);
        let mut expected = control_keys(1);
        if !with_watermark {
            expected.remove(WATERMARK_KEY);
        }
        assert_eq!(keys(&t), expected, "{point}");

        let clean = Purge::new(&t, &deny);
        let again = clean.yes(clean.dry().digest());
        assert_eq!(again.code, 0, "{point}: {}", again.report);
        assert_eq!(clean.confirm().code, 0, "{point}");
    }
}

#[test]
fn the_delete_deletes_the_plan_and_nothing_injected_after_it() {
    let t = folded(&target(), "T");
    let deny = denied();
    let counting = Counting {
        inner: &t,
        calls: Mutex::new(0),
    };
    let p = Purge::new(&counting, &deny).seams(&[("NXF_FOLD_TEST_PURGE_INJECT_AT", "RowsBatch")]);
    let dry = p.dry();
    let planned = dry.found("rows") + dry.found("entries");
    let batches = planned.div_ceil(25);
    *counting.calls.lock().unwrap() = 0;
    let yes = p.yes(dry.digest());
    assert_eq!(yes.code, 3, "{}", yes.report);
    assert_eq!(
        *counting.calls.lock().unwrap(),
        batches + 1,
        "one call per planned batch and one for the watermark"
    );
    // Every injected item is still there: no planned batch reached it.
    let mut expected = BTreeSet::new();
    for n in 1..=batches {
        expected.extend(control_keys(n).into_iter().filter(|k| k != WATERMARK_KEY));
    }
    assert_eq!(keys(&t), expected);
    assert_eq!(yes.deleted("rows") + yes.deleted("entries"), planned);
}

// ----- UnprocessedItems ------------------------------------------------------------------------

#[test]
fn unprocessed_items_are_sent_again_four_times_then_the_run_fails_counting_what_went() {
    for (partial_calls, code) in [(4, 0), (5, 1), (9, 1)] {
        let t = folded(&target(), "T");
        let deny = denied();
        let flaky = Flaky {
            inner: &t,
            partial_calls,
            state: Mutex::new((Vec::new(), 0)),
        };
        let p = Purge::new(&flaky, &deny);
        let dry = p.dry();
        let total = t.rows().len() as u64;
        let yes = p.yes(dry.digest());
        assert_eq!(yes.code, code, "{partial_calls}: {}", yes.report);
        let gone = total - t.rows().len() as u64;
        let counted: u64 = ["rows", "entries", "watermark"]
            .iter()
            .map(|k| yes.deleted(k))
            .sum();
        assert_eq!(counted, gone, "deleted counts exactly what went");
        if code == 1 {
            assert_eq!(yes.error(), "Table");
            assert!(gone > 0, "half of the first batch went");
            let clean = Purge::new(&t, &deny);
            assert_eq!(clean.yes(clean.dry().digest()).code, 0);
            assert_eq!(clean.confirm().code, 0);
        }
    }
}

// ----- defence in depth ------------------------------------------------------------------------

#[test]
fn a_key_of_another_partition_in_a_batch_stops_the_delete_before_it_is_sent() {
    let t = folded(&target(), "T");
    let deny = denied();
    let foreign = Foreign { inner: &t };
    let p = Purge::new(&foreign, &deny);
    let before = t.rows();
    let yes = p.yes(p.dry().digest());
    assert_eq!(yes.code, 1, "{}", yes.report);
    assert_eq!(t.rows(), before, "nothing was deleted");
}

#[test]
fn a_fold_writes_only_into_its_own_partition() {
    for stream in [target(), common::purge::other()] {
        let t = folded(&stream, "S");
        for (sk, row) in t.rows() {
            assert_eq!(
                row.get("stream_id"),
                Some(&Cell::Text(stream.clone())),
                "{sk}"
            );
        }
    }
}

// ----- refusals, ids in output, exit codes -----------------------------------------------------

#[test]
fn every_refused_id_is_64_with_its_rule() {
    let deny = denied();
    for (id, rule) in [
        (String::new(), "empty"),
        ("#due".to_string(), "leading_hash"),
        ("a/b".to_string(), "slash"),
        ("a\u{7}b".to_string(), "control"),
        ("x".repeat(1_001), "too_long"),
    ] {
        let t = MemTable::new(&id);
        let run = Purge::new(&t, &deny).dry();
        assert_eq!(run.code, 64, "{id:?}");
        assert_eq!(run.report["refused"]["rule"], rule);
        assert_eq!(t.requests().queries, 0);
    }
}

#[test]
fn no_report_or_progress_line_carries_the_id_a_key_or_a_marker() {
    let t = folded(&target(), "T");
    let deny = denied();
    let p = Purge::new(&t, &deny);
    let all_keys = keys(&t);
    let dry = p.dry();
    let yes = p.yes(dry.digest());
    let confirm = p.confirm();
    for run in [&dry, &yes, &confirm] {
        let text = format!("{}\n{}", run.report, run.stderr.join("\n"));
        assert!(!text.contains(&target()), "{text}");
        assert!(!text.contains(H), "the account hash: {text}");
        for marker in markers("T") {
            assert!(!text.contains(&marker), "{marker}");
        }
        // Every sort key but the watermark's, whose name is the layout's own and in `covers`.
        for key in all_keys.iter().filter(|k| *k != WATERMARK_KEY) {
            assert!(!text.contains(key.as_str()), "{key}");
        }
        assert_eq!(
            run.report["sid"],
            c9().tag(nxs_fold_ddb::purge::Kind::Sid, &target())
        );
        assert!(run
            .stderr
            .iter()
            .all(|l| l.starts_with("nxf-fold-purge: sid=h1:t1:")));
    }
}

#[test]
fn without_a_key_the_sid_is_a_dash_and_one_line_says_log_ids_are_off() {
    let t = folded(&target(), "T");
    let deny = denied();
    let mut p = Purge::new(&t, &deny);
    p.key = None;
    let run = p.dry();
    assert_eq!(run.report["sid"], "-");
    let off: Vec<&String> = run
        .stderr
        .iter()
        .filter(|l| l.contains("log ids are off"))
        .collect();
    assert_eq!(off.len(), 1, "{:?}", run.stderr);
}

#[test]
fn exit_codes_follow_the_table() {
    let deny = denied();
    let t = MemTable::new(&target());
    let base = || Purge::new(&t, &deny).confirm().outcome;
    let with = |error: PurgeError| {
        let mut o = base();
        o.error = Some(error);
        exit_code(&o)
    };
    assert_eq!(with(PurgeError::InvalidStreamId { rule: "empty" }), 64);
    assert_eq!(with(PurgeError::Usage("x".into())), 64);
    assert_eq!(with(PurgeError::Config("x".into())), 78);
    assert_eq!(with(PurgeError::NotAFoldTable("x".into())), 78);
    assert_eq!(with(PurgeError::IdentityMismatch), 78);
    assert_eq!(with(PurgeError::EmptyStore), 78);
    assert_eq!(with(PurgeError::UnknownKind(vec![])), 78);
    assert_eq!(with(PurgeError::NotADenyTable("x".into())), 78);
    assert_eq!(with(PurgeError::NotFenced), 78);
    assert_eq!(with(PurgeError::PlanChanged), 4);
    assert_eq!(with(PurgeError::Table("x".into())), 1);

    let mut confirm = base();
    assert_eq!(exit_code(&confirm), 0);
    confirm.report.kinds[2].found = 1;
    assert_eq!(exit_code(&confirm), 3);
    let mut delete = base();
    delete.report.mode = "delete";
    delete.report.residue = Some(vec![]);
    assert_eq!(exit_code(&delete), 0);
    delete.report.residue = Some(vec![nxs_fold_ddb::purge::ResidueEntry {
        kind: "rows",
        count: 1,
    }]);
    assert_eq!(exit_code(&delete), 3);
    let mut dry = base();
    dry.report.mode = "dry_run";
    dry.report.kinds[0].found = 9;
    assert_eq!(exit_code(&dry), 0);
}

/// The report's shape, pinned for a success and for an exit 1 with its partial report.
#[test]
fn the_report_shape_is_pinned() {
    fn shape(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, v)| {
                        let inner = if k == "by_table" || k == "by_prefix" || k == "identity" {
                            Value::String("<map>".into())
                        } else {
                            shape(v)
                        };
                        (k.clone(), inner)
                    })
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(shape).collect()),
            Value::String(_) => Value::String("<s>".into()),
            Value::Number(_) => Value::String("<n>".into()),
            other => other.clone(),
        }
    }
    let t = folded(&target(), "T");
    let deny = denied();
    let p = Purge::new(&t, &deny);
    let yes = p.yes(p.dry().digest());
    let golden = serde_json::json!({
        "format": "<n>", "tool_version": "<s>",
        "covers": {"view_tables": shape(&yes.report["covers"]["view_tables"]),
                   "prefixes": ["<s>", "<s>", "<s>", "<s>", "<s>", "<s>", "<s>"],
                   "index_revision": "<n>"},
        "sid": "<s>", "mode": "<s>",
        "store": {"backend": "<s>", "fold_table": "<s>", "region": "<s>", "identity": "<map>",
                  "identity_digest": "<s>", "populated": true},
        "deny": null, "fence": "<s>", "plan_digest": "<s>", "folded_through": "<n>",
        "kinds": [
            {"kind": "<s>", "found": "<n>", "deleted": "<n>", "by_table": "<map>"},
            {"kind": "<s>", "found": "<n>", "deleted": "<n>", "by_prefix": "<map>"},
            {"kind": "<s>", "found": "<n>", "deleted": "<n>"}],
        "physical": "<s>", "residue": [], "refused": null, "complete": true, "error": null,
        "started_at": "<s>", "finished_at": "<s>"
    });
    assert_eq!(shape(&yes.report), golden);
    assert_eq!(
        yes.report["store"]["identity"],
        serde_json::json!({
            "table_arn": "arn:aws:dynamodb:eu-central-1:123456789012:table/board_fold",
            "table_id": "1b2c3d4e-0000-4000-8000-000000000001",
            "created": "2026-09-01T10:00:00Z"
        })
    );
    assert_eq!(yes.report["started_at"], "2026-10-14T17:46:40Z");

    // Exit 1: the partial report, `deleted` as far as the run got, the error by kind.
    let t = folded(&target(), "T");
    let flaky = Flaky {
        inner: &t,
        partial_calls: 5,
        state: Mutex::new((Vec::new(), 0)),
    };
    let p = Purge::new(&flaky, &deny);
    let failed = p.yes(p.dry().digest());
    assert_eq!(failed.code, 1);
    let mut golden = golden;
    golden["physical"] = Value::Null;
    golden["residue"] = Value::Null;
    golden["complete"] = Value::Bool(false);
    golden["error"] = serde_json::json!({"kind": "<s>", "message": "<s>"});
    assert_eq!(shape(&failed.report), golden);
    assert_eq!(failed.report["error"]["kind"], "Table");
}

// ----- fake tables -----------------------------------------------------------------------------

/// A deny list that cannot be read.
struct Unreachable;

#[derive(Debug)]
struct Down;
impl std::fmt::Display for Down {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GetItem: ResourceNotFoundException")
    }
}
impl std::error::Error for Down {}

impl DenyList for Unreachable {
    type Error = Down;
    async fn is_denied(&self, _: &AccountHash) -> Result<bool, Down> {
        Err(Down)
    }
}

/// Delegates every request to `inner`; each wrapper overrides one.
macro_rules! delegate {
    () => {
        type Error = std::convert::Infallible;
        fn stream(&self) -> &str {
            self.inner.stream()
        }
        async fn get(&self, sk: &str) -> Result<Option<Row>, Self::Error> {
            self.inner.get(sk).await
        }
        async fn get_many(&self, sks: &[String]) -> Result<BTreeMap<String, Row>, Self::Error> {
            self.inner.get_many(sks).await
        }
        async fn write(&self, write: &Write) -> Result<Outcome, Self::Error> {
            self.inner.write(write).await
        }
        async fn delete(&self, sk: &str) -> Result<(), Self::Error> {
            self.inner.delete(sk).await
        }
        async fn query_prefix(&self, prefix: &str) -> Result<Vec<Row>, Self::Error> {
            self.inner.query_prefix(prefix).await
        }
        async fn query_active(&self) -> Result<Vec<Row>, Self::Error> {
            self.inner.query_active().await
        }
        async fn query_dated(
            &self,
            lane: Dated,
            limit: Option<usize>,
        ) -> Result<Vec<Row>, Self::Error> {
            self.inner.query_dated(lane, limit).await
        }
        async fn query_typed(&self, item_type: &str) -> Result<Vec<Row>, Self::Error> {
            self.inner.query_typed(item_type).await
        }
    };
}

/// Leaves the second half of every batch unprocessed for its first `partial_calls` calls — the
/// fake `BatchDeleter` of the DoD.
struct Flaky<'a> {
    inner: &'a MemTable,
    partial_calls: usize,
    /// The keys the last call left, and how many calls the current batch has had.
    state: Mutex<(Vec<String>, usize)>,
}

impl Table for Flaky<'_> {
    delegate!();
    async fn delete_batch(&self, sks: &[String]) -> Result<Vec<String>, Self::Error> {
        let calls = {
            let mut state = self.state.lock().unwrap();
            state.1 = if state.0 == sks { state.1 + 1 } else { 1 };
            state.1
        };
        let keep = if calls <= self.partial_calls {
            sks.len() / 2
        } else {
            sks.len()
        };
        for sk in &sks[..keep] {
            self.inner.delete(sk).await?;
        }
        let left = sks[keep..].to_vec();
        self.state.lock().unwrap().0 = left.clone();
        Ok(left)
    }
}

/// Counts the delete calls.
struct Counting<'a> {
    inner: &'a MemTable,
    calls: Mutex<u64>,
}

impl Table for Counting<'_> {
    delegate!();
    async fn delete_batch(&self, sks: &[String]) -> Result<Vec<String>, Self::Error> {
        *self.calls.lock().unwrap() += 1;
        self.inner.delete_batch(sks).await
    }
}

/// Answers one key of the plan as another partition's.
struct Foreign<'a> {
    inner: &'a MemTable,
}

impl Table for Foreign<'_> {
    delegate!();
    async fn sort_keys_after(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<KeyRow>, Option<String>), Self::Error> {
        let (mut keys, cursor) = self.inner.sort_keys_after(after, limit).await?;
        if let Some(k) = keys.iter_mut().find(|k| k.sk.starts_with("items#")) {
            k.stream = format!("n-{H}-10");
        }
        Ok((keys, cursor))
    }
}

/// Fails the delete call that names `sk`.
struct FailOn<'a> {
    inner: &'a MemTable,
    sk: &'static str,
}

#[derive(Debug)]
struct Refused;
impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BatchWriteItem: ProvisionedThroughputExceededException")
    }
}
impl std::error::Error for Refused {}

impl Table for FailOn<'_> {
    type Error = Refused;
    fn stream(&self) -> &str {
        self.inner.stream()
    }
    async fn get(&self, sk: &str) -> Result<Option<Row>, Refused> {
        Ok(ready(self.inner.get(sk)).unwrap())
    }
    async fn get_many(&self, sks: &[String]) -> Result<BTreeMap<String, Row>, Refused> {
        Ok(ready(self.inner.get_many(sks)).unwrap())
    }
    async fn write(&self, write: &Write) -> Result<Outcome, Refused> {
        Ok(ready(self.inner.write(write)).unwrap())
    }
    async fn delete(&self, sk: &str) -> Result<(), Refused> {
        ready(self.inner.delete(sk)).unwrap();
        Ok(())
    }
    async fn query_prefix(&self, prefix: &str) -> Result<Vec<Row>, Refused> {
        Ok(ready(self.inner.query_prefix(prefix)).unwrap())
    }
    async fn query_active(&self) -> Result<Vec<Row>, Refused> {
        Ok(ready(self.inner.query_active()).unwrap())
    }
    async fn query_dated(&self, lane: Dated, limit: Option<usize>) -> Result<Vec<Row>, Refused> {
        Ok(ready(self.inner.query_dated(lane, limit)).unwrap())
    }
    async fn query_typed(&self, item_type: &str) -> Result<Vec<Row>, Refused> {
        Ok(ready(self.inner.query_typed(item_type)).unwrap())
    }
    async fn delete_batch(&self, sks: &[String]) -> Result<Vec<String>, Refused> {
        if sks.iter().any(|s| s == self.sk) {
            return Err(Refused);
        }
        Ok(ready(self.inner.delete_batch(sks)).unwrap())
    }
}
