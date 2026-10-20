//! Unit tests of the backend-neutral purge: the refusals, the digests, the report shape, the exit
//! code table, the lease rule (on a fake clock) and the run's order, against an in-memory store.
//! The backends' own behaviour is tested on the real stores (`tests/purge_*.rs`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::registry::{LeaseItem, LeaseRenewed, LeaseTaken, StreamLease};
use crate::store::{StoreError, StoreResult};

const C9_KEY: &str = "t1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

fn sid(id: &str) -> StreamId {
    StreamId(id.to_string())
}

// ----- validate_purge_id -------------------------------------------------------------------------

#[test]
fn every_refusal_rule_is_named() {
    for (id, rule) in [
        ("".to_string(), "empty"),
        ("#due".to_string(), "leading_hash"),
        ("#".to_string(), "leading_hash"),
        ("n-abc/1".to_string(), "slash"),
        ("/".to_string(), "slash"),
        ("n-abc\n1".to_string(), "control"),
        ("n-abc\u{0}".to_string(), "control"),
        ("n-abc\u{7f}".to_string(), "control"),
        ("n-abc\u{85}".to_string(), "control"),
        ("x".repeat(1_001), "too_long"),
        ("ä".repeat(501), "too_long"),
    ] {
        match validate_purge_id(&sid(&id)) {
            Err(PurgeError::InvalidStreamId { rule: got }) => assert_eq!(got, rule, "{id:?}"),
            other => panic!("{id:?}: expected {rule}, got {other:?}"),
        }
    }
}

#[test]
fn the_length_limit_is_one_thousand_bytes() {
    assert!(validate_purge_id(&sid(&"x".repeat(1_000))).is_ok());
    assert!(validate_purge_id(&sid(&"ä".repeat(500))).is_ok());
    assert!(validate_purge_id(&sid(&"x".repeat(1_001))).is_err());
}

#[test]
fn ordinary_ids_are_accepted() {
    for id in [
        "6j6v",
        "u-0123456789abcdef0123456789abcdef",
        "n-0123456789abcdef0123456789abcdef-1",
        "a#b",
        "with space",
        "ünïcode",
    ] {
        assert!(validate_purge_id(&sid(id)).is_ok(), "{id:?}");
    }
}

// ----- digests ---------------------------------------------------------------------------------

fn op(seq: i64, op_id: &str) -> PlanKey {
    PlanKey::Op {
        seq,
        op_id: op_id.to_string(),
    }
}

fn identity() -> Value {
    json!({"ops_path": "/srv/relay.sqlite", "ops_store_id": "00112233445566778899aabbccddeeff"})
}

fn plan_with(ops: Vec<PlanKey>, registry: Vec<PlanKey>) -> Plan {
    let mut o = PlannedKind::new("ops");
    o.keys = ops;
    let mut r = PlannedKind::new("registry");
    for key in &registry {
        if let PlanKey::Sk(sk) = key {
            let prefix = format!("{}#", sk.split('#').next().unwrap_or(""));
            *r.by_prefix.entry(prefix).or_insert(0) += 1;
        }
    }
    r.keys = registry;
    Plan {
        kinds: vec![o, r],
        ops: OpsFacts {
            newest_wall_clock: parse_rfc3339("2026-10-18T21:04:11Z"),
            wall_clock_unparseable: 0,
            value_bytes: Some(10),
        },
        unknown: Vec::new(),
        lease: None,
        newest_last_seen: None,
    }
}

#[test]
fn the_plan_digest_is_stable() {
    let a = plan_with(vec![op(1, "o1"), op(2, "o2")], vec![]);
    assert_eq!(
        plan_digest(&identity(), &a),
        plan_digest(&identity(), &a.clone())
    );
}

#[test]
fn the_plan_digest_changes_when_an_op_is_added() {
    let a = plan_with(vec![op(1, "o1")], vec![]);
    let b = plan_with(vec![op(1, "o1"), op(2, "o2")], vec![]);
    assert_ne!(plan_digest(&identity(), &a), plan_digest(&identity(), &b));
}

#[test]
fn the_plan_digest_is_bound_to_the_op_ids_not_only_the_counts() {
    // MF-R4-1: two streams with equal counts, equal seqs and an equal newest wall_clock.
    let a = plan_with(vec![op(1, "01HA"), op(2, "01HB")], vec![]);
    let b = plan_with(vec![op(1, "01HX"), op(2, "01HY")], vec![]);
    assert_ne!(plan_digest(&identity(), &a), plan_digest(&identity(), &b));
}

#[test]
fn the_plan_digest_changes_with_the_store_identity_and_a_registry_key() {
    let a = plan_with(vec![op(1, "o1")], vec![PlanKey::Sk("prefix#ab12".into())]);
    let b = plan_with(vec![op(1, "o1")], vec![PlanKey::Sk("prefix#ab13".into())]);
    assert_ne!(plan_digest(&identity(), &a), plan_digest(&identity(), &b));
    let other = json!({"ops_path": "/srv/relay.sqlite", "ops_store_id": "ffff"});
    assert_ne!(plan_digest(&identity(), &a), plan_digest(&other, &a));
}

#[test]
fn the_plan_digest_ignores_the_lease_expired_items_and_last_seen() {
    let a = plan_with(vec![op(1, "o1")], vec![PlanKey::Sk("machine#m1".into())]);
    let mut b = a.clone();
    b.lease = Some(LeaseItem {
        holder: "someone".into(),
        expires_at: 99,
    });
    b.newest_last_seen = Some(12345);
    b.kinds[1].expired.push(PlanKey::Sk("machine#gone".into()));
    *b.kinds[1].by_prefix.entry("machine#".into()).or_insert(0) += 1;
    assert_eq!(plan_digest(&identity(), &a), plan_digest(&identity(), &b));
}

#[test]
fn an_empty_plan_has_one_digest_per_store() {
    // No stream id goes into the digest, so the target after its purge and a never-existing id
    // share it — and a digest in a log cannot be rebuilt from a candidate id.
    let empty = plan_with(vec![], vec![]);
    assert_eq!(
        plan_digest(&identity(), &empty),
        plan_digest(&identity(), &empty.clone())
    );
    assert_eq!(
        plan_digest(&identity(), &empty).to_string().len(),
        64,
        "64 lowercase hex characters"
    );
}

#[test]
fn the_identity_digest_is_sha256_of_the_canonical_identity() {
    let a = identity();
    assert_eq!(identity_digest(&a), identity_digest(&a.clone()));
    let mut b = a.clone();
    b["ops_store_id"] = json!("ffffffffffffffffffffffffffffffff");
    assert_ne!(identity_digest(&a), identity_digest(&b));
    // Canonical: key order does not matter.
    let reordered: Value = serde_json::from_str(
        r#"{"ops_store_id":"00112233445566778899aabbccddeeff","ops_path":"/srv/relay.sqlite"}"#,
    )
    .unwrap();
    assert_eq!(identity_digest(&a), identity_digest(&reordered));
    let expected = Sha256Hex::of(serde_json::to_string(&a).unwrap().as_bytes());
    assert_eq!(identity_digest(&a), expected);
}

#[test]
fn a_digest_round_trips_through_hex() {
    let d = identity_digest(&identity());
    assert_eq!(Sha256Hex::parse(&d.to_string()), Some(d));
    assert_eq!(Sha256Hex::parse(&d.to_string().to_uppercase()), Some(d));
    assert_eq!(Sha256Hex::parse("abc"), None);
    assert_eq!(Sha256Hex::parse(&"g".repeat(64)), None);
}

#[test]
fn seqs_compress_into_inclusive_runs() {
    assert_eq!(seq_ranges([1, 2, 3, 7]), vec![[1, 3], [7, 7]]);
    assert_eq!(seq_ranges([]), Vec::<[i64; 2]>::new());
    assert_eq!(seq_ranges([5]), vec![[5, 5]]);
    assert_eq!(seq_ranges([1, 2, 4, 5, 9]), vec![[1, 2], [4, 5], [9, 9]]);
}

#[test]
fn unknown_kinds_and_tables_are_named_only_when_they_conform() {
    assert_eq!(name_unknown_prefix("zzz#1"), "zzz#");
    assert_eq!(name_unknown_prefix("snapshot#n-abc"), "snapshot#");
    assert_eq!(
        name_unknown_prefix("n-0123456789abcdef-1"),
        "<nonconforming, 20 bytes>"
    );
    assert_eq!(name_unknown_prefix("Zzz#1"), "<nonconforming, 5 bytes>");
    assert_eq!(
        name_unknown_prefix(&format!("{}#1", "a".repeat(33))),
        "<nonconforming, 35 bytes>"
    );
    assert_eq!(name_unknown_table("zzz_extra"), "zzz_extra");
    assert_eq!(name_unknown_table("Users"), "<nonconforming, 5 bytes>");
    assert_eq!(
        name_unknown_table("n-0123-stream"),
        "<nonconforming, 13 bytes>"
    );
    assert_eq!(
        name_unknown_table(&"t".repeat(64)),
        "<nonconforming, 64 bytes>"
    );
}

// ----- the fake store --------------------------------------------------------------------------

/// A fake clock: Unix seconds, moved by the test (and by the fake store, to simulate a slow run).
#[derive(Clone)]
struct FakeClock(Arc<Mutex<f64>>);

impl FakeClock {
    fn at(secs: f64) -> FakeClock {
        FakeClock(Arc::new(Mutex::new(secs)))
    }
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs_f64(*self.0.lock().unwrap())
    }
    fn advance(&self, secs: f64) {
        *self.0.lock().unwrap() += secs;
    }
}

/// A lease item in memory, with the conditions DynamoDB evaluates.
struct FakeLease {
    item: Mutex<Option<LeaseItem>>,
    renew_error: AtomicBool,
    puts: Mutex<u32>,
    deletes: Mutex<u32>,
}

impl FakeLease {
    fn new() -> FakeLease {
        FakeLease {
            item: Mutex::new(None),
            renew_error: AtomicBool::new(false),
            puts: Mutex::new(0),
            deletes: Mutex::new(0),
        }
    }
}

impl StreamLease for FakeLease {
    fn take(
        &self,
        _stream: &StreamId,
        holder: &str,
        ttl: Duration,
        now: SystemTime,
    ) -> StoreResult<LeaseTaken> {
        *self.puts.lock().unwrap() += 1;
        let mut item = self.item.lock().unwrap();
        let now_s = unix_secs(now);
        match item.clone() {
            Some(held) if held.expires_at >= now_s => Ok(LeaseTaken::Held(held)),
            previous => {
                let expires_at = unix_secs(now + ttl);
                *item = Some(LeaseItem {
                    holder: holder.to_string(),
                    expires_at,
                });
                Ok(LeaseTaken::Taken {
                    expires_at,
                    previous,
                })
            }
        }
    }

    fn renew(
        &self,
        _stream: &StreamId,
        holder: &str,
        ttl: Duration,
        now: SystemTime,
    ) -> StoreResult<LeaseRenewed> {
        *self.puts.lock().unwrap() += 1;
        if self.renew_error.load(Ordering::SeqCst) {
            return Err(StoreError("dynamodb error: ThrottlingException".into()));
        }
        let mut item = self.item.lock().unwrap();
        match item.as_mut() {
            Some(held) if held.holder == holder => {
                held.expires_at = unix_secs(now + ttl);
                Ok(LeaseRenewed::Renewed {
                    expires_at: held.expires_at,
                })
            }
            _ => Ok(LeaseRenewed::Lost),
        }
    }

    fn release(&self, _stream: &StreamId, holder: &str) -> StoreResult<bool> {
        *self.deletes.lock().unwrap() += 1;
        let mut item = self.item.lock().unwrap();
        if item.as_ref().is_some_and(|i| i.holder == holder) {
            *item = None;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn read(&self, _stream: &StreamId) -> StoreResult<Option<LeaseItem>> {
        Ok(self.item.lock().unwrap().clone())
    }
}

/// An in-memory relay store: per stream, op keys and registry keys. Deletes in batches of
/// `batch` keys, asking the lease rule before each, like the DynamoDB store.
struct FakeStore {
    data: Mutex<BTreeMap<String, (Vec<PlanKey>, Vec<PlanKey>)>>,
    identity: Value,
    lease: Option<FakeLease>,
    batch: usize,
    /// Advance the clock by this many seconds per batch (a slow store).
    secs_per_batch: f64,
    /// Seconds per plan page, and how many pages a plan reads.
    plan_pages: u64,
    secs_per_page: f64,
    clock: FakeClock,
    /// Fail the delete after this many batches.
    fail_after_batches: Option<usize>,
    /// Keys a writer adds right after the delete, before the verification.
    late_write: Mutex<Option<PlanKey>>,
    unknown: Vec<String>,
    plans: Mutex<u32>,
    sql: bool,
}

impl FakeStore {
    fn new(clock: &FakeClock) -> FakeStore {
        FakeStore {
            data: Mutex::new(BTreeMap::new()),
            identity: identity(),
            lease: None,
            batch: 2,
            secs_per_batch: 0.0,
            plan_pages: 1,
            secs_per_page: 0.0,
            clock: clock.clone(),
            fail_after_batches: None,
            late_write: Mutex::new(None),
            unknown: Vec::new(),
            plans: Mutex::new(0),
            sql: false,
        }
    }

    fn with_lease(mut self) -> FakeStore {
        self.lease = Some(FakeLease::new());
        self
    }

    fn seed(&self, stream: &str, ops: usize, registry: usize) {
        let ops = (1..=ops as i64)
            .map(|s| op(s, &format!("{stream}-op-{s}")))
            .collect();
        let reg = (0..registry)
            .map(|i| PlanKey::Sk(format!("machine#m{i}")))
            .collect();
        self.data
            .lock()
            .unwrap()
            .insert(stream.to_string(), (ops, reg));
    }

    fn count(&self, stream: &str) -> (usize, usize) {
        self.data
            .lock()
            .unwrap()
            .get(stream)
            .map(|(o, r)| (o.len(), r.len()))
            .unwrap_or((0, 0))
    }

    fn populated(&self) -> bool {
        self.data
            .lock()
            .unwrap()
            .values()
            .any(|(o, _)| !o.is_empty())
    }
}

impl PurgeStore for FakeStore {
    fn backend(&self) -> &'static str {
        if self.sql {
            "sqlite"
        } else {
            "dynamodb"
        }
    }

    fn kinds(&self) -> &'static [&'static str] {
        &["ops", "registry"]
    }

    fn covers(&self) -> Vec<String> {
        vec!["ops".into(), "registry:machine#".into()]
    }

    fn identity(&self) -> Result<StoreIdentity, PurgeError> {
        let mut locator = serde_json::Map::new();
        locator.insert("ops_table".into(), json!("ops"));
        Ok(StoreIdentity {
            backend: self.backend(),
            locator,
            identity: self.identity.clone(),
            populated: self.populated(),
        })
    }

    fn plan(&self, stream: &StreamId, ctx: &mut RunCtx<'_>) -> Result<Plan, PurgeError> {
        *self.plans.lock().unwrap() += 1;
        for _ in 0..self.plan_pages {
            self.clock.advance(self.secs_per_page);
            ctx.plan_page()?;
        }
        let (ops, reg) = self
            .data
            .lock()
            .unwrap()
            .get(stream.as_str())
            .cloned()
            .unwrap_or_default();
        let mut plan = plan_with(ops, reg);
        if plan.kinds[0].keys.is_empty() {
            plan.ops = OpsFacts::default();
        }
        plan.unknown = self.unknown.clone();
        if let Some(lease) = &self.lease {
            plan.lease = lease.read(stream)?;
        }
        Ok(plan)
    }

    fn delete(
        &self,
        stream: &StreamId,
        plan: &Plan,
        _expected: &PlanDigest,
        _identity: &Value,
        ctx: &mut RunCtx<'_>,
        deleted: &mut Deleted,
    ) -> Result<Physical, PurgeError> {
        let mut batches = 0;
        for kind in &plan.kinds {
            let keys: Vec<PlanKey> = kind.all_keys().cloned().collect();
            for chunk in keys.chunks(self.batch) {
                ctx.before_batch()?;
                if self.fail_after_batches == Some(batches) {
                    return Err(PurgeError::Store(StoreError(
                        "dynamodb error: InternalServerError".into(),
                    )));
                }
                let mut data = self.data.lock().unwrap();
                let entry = data.entry(stream.as_str().to_string()).or_default();
                let list = if kind.kind == "ops" {
                    &mut entry.0
                } else {
                    &mut entry.1
                };
                list.retain(|k| !chunk.contains(k));
                deleted.add(kind.kind, chunk.len() as u64);
                batches += 1;
                drop(data);
                self.clock.advance(self.secs_per_batch);
                ctx.reached(&format!("D-batch:{batches}"))?;
            }
        }
        if let Some(key) = self.late_write.lock().unwrap().take() {
            self.data
                .lock()
                .unwrap()
                .entry(stream.as_str().to_string())
                .or_default()
                .0
                .push(key);
        }
        Ok(if self.sql {
            Physical::FileLevel
        } else {
            Physical::Provider
        })
    }

    fn stream_lease(&self) -> Option<&dyn StreamLease> {
        self.lease.as_ref().map(|l| l as &dyn StreamLease)
    }

    #[cfg(feature = "test-seams")]
    fn inject(&self, _stream: &StreamId, _write: InjectWrite) -> Result<(), StoreError> {
        Ok(())
    }
}

struct Run {
    outcome: PurgeOutcome,
    stderr: Vec<String>,
}

impl Run {
    fn code(&self) -> u8 {
        exit_code(&self.outcome)
    }
    fn json(&self) -> Value {
        serde_json::to_value(&self.outcome.report).unwrap()
    }
    fn digest(&self) -> PlanDigest {
        Sha256Hex::parse(
            self.outcome
                .report
                .plan_digest
                .as_deref()
                .expect("a digest"),
        )
        .unwrap()
    }
}

fn run_with(
    store: &Arc<FakeStore>,
    clock: &FakeClock,
    stream: &str,
    mode: PurgeMode,
    opts: &PurgeOptions,
    seams: &PurgeSeams,
) -> Run {
    let deps = PurgeDeps {
        store: store.clone() as Arc<dyn PurgeStore>,
    };
    let c = clock.clone();
    let now = move || c.now();
    let mut stderr = Vec::new();
    let outcome = purge_stream_with(
        &deps,
        &sid(stream),
        mode,
        opts,
        seams,
        &now,
        &mut |line: &str| stderr.push(line.to_string()),
    );
    Run { outcome, stderr }
}

fn run(store: &Arc<FakeStore>, clock: &FakeClock, stream: &str, mode: PurgeMode) -> Run {
    run_with(
        store,
        clock,
        stream,
        mode,
        &PurgeOptions::default(),
        &PurgeSeams::shipped(),
    )
}

const DRY: PurgeMode = PurgeMode::DryRun {
    expect_empty: false,
};
const CONFIRM: PurgeMode = PurgeMode::DryRun { expect_empty: true };

fn yes(d: PlanDigest) -> PurgeMode {
    PurgeMode::Delete { plan: d }
}

fn seeded(clock: &FakeClock) -> Arc<FakeStore> {
    let store = FakeStore::new(clock);
    store.seed("n-h-1", 5, 3);
    store.seed("n-h-10", 4, 1);
    Arc::new(store)
}

// ----- the run ---------------------------------------------------------------------------------

#[test]
fn a_dry_run_then_a_bound_yes_then_a_confirm_empty_the_stream_and_nothing_else() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let dry = run(&store, &clock, "n-h-1", DRY);
    assert_eq!(dry.code(), 0, "{:?}", dry.outcome.error);
    assert_eq!(store.count("n-h-1"), (5, 3), "a dry run deletes nothing");
    let report = dry.json();
    assert_eq!(report["kinds"][0]["found"], 5);
    assert_eq!(report["kinds"][1]["found"], 3);
    assert_eq!(report["kinds"][0]["deleted"], Value::Null);

    let delete = run(&store, &clock, "n-h-1", yes(dry.digest()));
    assert_eq!(delete.code(), 0, "{:?}", delete.outcome.error);
    let report = delete.json();
    assert_eq!(report["kinds"][0]["deleted"], 5);
    assert_eq!(report["kinds"][1]["deleted"], 3);
    assert_eq!(report["residue"], json!([]));
    assert_eq!(report["complete"], true);
    assert_eq!(store.count("n-h-1"), (0, 0));
    assert_eq!(store.count("n-h-10"), (4, 1), "the neighbour is untouched");

    let confirm = run(&store, &clock, "n-h-1", CONFIRM);
    assert_eq!(confirm.code(), 0, "{:?}", confirm.outcome.error);
}

#[test]
fn a_yes_with_another_plans_digest_deletes_nothing_and_exits_4() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let neighbour = run(&store, &clock, "n-h-10", DRY);
    let out = run(&store, &clock, "n-h-1", yes(neighbour.digest()));
    assert_eq!(out.code(), 4);
    assert!(matches!(out.outcome.error, Some(PurgeError::PlanChanged)));
    assert_eq!(store.count("n-h-1"), (5, 3));
}

#[test]
fn a_refused_id_reads_nothing_and_reports_its_rule() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let out = run(&store, &clock, "#due", DRY);
    assert_eq!(out.code(), 64);
    assert_eq!(out.json()["refused"], json!({"rule": "leading_hash"}));
    assert_eq!(*store.plans.lock().unwrap(), 0, "nothing read");
}

#[test]
fn a_wrong_identity_pin_stops_before_the_stream_is_read() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let opts = PurgeOptions {
        expect_identity: Some(Sha256Hex([7; 32])),
        log_key: None,
    };
    for mode in [DRY, CONFIRM, yes(Sha256Hex([1; 32]))] {
        let out = run_with(&store, &clock, "n-h-1", mode, &opts, &PurgeSeams::shipped());
        assert_eq!(out.code(), 78, "{mode:?}");
        assert!(matches!(
            out.outcome.error,
            Some(PurgeError::IdentityMismatch)
        ));
    }
    assert_eq!(*store.plans.lock().unwrap(), 0, "no plan was read");
}

#[test]
fn an_empty_store_refuses_without_a_pin_and_passes_with_it() {
    let clock = FakeClock::at(1_000.0);
    let store = Arc::new(FakeStore::new(&clock));
    for mode in [DRY, CONFIRM] {
        let out = run(&store, &clock, "n-h-1", mode);
        assert_eq!(out.code(), 78, "{mode:?}");
        assert!(matches!(out.outcome.error, Some(PurgeError::EmptyStore)));
    }
    let pin = PurgeOptions {
        expect_identity: Some(identity_digest(&identity())),
        log_key: None,
    };
    let out = run_with(
        &store,
        &clock,
        "n-h-1",
        CONFIRM,
        &pin,
        &PurgeSeams::shipped(),
    );
    assert_eq!(out.code(), 0, "{:?}", out.outcome.error);
}

#[test]
fn a_yes_that_deletes_the_last_stream_exits_zero_and_the_confirm_needs_the_pin() {
    let clock = FakeClock::at(1_000.0);
    let store = Arc::new(FakeStore::new(&clock));
    store.seed("n-h-1", 3, 0);
    let dry = run(&store, &clock, "n-h-1", DRY);
    let delete = run(&store, &clock, "n-h-1", yes(dry.digest()));
    assert_eq!(
        delete.code(),
        0,
        "EmptyStore is judged on the plan, never the verification"
    );
    assert_eq!(run(&store, &clock, "n-h-1", CONFIRM).code(), 78);
    let pin = PurgeOptions {
        expect_identity: Some(identity_digest(&identity())),
        log_key: None,
    };
    let out = run_with(
        &store,
        &clock,
        "n-h-1",
        CONFIRM,
        &pin,
        &PurgeSeams::shipped(),
    );
    assert_eq!(out.code(), 0);
}

#[test]
fn a_never_existing_stream_of_a_populated_store_confirms_with_zero_found() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let out = run(&store, &clock, "n-h-404", CONFIRM);
    assert_eq!(out.code(), 0);
    assert!(out.json()["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .all(|k| k["found"] == 0));
}

#[test]
fn the_empty_plans_digest_is_the_same_after_a_purge_and_for_a_never_existing_id() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let dry = run(&store, &clock, "n-h-1", DRY);
    run(&store, &clock, "n-h-1", yes(dry.digest()));
    let after = run(&store, &clock, "n-h-1", DRY).digest();
    let never = run(&store, &clock, "n-h-404", DRY).digest();
    assert_eq!(after, never);
}

#[test]
fn a_write_after_the_delete_is_residue_and_exit_3() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let dry = run(&store, &clock, "n-h-1", DRY);
    *store.late_write.lock().unwrap() = Some(op(1, "late"));
    let out = run(&store, &clock, "n-h-1", yes(dry.digest()));
    assert_eq!(out.code(), 3);
    assert_eq!(out.json()["residue"], json!([{"kind": "ops", "count": 1}]));
    assert_eq!(out.json()["complete"], false);
    let confirm = run(&store, &clock, "n-h-1", CONFIRM);
    assert_eq!(confirm.code(), 3);
}

#[test]
fn an_unknown_kind_refuses_every_mode_before_anything_is_deleted() {
    let clock = FakeClock::at(1_000.0);
    let mut store = FakeStore::new(&clock).with_lease();
    store.unknown = vec!["zzz#".into()];
    store.seed("n-h-1", 5, 3);
    let store = Arc::new(store);
    let dry = run(&store, &clock, "n-h-1", DRY);
    assert_eq!(dry.code(), 78);
    assert!(dry.json()["error"]["message"]
        .as_str()
        .unwrap()
        .contains("zzz#"));
    let out = run(&store, &clock, "n-h-1", yes(Sha256Hex([0; 32])));
    assert_eq!(out.code(), 78);
    assert_eq!(store.count("n-h-1"), (5, 3));
    assert_eq!(
        store.lease.as_ref().unwrap().read(&sid("n-h-1")).unwrap(),
        None,
        "the lease is released after a 78"
    );
}

// ----- the lease -------------------------------------------------------------------------------

fn lease_store(clock: &FakeClock) -> Arc<FakeStore> {
    let store = FakeStore::new(clock).with_lease();
    store.seed("n-h-1", 6, 2);
    store.seed("n-h-10", 4, 1);
    Arc::new(store)
}

#[test]
fn a_foreign_unexpired_lease_makes_yes_exit_2_and_dry_run_report_it() {
    let clock = FakeClock::at(1_000.0);
    let store = lease_store(&clock);
    *store.lease.as_ref().unwrap().item.lock().unwrap() = Some(LeaseItem {
        holder: "sweep".into(),
        expires_at: 5_000,
    });
    let dry = run(&store, &clock, "n-h-1", DRY);
    assert_eq!(dry.code(), 0);
    assert_eq!(dry.json()["lease"]["found"], true);
    assert_eq!(dry.json()["lease"]["expired"], false);
    let confirm = run(&store, &clock, "n-h-1", CONFIRM);
    assert_eq!(confirm.code(), 3, "a lease item is something left");

    let out = run(&store, &clock, "n-h-1", yes(dry.digest()));
    assert_eq!(out.code(), 2);
    assert!(matches!(out.outcome.error, Some(PurgeError::LeaseHeld)));
    assert_eq!(store.count("n-h-1"), (6, 2), "nothing deleted");
    assert_eq!(
        store
            .lease
            .as_ref()
            .unwrap()
            .read(&sid("n-h-1"))
            .unwrap()
            .unwrap()
            .holder,
        "sweep",
        "never releases a lease that is not its own"
    );
}

#[test]
fn an_expired_lease_is_taken_over_and_released() {
    let clock = FakeClock::at(1_000.0);
    let store = lease_store(&clock);
    *store.lease.as_ref().unwrap().item.lock().unwrap() = Some(LeaseItem {
        holder: "crashed".into(),
        expires_at: 900,
    });
    let dry = run(&store, &clock, "n-h-1", DRY);
    assert_eq!(dry.json()["lease"]["expired"], true);
    let out = run(&store, &clock, "n-h-1", yes(dry.digest()));
    assert_eq!(out.code(), 0, "{:?}", out.outcome.error);
    let lease = &out.json()["lease"];
    assert_eq!(lease["found"], true);
    assert_eq!(lease["taken"], true);
    assert_eq!(lease["released"], true);
    assert_eq!(
        store.lease.as_ref().unwrap().read(&sid("n-h-1")).unwrap(),
        None
    );
}

#[test]
fn the_lease_is_released_after_exits_0_3_and_4_and_1() {
    let clock = FakeClock::at(1_000.0);
    // 0
    let store = lease_store(&clock);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    assert_eq!(run(&store, &clock, "n-h-1", yes(d)).code(), 0);
    assert_eq!(
        store.lease.as_ref().unwrap().read(&sid("n-h-1")).unwrap(),
        None
    );
    // 3
    let store = lease_store(&clock);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    *store.late_write.lock().unwrap() = Some(op(1, "late"));
    assert_eq!(run(&store, &clock, "n-h-1", yes(d)).code(), 3);
    assert_eq!(
        store.lease.as_ref().unwrap().read(&sid("n-h-1")).unwrap(),
        None
    );
    // 4
    let store = lease_store(&clock);
    assert_eq!(
        run(&store, &clock, "n-h-1", yes(Sha256Hex([9; 32]))).code(),
        4
    );
    assert_eq!(
        store.lease.as_ref().unwrap().read(&sid("n-h-1")).unwrap(),
        None
    );
    // 1
    let mut fresh = FakeStore::new(&clock).with_lease();
    fresh.fail_after_batches = Some(1);
    fresh.seed("n-h-1", 6, 2);
    let store = Arc::new(fresh);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    let out = run(&store, &clock, "n-h-1", yes(d));
    assert_eq!(out.code(), 1);
    assert_eq!(
        out.json()["kinds"][0]["deleted"],
        2,
        "exactly the one batch that went"
    );
    assert_eq!(out.json()["lease"]["released"], true);
    assert_eq!(
        store.lease.as_ref().unwrap().read(&sid("n-h-1")).unwrap(),
        None
    );
}

#[test]
fn a_run_past_its_own_expiry_is_lease_lost_even_when_nobody_took_it() {
    // ttl 8 s, margin 2 s: a store that takes 10 s per batch is past expires_at - margin before
    // its second batch, whether or not another holder ever appears.
    let clock = FakeClock::at(1_000.0);
    let mut store = FakeStore::new(&clock).with_lease();
    store.secs_per_batch = 10.0;
    store.seed("n-h-1", 8, 0);
    let store = Arc::new(store);
    let seams = PurgeSeams::with_lease_secs(8);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    let out = run_with(
        &store,
        &clock,
        "n-h-1",
        yes(d),
        &PurgeOptions::default(),
        &seams,
    );
    assert_eq!(out.code(), 2);
    assert!(matches!(out.outcome.error, Some(PurgeError::LeaseLost)));
    assert_eq!(
        out.json()["kinds"][0]["deleted"],
        2,
        "one batch of two went"
    );
    assert_eq!(store.count("n-h-1").0, 6);
}

#[test]
fn the_lease_renews_at_half_its_ttl_and_not_more_often() {
    // 900 s ttl; each batch takes 100 s: the first renewal comes once less than 450 s are left.
    let clock = FakeClock::at(1_000.0);
    let mut store = FakeStore::new(&clock).with_lease();
    store.secs_per_batch = 100.0;
    store.seed("n-h-1", 20, 0);
    let store = Arc::new(store);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    let out = run(&store, &clock, "n-h-1", yes(d));
    assert_eq!(out.code(), 0, "{:?}", out.outcome.error);
    // 10 batches, 1000 s: renewals happen before the batch at t+500 and t+1000 (once each half).
    let renewals = out.json()["lease"]["renewals"].as_u64().unwrap();
    assert!((1..=3).contains(&renewals), "{renewals}");
    assert_eq!(
        *store.lease.as_ref().unwrap().puts.lock().unwrap() as u64,
        1 + renewals
    );
}

#[test]
fn a_renewal_lost_to_another_holder_is_lease_lost() {
    let clock = FakeClock::at(1_000.0);
    let mut store = FakeStore::new(&clock).with_lease();
    store.secs_per_batch = 300.0;
    store.seed("n-h-1", 8, 0);
    let store = Arc::new(store);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    // Someone else holds the lease from the moment we took it (a steal).
    let lease = store.lease.as_ref().unwrap();
    let deps = PurgeDeps {
        store: store.clone() as Arc<dyn PurgeStore>,
    };
    let c = clock.clone();
    let now = move || c.now();
    let s2 = store.clone();
    let mut stole = false;
    let mut progress = |line: &str| {
        if line.contains("lease taken") && !stole {
            *s2.lease.as_ref().unwrap().item.lock().unwrap() = Some(LeaseItem {
                holder: "thief".into(),
                expires_at: 999_999,
            });
            stole = true;
        }
    };
    let outcome = purge_stream_with(
        &deps,
        &sid("n-h-1"),
        yes(d),
        &PurgeOptions::default(),
        &PurgeSeams::shipped(),
        &now,
        &mut progress,
    );
    assert_eq!(exit_code(&outcome), 2);
    assert!(matches!(outcome.error, Some(PurgeError::LeaseLost)));
    assert_eq!(lease.read(&sid("n-h-1")).unwrap().unwrap().holder, "thief");
}

#[test]
fn a_renewal_that_fails_for_another_reason_is_exit_1_and_still_releases() {
    let clock = FakeClock::at(1_000.0);
    let mut store = FakeStore::new(&clock).with_lease();
    store.secs_per_batch = 500.0;
    store.seed("n-h-1", 8, 0);
    let store = Arc::new(store);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    store
        .lease
        .as_ref()
        .unwrap()
        .renew_error
        .store(true, Ordering::SeqCst);
    let out = run(&store, &clock, "n-h-1", yes(d));
    assert_eq!(out.code(), 1);
    assert_eq!(out.json()["lease"]["released"], true);
    assert_eq!(*store.lease.as_ref().unwrap().deletes.lock().unwrap(), 1);
}

#[test]
fn a_plan_of_more_than_fifty_pages_renews_during_the_plan() {
    // MF-14: 120 plan pages of 10 s each, ttl 900 s: the plan alone outlasts half the lease.
    let clock = FakeClock::at(1_000.0);
    let mut store = FakeStore::new(&clock).with_lease();
    store.plan_pages = 120;
    store.seed("n-h-1", 2, 0);
    let store = Arc::new(store);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    store.lease.as_ref().unwrap().item.lock().unwrap().take();
    let mut slow = Arc::try_unwrap(store).ok().expect("one owner");
    slow.secs_per_page = 10.0;
    let store = Arc::new(slow);
    let out = run(&store, &clock, "n-h-1", yes(d));
    assert_eq!(out.code(), 0, "{:?}", out.outcome.error);
    assert!(out.json()["lease"]["renewals"].as_u64().unwrap() >= 1);
}

// ----- the report ------------------------------------------------------------------------------

#[test]
fn with_a_key_the_report_and_progress_carry_the_sid_token() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let opts = PurgeOptions {
        expect_identity: None,
        log_key: Some(LogKey::parse(C9_KEY).unwrap()),
    };
    let out = run_with(&store, &clock, "6j6v", DRY, &opts, &PurgeSeams::shipped());
    assert_eq!(out.json()["sid"], "h1:t1:d5bfa71653806fdbcfb1052f");
    assert!(out
        .stderr
        .iter()
        .all(|l| l.contains("sid=h1:t1:d5bfa71653806fdbcfb1052f")));
    assert!(out.stderr.iter().all(|l| !l.contains("6j6v ")));
}

#[test]
fn without_a_key_the_sid_is_a_dash_and_stderr_says_so_once() {
    let clock = FakeClock::at(1_000.0);
    let store = seeded(&clock);
    let out = run(&store, &clock, "n-h-1", DRY);
    assert_eq!(out.json()["sid"], "-");
    let off = out
        .stderr
        .iter()
        .filter(|l| l.contains("log ids are off"))
        .count();
    assert_eq!(off, 1, "{:?}", out.stderr);
    assert!(
        out.stderr.iter().all(|l| !l.contains("n-h-1")),
        "{:?}",
        out.stderr
    );
}

/// The report's shape, pinned: every key, in order, for a success and for exits 1 and 2 with
/// their partial reports. Values that depend on the run are normalised first.
fn shape(report: &Value) -> Value {
    let mut r = report.clone();
    r["tool_version"] = json!("<version>");
    r["plan_digest"] = json!("<digest>");
    r["store"]["identity_digest"] = json!("<digest>");
    r
}

#[test]
fn the_report_shape_is_pinned_for_a_success() {
    let clock = FakeClock::at(1_000.0);
    let store = lease_store(&clock);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    let out = run(&store, &clock, "n-h-1", yes(d));
    let expected = json!({
        "format": 1,
        "tool_version": "<version>",
        "covers": ["ops", "registry:machine#"],
        "sid": "-",
        "mode": "delete",
        "store": {
            "backend": "dynamodb",
            "ops_table": "ops",
            "identity": identity(),
            "identity_digest": "<digest>",
            "populated": true
        },
        "plan_digest": "<digest>",
        "kinds": [
            {"kind": "ops", "found": 6, "deleted": 6, "seq_ranges": [[1, 6]], "max_seq": 6,
             "newest_wall_clock": "2026-10-18T21:04:11Z", "wall_clock_unparseable": 0,
             "value_bytes": 10},
            {"kind": "registry", "found": 2, "deleted": 2, "by_prefix": {"machine#": 2},
             "expired": 0}
        ],
        "machines": {"newest_last_seen": null},
        "large": null,
        "lease": {"found": false, "expired": null, "taken": true, "renewals": 0, "released": true},
        "present": {"counter": true, "registry": true},
        "physical": "provider",
        "residue": [],
        "refused": null,
        "complete": true,
        "error": null,
        "started_at": "1970-01-01T00:16:40Z",
        "finished_at": "1970-01-01T00:16:40Z"
    });
    assert_eq!(shape(&out.json()), expected);
    // And the key ORDER is the declared one (serde_json's map sorts, so compare the text).
    let text = serde_json::to_string(&out.outcome.report).unwrap();
    let positions: Vec<usize> = [
        "\"format\"",
        "\"tool_version\"",
        "\"covers\"",
        "\"sid\"",
        "\"mode\"",
        "\"store\"",
        "\"plan_digest\"",
        "\"kinds\"",
        "\"machines\"",
        "\"large\"",
        "\"lease\"",
        "\"present\"",
        "\"physical\"",
        "\"residue\"",
        "\"refused\"",
        "\"complete\"",
        "\"error\"",
        "\"started_at\"",
        "\"finished_at\"",
    ]
    .iter()
    .map(|k| text.find(k).unwrap_or_else(|| panic!("{k} in {text}")))
    .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{text}");
}

#[test]
fn the_report_shape_is_pinned_for_exit_1_with_its_partial_counts() {
    let clock = FakeClock::at(1_000.0);
    let mut store = FakeStore::new(&clock).with_lease();
    store.fail_after_batches = Some(2);
    store.seed("n-h-1", 6, 2);
    let store = Arc::new(store);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    let out = run(&store, &clock, "n-h-1", yes(d));
    assert_eq!(out.code(), 1);
    let r = shape(&out.json());
    assert_eq!(r["kinds"][0]["deleted"], 4);
    assert_eq!(r["kinds"][1]["deleted"], 0);
    assert_eq!(r["residue"], Value::Null, "no verification ran");
    assert_eq!(r["complete"], false);
    assert_eq!(r["physical"], Value::Null);
    assert_eq!(
        r["error"],
        json!({"kind": "Store", "message": "op store: dynamodb error: InternalServerError"})
    );
    assert_eq!(r["lease"]["released"], true);
}

#[test]
fn the_report_shape_is_pinned_for_exit_2_with_its_partial_counts() {
    let clock = FakeClock::at(1_000.0);
    let mut store = FakeStore::new(&clock).with_lease();
    store.secs_per_batch = 10.0;
    store.seed("n-h-1", 8, 0);
    let store = Arc::new(store);
    let d = run(&store, &clock, "n-h-1", DRY).digest();
    let out = run_with(
        &store,
        &clock,
        "n-h-1",
        yes(d),
        &PurgeOptions::default(),
        &PurgeSeams::with_lease_secs(8),
    );
    assert_eq!(out.code(), 2);
    let r = shape(&out.json());
    assert_eq!(r["kinds"][0]["deleted"], 2);
    assert_eq!(r["error"]["kind"], "LeaseLost");
    assert_eq!(r["lease"]["taken"], true);
    assert_eq!(r["residue"], Value::Null);
    assert_eq!(r["complete"], false);
}

// ----- exit_code -------------------------------------------------------------------------------

fn outcome(mode: PurgeMode, error: Option<PurgeError>) -> PurgeOutcome {
    PurgeOutcome {
        report: new_report(mode, "-".into(), UNIX_EPOCH),
        error,
    }
}

#[test]
fn exit_code_maps_every_error_as_the_table_says() {
    let d = yes(Sha256Hex([0; 32]));
    for (error, code) in [
        (PurgeError::InvalidStreamId { rule: "empty" }, 64),
        (PurgeError::Usage("x".into()), 64),
        (PurgeError::Config("x".into()), 78),
        (PurgeError::NotARelayStore("x".into()), 78),
        (PurgeError::IdentityMismatch, 78),
        (PurgeError::EmptyStore, 78),
        (PurgeError::UnknownKind(vec!["zzz#".into()]), 78),
        (PurgeError::Unsupported, 78),
        (PurgeError::PlanChanged, 4),
        (PurgeError::LeaseHeld, 2),
        (PurgeError::LeaseLost, 2),
        (PurgeError::Store(StoreError("x".into())), 1),
    ] {
        let name = error.kind();
        assert_eq!(exit_code(&outcome(d, Some(error))), code, "{name}");
    }
}

#[test]
fn exit_code_maps_the_runs_without_an_error() {
    // A dry run is 0 whatever it found.
    let mut o = outcome(DRY, None);
    o.report.kinds.push(KindReport {
        kind: "ops",
        found: 3,
        deleted: None,
        detail: KindDetail::Plain {},
    });
    assert_eq!(exit_code(&o), 0);
    // A confirm is 0 only with every found at 0 and no lease item.
    let mut o = outcome(CONFIRM, None);
    o.report.kinds.push(KindReport {
        kind: "ops",
        found: 0,
        deleted: None,
        detail: KindDetail::Plain {},
    });
    assert_eq!(exit_code(&o), 0);
    o.report.kinds[0].found = 1;
    assert_eq!(exit_code(&o), 3);
    o.report.kinds[0].found = 0;
    o.report.lease = Some(LeaseReport {
        found: true,
        ..LeaseReport::default()
    });
    assert_eq!(exit_code(&o), 3);
    // A delete is 3 with residue, 5 when only the checkpoint is pending, 0 otherwise.
    let d = yes(Sha256Hex([0; 32]));
    let mut o = outcome(d, None);
    o.report.residue = Some(vec![]);
    o.report.physical = Some(Physical::FileLevel);
    assert_eq!(exit_code(&o), 0);
    o.report.physical = Some(Physical::Pending);
    assert_eq!(exit_code(&o), 5);
    o.report.residue = Some(vec![ResidueEntry {
        kind: "ops",
        count: 1,
        by_prefix: None,
    }]);
    assert_eq!(exit_code(&o), 3);
}
