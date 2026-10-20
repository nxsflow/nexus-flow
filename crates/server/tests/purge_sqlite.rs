//! `nxf-relay purge` on SQLite (6j6v.f6yb), one file and two files: every DoD line of the design
//! that SQLite can show, each run as a real `nxf-relay purge` process against relay files the
//! server's own stores wrote. The real-relay acceptance with real workspaces is
//! `purge_acceptance.rs`; this suite seeds through the stores so it can afford one fresh store
//! per case.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde_json::{json, Value};
use tempfile::TempDir;

use nxs_server::presence::PresenceStore;
use nxs_server::registry::{PrefixRegistry, SqlitePrefixRegistry};
use nxs_server::store::{OpStore, SqliteOpStore};
use nxs_sync::protocol::{MachineHello, StreamId};
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

#[path = "support/purge.rs"]
mod support;
use support::{file_contains, purge, Purged, C9_KEY};

const H: &str = "0123456789abcdef0123456789abcdef";

fn target() -> String {
    format!("n-{H}-1")
}
fn neighbour() -> String {
    format!("n-{H}-10")
}
fn prefix_neighbour() -> String {
    format!("n-{H}-1-x")
}

/// A random-enough tag per fixture, so markers and ids of two fixtures never meet.
fn tag() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    );
    format!("{:012x}", h.finish() & 0xffff_ffff_ffff)
}

fn wall_clock(i: usize) -> String {
    match i {
        3 => String::new(),
        4 => "2026-10-18T23:00:00+02:00".to_string(),
        _ => format!("2026-10-18T20:{:02}:{:02}Z", (i / 60) % 60, i % 60),
    }
}

/// What a fixture seeded into one stream, to compare a dry run against.
#[derive(Debug, Clone)]
struct Seeded {
    ops: usize,
    value_bytes: u64,
    machines: usize,
    prefixes: usize,
    op_ids: Vec<String>,
    machine_ids: Vec<String>,
    replica_uuids: Vec<String>,
    prefix_values: Vec<String>,
}

/// One relay store on disk, written by the server's own SQLite stores.
struct Fixture {
    _dir: TempDir,
    ops: PathBuf,
    registry: PathBuf,
    tag: String,
    seeded: BTreeMap<String, Seeded>,
}

impl Fixture {
    fn new(two_files: bool) -> Fixture {
        let dir = TempDir::new().unwrap();
        let ops = dir.path().join("relay.sqlite");
        let registry = if two_files {
            dir.path().join("registry.sqlite")
        } else {
            ops.clone()
        };
        // The server's open writes the tables and the identity row of each file.
        SqliteOpStore::open(ops.to_str().unwrap()).unwrap();
        SqlitePrefixRegistry::open(registry.to_str().unwrap()).unwrap();
        Fixture {
            _dir: dir,
            ops,
            registry,
            tag: tag(),
            seeded: BTreeMap::new(),
        }
    }

    /// The target with `ops` ops (three carrying markers, one of them over the page size), its
    /// neighbours with the SAME counts and clocks but other ids, 2 prefixes and 30 machines each.
    fn seeded(two_files: bool, ops: usize) -> Fixture {
        let mut f = Fixture::new(two_files);
        for stream in [target(), neighbour(), prefix_neighbour()] {
            f.seed(&stream, ops, 30);
        }
        f
    }

    fn op_store(&self) -> SqliteOpStore {
        SqliteOpStore::open(self.ops.to_str().unwrap()).unwrap()
    }

    fn registry_store(&self) -> SqlitePrefixRegistry {
        SqlitePrefixRegistry::open(self.registry.to_str().unwrap()).unwrap()
    }

    fn marker(&self, stream: &str, what: &str) -> String {
        let n = stream.rsplit('-').next().unwrap_or("x");
        // Delimited on both sides: `-1` must never be a substring of `-10`'s marker.
        format!("MARKER{}{what}<{n}>", self.tag)
    }

    fn seed(&mut self, stream: &str, ops: usize, machines: usize) {
        let s = StreamId(stream.to_string());
        let store = self.op_store();
        let mut seeded = Seeded {
            ops,
            value_bytes: 0,
            machines,
            prefixes: 2,
            op_ids: Vec::new(),
            machine_ids: Vec::new(),
            replica_uuids: Vec::new(),
            prefix_values: Vec::new(),
        };
        let ops_list: Vec<WireOp> = (1..=ops)
            .map(|i| {
                let value = match i {
                    1 => self.marker(stream, "title"),
                    2 => format!("{} {}", self.marker(stream, "body"), "x".repeat(6000)),
                    3 => self.marker(stream, "deleted"),
                    _ => format!("value {i}"),
                };
                WireOp {
                    envelope_version: ENVELOPE_VERSION,
                    op_id: format!("01J{}{stream}{i:05}", self.tag),
                    lamport: i as i64,
                    site: 1,
                    domain: "task".into(),
                    target_kind: "item".into(),
                    target_id: format!("ab12.{i:04}"),
                    field: if i == 3 {
                        "deleted".into()
                    } else {
                        "title".into()
                    },
                    op_type: "set".into(),
                    value: Some(value),
                    author: "seed".into(),
                    wall_clock: wall_clock(i),
                    extra: Default::default(),
                }
            })
            .collect();
        for op in &ops_list {
            store.append(&s, op).unwrap();
            seeded.value_bytes += op.value.as_ref().map_or(0, |v| v.len() as u64);
            seeded.op_ids.push(op.op_id.clone());
        }
        let reg = self.registry_store();
        for (i, prefix) in ["ab12", "cd34"].iter().enumerate() {
            let uuid = format!("uuid-{}-{stream}-{i}", self.tag);
            reg.register(&s, prefix, &uuid).unwrap();
            seeded.replica_uuids.push(uuid);
            seeded.prefix_values.push(prefix.to_string());
        }
        for i in 0..machines {
            let id = format!("m-{}-{stream}-{i}", self.tag);
            let hello = MachineHello {
                machine_id: id.clone(),
                name: format!("{} laptop {i}", self.marker(stream, "machine")),
                interval_secs: 300,
            };
            reg.announce(&s, &hello, 1_760_000_000 + i as i64, i64::MIN)
                .unwrap();
            seeded.machine_ids.push(id);
        }
        self.seeded.insert(stream.to_string(), seeded);
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("NXF_RELAY_BACKEND", "sqlite".into()),
            ("NXF_RELAY_DB", self.ops.to_str().unwrap().into()),
            (
                "NXF_RELAY_REGISTRY_DB",
                self.registry.to_str().unwrap().into(),
            ),
            ("NXS_LOG_HMAC_KEY", C9_KEY.into()),
        ]
    }

    fn run(&self, args: &[&str], extra: &[(&str, &str)]) -> Purged {
        let mut env: Vec<(&str, String)> = self.env();
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

    fn files(&self) -> Vec<PathBuf> {
        if self.ops == self.registry {
            vec![self.ops.clone()]
        } else {
            vec![self.ops.clone(), self.registry.clone()]
        }
    }

    /// Every row of every table of every file, as text, sorted — with the rows of `without`
    /// left out of the tables that have a `stream_id`.
    fn dump(&self, without: Option<&str>) -> Vec<String> {
        let mut rows = Vec::new();
        for (fi, file) in self.files().iter().enumerate() {
            let conn = Connection::open(file).unwrap();
            let tables: Vec<String> = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            for table in tables {
                let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
                let names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                let sid_col = names.iter().position(|n| n == "stream_id");
                let n = names.len();
                let mut q = stmt.query([]).unwrap();
                while let Some(row) = q.next().unwrap() {
                    let cells: Vec<String> = (0..n)
                        .map(|i| match row.get_ref(i).unwrap() {
                            ValueRef::Null => "NULL".into(),
                            ValueRef::Integer(v) => v.to_string(),
                            ValueRef::Real(v) => v.to_string(),
                            ValueRef::Text(t) => format!("{:?}", String::from_utf8_lossy(t)),
                            ValueRef::Blob(b) => format!("{b:?}"),
                        })
                        .collect();
                    if let (Some(col), Some(skip)) = (sid_col, without) {
                        if cells[col] == format!("{skip:?}") {
                            continue;
                        }
                    }
                    rows.push(format!("{fi}|{table}|{}", cells.join("|")));
                }
            }
        }
        rows.sort();
        rows
    }

    /// Rows with `stream`'s id in any table.
    fn rows_of(&self, stream: &str) -> usize {
        let all = self.dump(None);
        let rest = self.dump(Some(stream));
        all.len() - rest.len()
    }
}

fn assert_neighbours_identical(f: &Fixture, before: &[String], what: &str) {
    assert_eq!(
        f.dump(Some(&target())),
        before,
        "{what}: everything outside the target must be byte-identical"
    );
}

// ----- dry run, delete, confirm ----------------------------------------------------------------

fn the_full_cycle(two_files: bool) {
    let f = Fixture::seeded(two_files, 40);
    let all_before = f.dump(None);
    let outside_before = f.dump(Some(&target()));

    let dry = f.dry(&target());
    dry.expect(0, "dry run");
    assert_eq!(f.dump(None), all_before, "a dry run deletes nothing");
    let seeded = &f.seeded[&target()];
    assert_eq!(dry.found("ops"), seeded.ops as u64);
    assert_eq!(
        dry.found("registry"),
        (seeded.prefixes + seeded.machines) as u64
    );
    let ops = dry.kind("ops");
    assert_eq!(ops["seq_ranges"], json!([[1, 40]]));
    assert_eq!(ops["max_seq"], 40);
    assert_eq!(ops["value_bytes"], seeded.value_bytes);
    assert_eq!(ops["wall_clock_unparseable"], 1);
    // 21:00:00Z (the +02:00 one) is later than every 20:mm:ssZ.
    assert_eq!(ops["newest_wall_clock"], "2026-10-18T21:00:00Z");
    assert_eq!(
        dry.kind("registry")["by_prefix"],
        json!({"machine_presence": 30, "prefix_claims": 2})
    );
    assert_eq!(
        dry.report["machines"]["newest_last_seen"],
        "2025-10-09T08:53:49Z"
    );
    assert_eq!(
        dry.report["present"],
        json!({"counter": true, "registry": true})
    );
    assert_eq!(dry.report["lease"], Value::Null);
    assert_eq!(dry.report["physical"], Value::Null);
    assert_eq!(dry.report["store"]["backend"], "sqlite");
    assert_eq!(dry.report["store"]["populated"], true);

    let yes = f.yes(&target(), &dry.digest());
    yes.expect(0, "bound --yes");
    for kind in ["ops", "registry"] {
        assert_eq!(
            yes.deleted(kind),
            dry.found(kind),
            "{kind}: deleted == found"
        );
    }
    assert_eq!(yes.report["physical"], "file_level");
    assert_eq!(yes.report["complete"], true);
    assert_eq!(yes.report["residue"], json!([]));
    assert_eq!(f.rows_of(&target()), 0, "no row of the target in any table");
    assert_neighbours_identical(&f, &outside_before, "after --yes");
    f.confirm(&target()).expect(0, "confirm");
}

#[test]
fn one_file_a_dry_run_a_bound_yes_and_a_confirm_remove_the_target_and_only_it() {
    the_full_cycle(false);
}

#[test]
fn two_files_a_dry_run_a_bound_yes_and_a_confirm_remove_the_target_and_only_it() {
    the_full_cycle(true);
}

#[test]
fn the_digest_is_stable_changes_with_a_push_and_ignores_a_heartbeat() {
    let f = Fixture::seeded(false, 12);
    let first = f.dry(&target()).digest();
    assert_eq!(
        f.dry(&target()).digest(),
        first,
        "stable across two dry runs"
    );
    // A heartbeat overwrites a planned machine's last_seen: no key changes, no digest change.
    let reg = f.registry_store();
    let s = StreamId(target());
    let machine = f.seeded[&target()].machine_ids[0].clone();
    reg.announce(
        &s,
        &MachineHello {
            machine_id: machine,
            name: "renamed".into(),
            interval_secs: 60,
        },
        1_900_000_000,
        i64::MIN,
    )
    .unwrap();
    assert_eq!(
        f.dry(&target()).digest(),
        first,
        "a heartbeat is not in the digest"
    );
    f.op_store()
        .append(&s, &op_with_id("pushed-after-the-dry-run"))
        .unwrap();
    assert_ne!(f.dry(&target()).digest(), first, "one more op changes it");
}

fn op_with_id(op_id: &str) -> WireOp {
    WireOp {
        envelope_version: ENVELOPE_VERSION,
        op_id: op_id.into(),
        lamport: 1,
        site: 1,
        domain: "task".into(),
        target_kind: "item".into(),
        target_id: "ab12.9999".into(),
        field: "title".into(),
        op_type: "set".into(),
        value: Some("late".into()),
        author: "seed".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    }
}

#[test]
fn a_yes_after_a_push_since_the_dry_run_is_exit_4_and_deletes_nothing() {
    let f = Fixture::seeded(true, 12);
    let dry = f.dry(&target());
    f.op_store()
        .append(&StreamId(target()), &op_with_id("late-op"))
        .unwrap();
    let before = f.dump(None);
    f.yes(&target(), &dry.digest()).expect(4, "stale plan");
    assert_eq!(f.dump(None), before, "nothing deleted");
}

#[test]
fn a_yes_with_another_streams_digest_is_exit_4_even_when_counts_and_clocks_are_equal() {
    // MF-R4-1: the neighbour has the same number of ops, the same seqs, the same wall clocks and
    // the same registry shape; only its ids differ.
    let f = Fixture::seeded(false, 12);
    let neighbour_digest = f.dry(&neighbour()).digest();
    let target_digest = f.dry(&target()).digest();
    assert_ne!(neighbour_digest, target_digest);
    let before = f.dump(None);
    f.yes(&target(), &neighbour_digest)
        .expect(4, "wrong digest");
    assert_eq!(f.dump(None), before);
}

#[test]
fn the_empty_plans_digest_is_the_same_after_a_purge_and_for_a_never_existing_id() {
    // No stream id in the digest: a digest in a log cannot be rebuilt from a candidate id.
    let f = Fixture::seeded(false, 4);
    let dry = f.dry(&target());
    f.yes(&target(), &dry.digest()).expect(0, "--yes");
    let after = f.dry(&target()).digest();
    let never = f.dry(&format!("n-{H}-404")).digest();
    assert_eq!(after, never);
}

#[test]
fn a_confirm_finds_anything_left_and_a_never_existing_stream_confirms_with_zero_found() {
    let f = Fixture::seeded(false, 5);
    let c = f.confirm(&target());
    c.expect(3, "confirm with the stream still there");
    let never = f.confirm(&format!("n-{H}-404"));
    never.expect(0, "a never-existing stream of a populated store");
    assert!(never.report["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .all(|k| k["found"] == 0));
}

#[test]
fn the_present_field_says_where_the_stream_was_found() {
    let mut f = Fixture::new(false);
    f.seed(&format!("n-{H}-2"), 3, 0);
    // Registry only: a register and an announce, no op.
    let reg = f.registry_store();
    let only_registry = StreamId(format!("n-{H}-3"));
    reg.register(&only_registry, "ef56", "uuid-only").unwrap();
    // A stream with ops only.
    let only_ops = StreamId(format!("n-{H}-4"));
    f.op_store()
        .append(&only_ops, &op_with_id("only-op"))
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

// ----- the store: identity, empty store, not a relay store, unknown tables ------------------------

#[test]
fn an_empty_store_needs_the_identity_pin_and_a_wrong_pin_refuses_every_mode() {
    let f = Fixture::new(false);
    let dry = f.dry(&target());
    dry.expect(78, "empty store without a pin");
    assert_eq!(dry.report["error"]["kind"], "EmptyStore");
    f.confirm(&target()).expect(78, "confirm without a pin");
    let fake = "00".repeat(32);
    f.yes(&target(), &fake).expect(78, "--yes without a pin");
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
    let wrong = "ab".repeat(32);
    for args in [
        vec![target(), "--dry-run".into()],
        vec![target(), "--dry-run".into(), "--expect-empty".into()],
        vec![target(), "--yes".into(), "--plan".into(), fake.clone()],
    ] {
        let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
        args.extend(["--expect-identity", &wrong]);
        let out = f.run(&args, &[]);
        out.expect(78, "wrong pin");
        assert_eq!(out.report["error"]["kind"], "IdentityMismatch");
        assert_eq!(out.report["kinds"], json!([]), "the stream was never read");
    }
}

#[test]
fn a_yes_that_deletes_the_stores_only_stream_exits_0_and_the_confirm_needs_the_pin() {
    let mut f = Fixture::new(false);
    f.seed(&target(), 5, 2);
    let dry = f.dry(&target());
    let pin = dry.identity_digest();
    f.yes(&target(), &dry.digest())
        .expect(0, "delete the only stream");
    f.confirm(&target()).expect(78, "confirm without the pin");
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
}

#[test]
fn what_is_not_a_relay_store_is_78_naming_what_is_missing_and_nothing_is_created() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("nowhere.sqlite");
    let env = |db: &Path, reg: &Path| -> Vec<(&'static str, String)> {
        vec![
            ("NXF_RELAY_BACKEND", "sqlite".into()),
            ("NXF_RELAY_DB", db.to_str().unwrap().into()),
            ("NXF_RELAY_REGISTRY_DB", reg.to_str().unwrap().into()),
        ]
    };
    let run = |env: Vec<(&'static str, String)>| -> Purged {
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        purge(&[&target(), "--dry-run"], &env)
    };
    let out = run(env(&missing, &missing));
    out.expect(78, "missing file");
    assert!(out.report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("NXF_RELAY_DB"));
    assert!(!missing.exists(), "the purge never creates a file");

    // A database without the relay's tables.
    let foreign = dir.path().join("foreign.sqlite");
    Connection::open(&foreign)
        .unwrap()
        .execute_batch("CREATE TABLE other(x TEXT);")
        .unwrap();
    let out = run(env(&foreign, &foreign));
    out.expect(78, "no relay tables");
    assert!(out.report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("stream_ops"));

    // A relay file from before machine_presence (and so before the identity row).
    let old = dir.path().join("old.sqlite");
    Connection::open(&old)
        .unwrap()
        .execute_batch(
            "CREATE TABLE stream_ops(stream_id TEXT, seq INTEGER, op_id TEXT, wall_clock TEXT, value TEXT);
             CREATE TABLE prefix_claims(stream_id TEXT, prefix TEXT, replica_uuid TEXT);",
        )
        .unwrap();
    let out = run(env(&old, &old));
    out.expect(78, "before machine_presence");
    assert!(out.report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("machine_presence"));

    // A current relay file whose identity row was never written (a relay older than this one).
    let f = Fixture::seeded(false, 3);
    Connection::open(&f.ops)
        .unwrap()
        .execute_batch("DELETE FROM relay_store_identity;")
        .unwrap();
    let out = f.dry(&target());
    out.expect(78, "no identity row");
    assert!(out.report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("relay_store_identity"));
    let tables_after: i64 = Connection::open(&old)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM sqlite_master", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tables_after, 2, "the purge created nothing in the old file");
}

#[test]
fn the_identity_follows_the_file_not_the_path() {
    let f = Fixture::seeded(false, 3);
    let first = f.dry(&target());
    let identity = first.report["store"]["identity"].clone();
    assert_eq!(
        identity["ops_path"],
        f.ops.canonicalize().unwrap().to_str().unwrap()
    );
    let digest = first.identity_digest();
    // A byte copy keeps the id: restore the copy at the same path.
    let copy = f.ops.with_extension("copy");
    {
        // Checkpoint so the main file holds everything before copying it.
        let c = Connection::open(&f.ops).unwrap();
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    std::fs::copy(&f.ops, &copy).unwrap();
    std::fs::remove_file(&f.ops).unwrap();
    let _ = std::fs::remove_file(f.ops.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(f.ops.with_extension("sqlite-shm"));
    std::fs::copy(&copy, &f.ops).unwrap();
    assert_eq!(
        f.dry(&target()).identity_digest(),
        digest,
        "a byte copy keeps it"
    );
    // A new file at the same path gets a new id.
    std::fs::remove_file(&f.ops).unwrap();
    let _ = std::fs::remove_file(f.ops.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(f.ops.with_extension("sqlite-shm"));
    SqliteOpStore::open(f.ops.to_str().unwrap()).unwrap();
    SqlitePrefixRegistry::open(f.ops.to_str().unwrap()).unwrap();
    f.op_store()
        .append(&StreamId("other".into()), &op_with_id("x"))
        .unwrap();
    assert_ne!(
        f.dry(&target()).identity_digest(),
        digest,
        "a file re-created at the same path is another store"
    );
}

#[test]
fn an_unknown_table_with_a_target_row_refuses_every_mode_naming_it() {
    for two_files in [false, true] {
        let f = Fixture::seeded(two_files, 5);
        Connection::open(&f.registry)
            .unwrap()
            .execute("CREATE TABLE zzz_extra(stream_id TEXT, v TEXT)", [])
            .unwrap();
        Connection::open(&f.registry)
            .unwrap()
            .execute("INSERT INTO zzz_extra VALUES (?1, 'kept')", [target()])
            .unwrap();
        let before = f.dump(None);
        let dry = f.dry(&target());
        dry.expect(78, "dry run");
        assert_eq!(dry.report["error"]["kind"], "UnknownKind");
        assert!(dry.report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("zzz_extra"));
        f.confirm(&target()).expect(78, "confirm");
        f.yes(&target(), &"00".repeat(32)).expect(78, "--yes");
        assert_eq!(f.dump(None), before, "nothing deleted anywhere");
    }
    // A table name outside the naming pattern is not echoed.
    let f = Fixture::seeded(false, 2);
    Connection::open(&f.ops)
        .unwrap()
        .execute_batch("CREATE TABLE \"Customer-Data\"(stream_id TEXT);")
        .unwrap();
    let dry = f.dry(&target());
    dry.expect(78, "nonconforming table");
    let message = dry.report["error"]["message"].as_str().unwrap();
    assert!(message.contains("<nonconforming, 13 bytes>"), "{message}");
    assert!(!message.contains("Customer"), "{message}");
}

#[test]
fn every_table_of_a_relay_store_is_purged_or_allowed() {
    // The schema test (design §5): a new relay table that keeps a stream's data under another
    // column name would slip past the runtime guard, which keys on `stream_id`; this does not.
    for two_files in [false, true] {
        let f = Fixture::new(two_files);
        let mut tables: Vec<String> = Vec::new();
        for file in f.files() {
            let conn = Connection::open(file).unwrap();
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table'")
                .unwrap();
            tables.extend(
                stmt.query_map([], |r| r.get::<_, String>(0))
                    .unwrap()
                    .map(Result::unwrap),
            );
        }
        tables.sort();
        tables.dedup();
        let mut expected: Vec<String> = nxs_server::purge::SQL_PURGE_TABLES
            .iter()
            .chain(nxs_server::purge::SQL_ALLOWED_TABLES)
            .map(|t| t.to_string())
            .collect();
        expected.sort();
        assert_eq!(tables, expected);
    }
}

// ----- crashes and injections ------------------------------------------------------------------

/// After an interrupted run: a fresh dry run, a bound --yes and a confirm end like an
/// uninterrupted run.
fn finish_and_check(f: &Fixture, outside_before: &[String], what: &str) {
    let dry = f.dry(&target());
    dry.expect(0, &format!("{what}: fresh dry run"));
    f.yes(&target(), &dry.digest())
        .expect(0, &format!("{what}: bound --yes"));
    f.confirm(&target()).expect(0, &format!("{what}: confirm"));
    assert_eq!(f.rows_of(&target()), 0, "{what}");
    assert_neighbours_identical(f, outside_before, what);
}

#[test]
fn a_run_aborted_at_every_sql_point_completes_on_rerun() {
    for (two_files, point) in [
        (false, "S0"),
        (false, "S1-tx"),
        (false, "S3"),
        (false, "V0"),
        (true, "S0"),
        (true, "S1-tx"),
        (true, "S1"),
        (true, "S3"),
        (true, "V0"),
    ] {
        let f = Fixture::seeded(two_files, 8);
        let outside = f.dump(Some(&target()));
        let dry = f.dry(&target());
        let crashed = f.run(
            &[&target(), "--yes", "--plan", &dry.digest()],
            &[("NXF_RELAY_TEST_PURGE_STOP_AFTER", point)],
        );
        assert_ne!(
            crashed.code,
            Some(0),
            "{point}: the abort must actually happen"
        );
        match point {
            "S0" | "S1-tx" => {
                assert_eq!(f.rows_of(&target()), 8 + 32, "{point}: nothing was deleted")
            }
            "S1" => {
                let after = f.dry(&target());
                assert_eq!(
                    (after.found("ops"), after.found("registry")),
                    (0, 32),
                    "{point}"
                );
            }
            _ => assert_eq!(f.rows_of(&target()), 0, "{point}: the delete had committed"),
        }
        finish_and_check(&f, &outside, &format!("two_files={two_files} {point}"));
    }
}

fn inject(f: &Fixture, point_write: &str) -> Purged {
    let dry = f.dry(&target());
    f.run(
        &[&target(), "--yes", "--plan", &dry.digest()],
        &[("NXF_RELAY_TEST_PURGE_INJECT_AT", point_write)],
    )
}

#[test]
fn each_injection_gives_exactly_its_exit_and_residue() {
    for two_files in [false, true] {
        for (point_write, code, residue) in [
            ("S0:op", 4, BTreeMap::new()),
            ("S0:machine", 4, BTreeMap::new()),
            ("S0:prefix", 4, BTreeMap::new()),
            ("V0:op", 3, BTreeMap::from([("ops".to_string(), 1)])),
            (
                "V0:machine",
                3,
                BTreeMap::from([("registry".to_string(), 1)]),
            ),
            (
                "V0:prefix",
                3,
                BTreeMap::from([("registry".to_string(), 1)]),
            ),
        ] {
            let f = Fixture::seeded(two_files, 6);
            let outside = f.dump(Some(&target()));
            let rows_before = f.rows_of(&target());
            let out = inject(&f, point_write);
            out.expect(code, point_write);
            if code == 4 {
                assert_eq!(
                    f.rows_of(&target()),
                    rows_before + 1,
                    "{point_write}: nothing deleted, the injected row stays"
                );
            } else {
                assert_eq!(out.residue(), residue, "{point_write}");
            }
            assert_neighbours_identical(&f, &outside, point_write);
            finish_and_check(&f, &outside, point_write);
        }
    }
}

// ----- physical erasure ------------------------------------------------------------------------

fn markers(f: &Fixture) -> Vec<String> {
    let t = target();
    vec![
        f.marker(&t, "title"),
        f.marker(&t, "body"),
        f.marker(&t, "deleted"),
        f.marker(&t, "machine"),
    ]
}

fn wal(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}-wal", path.display()))
}

fn any_marker_in(f: &Fixture, files: &[PathBuf]) -> Vec<String> {
    let mut found = Vec::new();
    for m in markers(f) {
        for file in files {
            if file_contains(file, &m) {
                found.push(format!("{m} in {}", file.display()));
            }
        }
    }
    found
}

fn main_and_wal(f: &Fixture) -> Vec<PathBuf> {
    f.files().iter().flat_map(|p| [p.clone(), wal(p)]).collect()
}

#[test]
fn after_yes_no_marker_is_left_in_the_main_file_or_the_wal() {
    for two_files in [false, true] {
        let f = Fixture::seeded(two_files, 30);
        assert!(
            !any_marker_in(&f, &main_and_wal(&f)).is_empty(),
            "the markers are on disk before the purge"
        );
        let dry = f.dry(&target());
        let yes = f.yes(&target(), &dry.digest());
        yes.expect(0, "--yes");
        assert_eq!(yes.report["physical"], "file_level");
        let left = any_marker_in(&f, &main_and_wal(&f));
        assert!(left.is_empty(), "markers left: {left:?}");
        // The neighbours' markers are still there: the byte search can find a marker.
        let n = neighbour();
        assert!(
            f.files()
                .iter()
                .any(|p| file_contains(p, &f.marker(&n, "title")))
                || main_and_wal(&f)
                    .iter()
                    .any(|p| file_contains(p, &f.marker(&n, "title")))
        );
    }
}

#[test]
fn with_secure_delete_off_the_same_delete_leaves_markers_behind() {
    // MF-2: the control is made by the TEST, not by a seam in the shipped code — a test-side
    // connection deletes the marker rows with secure_delete off and checkpoints; the byte search
    // must then find a marker, which proves the search can fail.
    let f = Fixture::seeded(false, 30);
    let conn = Connection::open(&f.ops).unwrap();
    conn.execute_batch("PRAGMA secure_delete = OFF;").unwrap();
    conn.execute("DELETE FROM stream_ops WHERE stream_id = ?1", [target()])
        .unwrap();
    conn.execute(
        "DELETE FROM machine_presence WHERE stream_id = ?1",
        [target()],
    )
    .unwrap();
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .unwrap();
    drop(conn);
    let left = any_marker_in(&f, &main_and_wal(&f));
    assert!(
        !left.is_empty(),
        "with secure_delete off the bytes stay in the file"
    );
}

#[test]
fn a_reader_holding_a_snapshot_makes_yes_exit_5_pending_until_a_later_yes() {
    let f = Fixture::seeded(false, 30);
    let dry = f.dry(&target());
    // A test-side reader holds a read transaction open across the --yes.
    let reader = Connection::open(&f.ops).unwrap();
    reader.execute_batch("BEGIN").unwrap();
    let _: i64 = reader
        .query_row("SELECT COUNT(*) FROM stream_ops", [], |r| r.get(0))
        .unwrap();
    let yes = f.yes(&target(), &dry.digest());
    yes.expect(5, "checkpoint busy");
    assert_eq!(yes.report["complete"], true);
    assert_eq!(yes.report["physical"], "pending");
    assert_eq!(yes.report["residue"], json!([]));
    let in_main: Vec<String> = markers(&f)
        .into_iter()
        .filter(|m| file_contains(&f.ops, m))
        .collect();
    assert!(!in_main.is_empty(), "the marker is still in the main file");
    reader.execute_batch("COMMIT").unwrap();
    drop(reader);
    let dry = f.dry(&target());
    assert_eq!(dry.found("ops"), 0, "the plan is empty now");
    let again = f.run(
        &[
            &target(),
            "--yes",
            "--plan",
            &dry.digest(),
            "--expect-identity",
            &dry.identity_digest(),
        ],
        &[],
    );
    again.expect(0, "a later --yes checkpoints");
    assert_eq!(again.report["physical"], "file_level");
    let left = any_marker_in(&f, &main_and_wal(&f));
    assert!(left.is_empty(), "{left:?}");
}

fn checksum_of(bytes: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).to_vec()
}

fn checksum(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok().map(|b| checksum_of(&b))
}

#[test]
fn dry_runs_and_confirms_leave_a_stopped_relays_files_byte_identical() {
    // MF-3: run against the files a relay left behind when it stopped (its WAL not checkpointed),
    // a dry run and a confirm must not checkpoint, truncate or delete anything.
    for two_files in [false, true] {
        let f = Fixture::seeded(two_files, 20);
        // Leave a non-empty WAL behind: write through a connection that never checkpoints and is
        // abandoned the way a killed process abandons it.
        let conn = Connection::open(&f.ops).unwrap();
        conn.execute_batch("PRAGMA wal_autocheckpoint = 0;")
            .unwrap();
        conn.set_db_config(
            rusqlite::config::DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE,
            true,
        )
        .unwrap();
        conn.execute(
            "INSERT INTO stream_ops(stream_id, seq, envelope_version, op_id, lamport, site,
             domain, target_kind, target_id, field, op_type, value, author, wall_clock)
             VALUES (?1, 999, 1, 'in-the-wal', 1, 1, 'task', 'item', 'x', 'title', 'set', 'w',
             'seed', '')",
            [neighbour()],
        )
        .unwrap();
        drop(conn);
        let files: Vec<PathBuf> = f
            .files()
            .iter()
            .flat_map(|p| {
                [
                    p.clone(),
                    wal(p),
                    PathBuf::from(format!("{}-shm", p.display())),
                ]
            })
            .collect();
        let before: Vec<_> = files.iter().map(|p| checksum(p)).collect();
        assert!(std::fs::metadata(&files[1]).is_ok_and(|m| m.len() > 0));
        f.dry(&target()).expect(0, "dry run");
        f.confirm(&format!("n-{H}-404")).expect(0, "confirm");
        let after: Vec<_> = files.iter().map(|p| checksum(p)).collect();
        // Main file and WAL byte-identical: no checkpoint moved a page, nothing truncated or
        // deleted the WAL. The `-shm` is the WAL index, not data: every reader writes its read
        // mark into it, so it is only required to still be there (note on 6j6v.f6yb, MF-3).
        // A cleanly closed file has no WAL; opening it to read creates an empty one, which holds
        // nothing (second note on 6j6v.f6yb, MF-3).
        let empty = checksum_of(&[]);
        for (i, ((p, b), a)) in files.iter().zip(&before).zip(&after).enumerate() {
            match (i % 3, b) {
                (2, Some(_)) => assert!(a.is_some(), "{} was deleted", p.display()),
                (2, None) => {}
                (1, None) => assert!(
                    a.is_none() || a.as_ref() == Some(&empty),
                    "{} appeared with content",
                    p.display()
                ),
                _ => assert_eq!(b, a, "{} changed", p.display()),
            }
        }
        assert!(
            before[1].is_some(),
            "the case that matters: a WAL with content was there"
        );
    }
}

// ----- the report and the command line ---------------------------------------------------------

#[test]
fn the_reports_sid_is_the_log_hash_token_and_no_id_reaches_the_output() {
    let f = Fixture::seeded(true, 10);
    let dry = f.dry(&target());
    let hash = std::process::Command::new(assert_cmd::cargo::cargo_bin("nxf-relay"))
        .args(["log-hash", "sid", &target()])
        .env("NXS_LOG_HMAC_KEY", C9_KEY)
        .output()
        .unwrap();
    let token = String::from_utf8(hash.stdout).unwrap();
    assert_eq!(
        format!("sid={}\n", dry.report["sid"].as_str().unwrap()),
        token,
        "the report's sid is log-hash's token after its `sid=` prefix (C8)"
    );
    assert!(dry
        .stderr
        .lines()
        .all(|l| l.contains(&format!("sid={}", dry.report["sid"].as_str().unwrap()))));
    let yes = f.yes(&target(), &dry.digest());
    let confirm = f.confirm(&target());
    let seeded = &f.seeded[&target()];
    let mut forbidden: Vec<String> = vec![target(), neighbour(), prefix_neighbour()];
    forbidden.extend(seeded.op_ids.iter().cloned());
    forbidden.extend(seeded.machine_ids.iter().cloned());
    forbidden.extend(seeded.replica_uuids.iter().cloned());
    forbidden.extend(markers(&f));
    for out in [&dry, &yes, &confirm] {
        for needle in &forbidden {
            assert!(!out.stdout.contains(needle), "{needle} in stdout");
            assert!(!out.stderr.contains(needle), "{needle} in stderr");
        }
        for prefix in &seeded.prefix_values {
            assert!(
                !out.stdout.contains(&format!("\"{prefix}\"")),
                "prefix in stdout"
            );
        }
    }
}

#[test]
fn without_a_key_the_sid_is_a_dash_and_stderr_says_so_once() {
    let f = Fixture::seeded(false, 3);
    let out = f.run(&[&target(), "--dry-run"], &[("NXS_LOG_HMAC_KEY", "")]);
    // An empty variable is malformed (C1), not "unset".
    out.expect(78, "an empty key");
    let mut env = f.env();
    env.retain(|(k, _)| *k != "NXS_LOG_HMAC_KEY");
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let out = purge(&[&target(), "--dry-run"], &env);
    out.expect(0, "no key");
    assert_eq!(out.report["sid"], "-");
    assert_eq!(
        out.stderr.matches("log ids are off").count(),
        1,
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains(&target()));
}

#[test]
fn every_c1_refusal_of_the_key_is_78_in_every_mode_and_never_echoed() {
    let f = Fixture::seeded(false, 3);
    let good = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
    let bad = [
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
    let fake = "00".repeat(32);
    for value in &bad {
        for args in [
            vec![target(), "--dry-run".to_string()],
            vec![
                target(),
                "--dry-run".to_string(),
                "--expect-empty".to_string(),
            ],
            vec![
                target(),
                "--yes".to_string(),
                "--plan".to_string(),
                fake.clone(),
            ],
        ] {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = f.run(&args, &[("NXS_LOG_HMAC_KEY", value)]);
            out.expect(78, value);
            assert!(!out.stdout.contains(value.as_str()) && !out.stderr.contains(value.as_str()));
            assert!(!out.stdout.contains(&good[..20]) && !out.stderr.contains(&good[..20]));
        }
    }
}

#[test]
fn every_refused_id_is_64_with_its_rule_and_reads_nothing() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("never-opened.sqlite");
    let env = [
        ("NXF_RELAY_BACKEND", "sqlite"),
        ("NXF_RELAY_DB", missing.to_str().unwrap()),
        ("NXF_RELAY_REGISTRY_DB", missing.to_str().unwrap()),
    ];
    for (id, rule) in [
        ("".to_string(), "empty"),
        ("#due".to_string(), "leading_hash"),
        ("a/b".to_string(), "slash"),
        ("a\u{1}b".to_string(), "control"),
        ("x".repeat(1_001), "too_long"),
    ] {
        let out = purge(&["--dry-run", "--", &id], &env);
        out.expect(64, rule);
        assert_eq!(out.report["refused"]["rule"], rule);
    }
    // 1,000 bytes is accepted: the run reaches the store (and refuses it as missing, 78).
    let out = purge(&["--dry-run", "--", &"x".repeat(1_000)], &env);
    out.expect(78, "1,000 bytes passes validation");
    assert!(!missing.exists());
}

#[test]
fn every_mode_flag_combination_outside_the_three_is_64_and_reads_nothing() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("never-opened.sqlite");
    let env = [
        ("NXF_RELAY_BACKEND", "sqlite"),
        ("NXF_RELAY_DB", missing.to_str().unwrap()),
        ("NXF_RELAY_REGISTRY_DB", missing.to_str().unwrap()),
    ];
    let d = "ab".repeat(32);
    let t = target();
    for args in [
        vec![t.as_str()],
        vec![t.as_str(), "--dry-run", "--yes"],
        vec![t.as_str(), "--yes"],
        vec![t.as_str(), "--plan", &d],
        vec![t.as_str(), "--dry-run", "--plan", &d],
        vec![t.as_str(), "--yes", "--plan", &d, "--expect-empty"],
        vec![t.as_str(), "--dry-run", "--keys"],
        vec![t.as_str(), "--yes", "--plan", &d, "--keys"],
        vec![t.as_str(), "--expect-empty"],
    ] {
        let out = purge(&args, &env);
        out.expect(64, &format!("{args:?}"));
    }
    assert!(!missing.exists());
}

#[test]
fn the_backend_variables_have_no_defaults() {
    let f = Fixture::seeded(false, 2);
    let ops = f.ops.to_str().unwrap().to_string();
    for (env, missing) in [
        (
            vec![
                ("NXF_RELAY_DB", ops.as_str()),
                ("NXF_RELAY_REGISTRY_DB", ops.as_str()),
            ],
            "NXF_RELAY_BACKEND",
        ),
        (
            vec![
                ("NXF_RELAY_BACKEND", "sqlite"),
                ("NXF_RELAY_REGISTRY_DB", ops.as_str()),
            ],
            "NXF_RELAY_DB",
        ),
        (
            vec![
                ("NXF_RELAY_BACKEND", "sqlite"),
                ("NXF_RELAY_DB", ops.as_str()),
            ],
            "NXF_RELAY_REGISTRY_DB",
        ),
    ] {
        let out = purge(&[&target(), "--dry-run"], &env);
        out.expect(78, missing);
        assert!(out.report["error"]["message"]
            .as_str()
            .unwrap()
            .contains(missing));
    }
}

#[test]
fn the_seams_reach_the_test_built_binary() {
    // The other suites rely on it; a binary built without the seams would turn every crash and
    // injection case vacuous rather than red.
    let f = Fixture::seeded(false, 3);
    let dry = f.dry(&target());
    let out = f.run(
        &[&target(), "--yes", "--plan", &dry.digest()],
        &[("NXF_RELAY_TEST_PURGE_FAIL_AT", "V0")],
    );
    out.expect(1, "a failure injected at V0");
}

// ----- the lock time ----------------------------------------------------------------------------

/// Seed `n` ops into one stream fast (one transaction, the server's schema), and time the delete
/// step of a bound --yes — its re-read and its checkpoint included — from the progress lines'
/// clock. Prints the numbers the runbook carries.
fn measure(n: usize) -> std::time::Duration {
    let f = Fixture::new(false);
    {
        let conn = Connection::open(&f.ops).unwrap();
        conn.execute_batch("BEGIN").unwrap();
        let mut stmt = conn
            .prepare(
                "INSERT INTO stream_ops(stream_id, seq, envelope_version, op_id, lamport, site,
                 domain, target_kind, target_id, field, op_type, value, author, wall_clock)
                 VALUES (?1, ?2, 1, ?3, ?2, 1, 'task', 'item', 'ab12.0001', 'title', 'set', ?4,
                 'seed', '2026-10-18T20:00:00Z')",
            )
            .unwrap();
        let value = "v".repeat(400);
        for i in 1..=n {
            stmt.execute(rusqlite::params![
                target(),
                i as i64,
                format!("op-{i}"),
                value
            ])
            .unwrap();
        }
        drop(stmt);
        conn.execute_batch("COMMIT").unwrap();
    }
    let dry = f.dry(&target());
    assert_eq!(
        dry.report["large"],
        n as u64 > nxs_server::purge::SQLITE_LARGE_OPS
    );
    let yes = f.yes(&target(), &dry.digest());
    yes.expect(0, "--yes");
    let field = |name: &str| -> u64 {
        yes.stderr
            .split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{name}=")))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("no {name} in {}", yes.stderr))
    };
    let (lock, checkpoint) = (field("lock_ms"), field("checkpoint_ms"));
    eprintln!("{n} ops: write lock {lock} ms, checkpoint {checkpoint} ms");
    std::time::Duration::from_millis(lock + checkpoint)
}

#[test]
fn the_delete_step_of_5000_ops_holds_the_lock_well_under_busy_timeout() {
    let took = measure(5_000);
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
}

#[test]
#[ignore = "a measurement for the runbook: cargo test --release -p nxs-server --test purge_sqlite -- --ignored --nocapture"]
fn the_delete_step_of_100000_ops() {
    let took = measure(100_000);
    eprintln!("100,000 ops: lock and checkpoint together {took:?}");
}
