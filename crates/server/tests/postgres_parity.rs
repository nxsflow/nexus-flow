//! Differential parity: the durable Postgres backend must be observably identical to the
//! SQLite impl that proved convergence (acceptance for nexus-flow-8ho.8 / .9). The tests
//! run the SAME scripts against both backends through the trait and assert the traces are
//! equal value-for-value, then hammer each backend's concurrency invariant on its own
//! (the "atomic monotone seq" / "claim-if-free" risk that the parity scripts can't show
//! because they're deterministic-sequential).
//!
//! ## Test harness decision (the one genuinely new choice in Slice 3)
//!
//! The in-process SQLite tests need no external state; a real Postgres does. We gate these
//! behind BOTH the `postgres` cargo feature AND a `DATABASE_URL` env var:
//!
//!   * No Docker dependency in the test run (testcontainers would force one, and isn't even
//!     usable on a dev box without a Docker daemon). The same `DATABASE_URL` contract works
//!     locally (a throwaway cluster) and in CI (a Postgres service container).
//!   * `cargo test` stays green everywhere with zero setup: without the feature the file is
//!     empty; with the feature but no `DATABASE_URL` every test prints a loud SKIP and
//!     passes — **except under CI** (`CI` env set), where a missing `DATABASE_URL` is a hard
//!     failure (see `base_url`), so a dropped database can never pass green.
//!   * Per-test isolation is a unique Postgres SCHEMA (search_path), so the tests run in
//!     parallel without interfering and leave the database reusable.
#![cfg(feature = "postgres")]

use std::sync::Arc;
use std::thread;

use nxs_server::presence::PresenceStore;
use nxs_server::registry::{PrefixRegistry, SqlitePrefixRegistry};
use nxs_server::registry_pg::PostgresPrefixRegistry;
use nxs_server::store::{OpStore, SqliteOpStore};
use nxs_server::store_pg::{PostgresOpStore, OPSTORE_LOCK_CLASS, REGISTRY_LOCK_CLASS};
use nxs_sync::protocol::{Cursor, MachineHello, RegisterOutcome, StreamId};

mod common;

use common::{base_url, isolated_url, wire, wire_from_a_newer_client};

/// Each backend under test gets its own schema, so the parity scripts never see another
/// test's rows.
fn pg_opstore(base: &str) -> PostgresOpStore {
    PostgresOpStore::connect(&isolated_url(base, "parity")).expect("connect postgres op store")
}

fn pg_registry(base: &str) -> PostgresPrefixRegistry {
    PostgresPrefixRegistry::connect(&isolated_url(base, "parity"))
        .expect("connect postgres registry")
}

// ----- OpStore differential parity ---------------------------------------------------

/// One deterministic script run against an `OpStore`, returning every observable output:
/// each append's cursor, then a full paginated read of each stream. Two backends that
/// produce equal traces are observably identical.
fn opstore_trace(store: &dyn OpStore) -> Vec<String> {
    let a = StreamId("stream-a".into());
    let b = StreamId("stream-b".into());
    let mut trace = Vec::new();

    // Interleave two streams; re-append one op (idempotent union, must not gap the seq).
    for (stream, id) in [
        (&a, "a1"),
        (&a, "a2"),
        (&b, "b1"),
        (&a, "a1"), // duplicate → existing cursor, no new seq
        (&a, "a3"),
        (&b, "b2"),
    ] {
        let c = store.append(stream, &wire(id)).unwrap();
        trace.push(format!("append {} {id} -> {}", stream.as_str(), c.0));
    }

    // Paginate each stream two ops at a time, following next-cursor until exhausted.
    for stream in [&a, &b] {
        let mut cursor = Cursor::BEGINNING;
        loop {
            let (ops, next) = store.read_since(stream, cursor, 2).unwrap();
            if ops.is_empty() {
                trace.push(format!(
                    "read {} @{} -> empty next={}",
                    stream.as_str(),
                    cursor.0,
                    next.0
                ));
                break;
            }
            let ids: Vec<&str> = ops.iter().map(|o| o.op_id.as_str()).collect();
            trace.push(format!(
                "read {} @{} -> [{}] next={}",
                stream.as_str(),
                cursor.0,
                ids.join(","),
                next.0
            ));
            cursor = next;
        }
    }
    trace
}

#[test]
fn opstore_parity_postgres_matches_sqlite() {
    let Some(base) = base_url("opstore_parity") else {
        return;
    };
    let sqlite_trace = opstore_trace(&SqliteOpStore::open_in_memory().unwrap());
    let pg_trace = opstore_trace(&pg_opstore(&base));
    assert_eq!(
        sqlite_trace, pg_trace,
        "Postgres op store must be observably identical to SQLite"
    );
}

#[test]
fn opstore_preserves_envelope_verbatim_postgres() {
    // Server opacity (§4.1): a future op (unknown op_type/target_kind, non-default
    // envelope_version, null value) round-trips through Postgres unchanged.
    let Some(base) = base_url("opstore_envelope") else {
        return;
    };
    let store = pg_opstore(&base);
    let s = StreamId("s".into());
    let mut op = wire("o1");
    op.envelope_version = 999;
    op.op_type = "future_set".into();
    op.target_kind = "gizmo".into();
    op.value = None;
    store.append(&s, &op).unwrap();
    let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(ops, vec![op]);
}

#[test]
fn an_unknown_envelope_field_survives_the_relay_postgres() {
    // 6j6v.5crb, per backend: the DoD is "belegt fuer alle drei Backends", because the drop was
    // never in the envelope alone — each backend decomposes the op into its own columns and
    // rebuilds it from them. SQLite proves the same thing in `store.rs`; DynamoDB in
    // `dynamodb_local.rs`. Without all three, a `sig` would survive one deployment and vanish on
    // another, which is worse than failing everywhere.
    let Some(base) = base_url("opstore_unknown_field") else {
        return;
    };
    let store = pg_opstore(&base);
    let s = StreamId("s".into());
    let op = wire_from_a_newer_client("future-1");
    store.append(&s, &op).unwrap();
    let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(
        ops,
        vec![op],
        "Postgres carries a field this build does not know, verbatim"
    );
    assert_eq!(
        ops[0].extra.get("sig").and_then(|v| v.as_str()),
        Some("ed25519:deadbeef")
    );
}

#[test]
fn a_relay_database_written_before_the_passthrough_column_upgrades_in_place_postgres() {
    // `CREATE TABLE IF NOT EXISTS` leaves an existing table alone, so a database written by a
    // build that predates `extra` would keep serving without the column and fail every INSERT.
    // Postgres is where the deployed relays actually keep their log, so the upgrade is proven
    // here against a real one, not only on SQLite.
    let Some(base) = base_url("opstore_upgrade") else {
        return;
    };
    let url = isolated_url(&base, "upgrade");
    let mut admin = postgres::Client::connect(&url, nxs_server::tls_pg::make_tls().expect("tls"))
        .expect("connect to seed the pre-upgrade schema");
    admin
        .batch_execute(
            "CREATE TABLE stream_ops(
                 stream_id        TEXT    NOT NULL,
                 seq              BIGINT  NOT NULL,
                 envelope_version INTEGER NOT NULL,
                 op_id            TEXT    NOT NULL,
                 lamport          BIGINT  NOT NULL,
                 site             BIGINT  NOT NULL,
                 domain           TEXT    NOT NULL DEFAULT 'task',
                 target_kind      TEXT    NOT NULL,
                 target_id        TEXT    NOT NULL,
                 field            TEXT    NOT NULL,
                 op_type          TEXT    NOT NULL,
                 value            TEXT,
                 author           TEXT    NOT NULL,
                 wall_clock       TEXT    NOT NULL,
                 PRIMARY KEY (stream_id, seq),
                 UNIQUE (stream_id, op_id)
             );
             INSERT INTO stream_ops VALUES
                 ('s',1,1,'old-1',1,1,'task','item','ab12.0001','title','set','t','a','');",
        )
        .expect("seed the pre-upgrade table");
    drop(admin);

    let store = PostgresOpStore::connect(&url).expect("connect upgrades the schema in place");
    let s = StreamId("s".into());
    let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(ops.len(), 1, "the pre-existing op is still served");
    assert!(
        ops[0].extra.is_empty(),
        "a row written before the column reads as 'carried nothing unknown'"
    );

    store
        .append(&s, &wire_from_a_newer_client("future-1"))
        .unwrap();
    let (ops, _) = store.read_since(&s, Cursor(1), 10).unwrap();
    assert_eq!(ops, vec![wire_from_a_newer_client("future-1")]);

    // Connecting AGAIN to an already-upgraded database is a no-op, not a duplicate-column error.
    PostgresOpStore::connect(&url).expect("re-connect is idempotent");
}

#[test]
fn opstore_concurrent_appends_get_unique_contiguous_sequences_postgres() {
    // THE named risk: atomic, per-stream, monotone seq under concurrent writers — here on
    // real, independent pooled connections (no client-side mutex hiding the question).
    let Some(base) = base_url("opstore_concurrency") else {
        return;
    };
    let store = Arc::new(pg_opstore(&base));
    let s = StreamId("s".into());
    const THREADS: i64 = 8;
    const PER: i64 = 50;

    let mut handles = Vec::new();
    for t in 0..THREADS {
        let store = Arc::clone(&store);
        let s = s.clone();
        handles.push(thread::spawn(move || {
            (0..PER)
                .map(|i| store.append(&s, &wire(&format!("o{t}-{i}"))).unwrap().0)
                .collect::<Vec<_>>()
        }));
    }
    let mut all: Vec<i64> = handles
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();
    all.sort_unstable();
    let expected: Vec<i64> = (1..=THREADS * PER).collect();
    assert_eq!(
        all, expected,
        "sequences are unique and contiguous under contention"
    );
}

// ----- PrefixRegistry differential parity --------------------------------------------

/// Deterministic register script; returns each outcome as a stable string. Both backends
/// reuse `candidate_prefix`, so the reassigned prefixes are equal — not just both "some
/// reassignment".
fn registry_trace(reg: &dyn PrefixRegistry) -> Vec<String> {
    let s = StreamId("s".into());
    let show = |o: RegisterOutcome| match o {
        RegisterOutcome::Registered => "registered".to_string(),
        RegisterOutcome::Reassigned { new_prefix } => format!("reassigned:{new_prefix}"),
    };
    let mut trace = Vec::new();
    for (prefix, uuid) in [
        ("aaaa", "uuid-A"),
        ("aaaa", "uuid-B"), // collides → reassigned
        ("aaaa", "uuid-C"), // collides → reassigned (distinct from B)
        ("aaaa", "uuid-A"), // idempotent → registered
        ("aaaa", "uuid-B"), // idempotent → same reassignment
        ("bbbb", "uuid-A"), // A already owns aaaa → reassigned to its existing prefix
    ] {
        trace.push(format!(
            "{prefix} {uuid} -> {}",
            show(reg.register(&s, prefix, uuid).unwrap())
        ));
    }
    trace
}

#[test]
fn registry_parity_postgres_matches_sqlite() {
    let Some(base) = base_url("registry_parity") else {
        return;
    };
    let sqlite_trace = registry_trace(&SqlitePrefixRegistry::open_in_memory().unwrap());
    let pg_trace = registry_trace(&pg_registry(&base));
    assert_eq!(
        sqlite_trace, pg_trace,
        "Postgres registry must be observably identical to SQLite"
    );
}

/// Deterministic presence script (6j6v.f0b5); returns every read as a stable string. The SAME
/// script is copied byte for byte into `dynamodb_local.rs::presence_trace` — separate test
/// binaries, no shared crate — so all three backends are held to one trace.
///
/// It covers what a reader can observe: insert, overwrite (name, cadence and time move, the row
/// count does not), per-stream isolation, the order (most recent first, ties by id in BYTE order —
/// `Mb` sorts before `ma`, which a locale collation would reverse) and the limit.
fn presence_trace(store: &dyn PresenceStore) -> Vec<String> {
    let a = StreamId("stream-a".into());
    let b = StreamId("stream-b".into());
    let hello = |id: &str, name: &str, interval_secs| MachineHello {
        machine_id: id.into(),
        name: name.into(),
        interval_secs,
    };
    fn read(
        store: &dyn PresenceStore,
        stream: &StreamId,
        since: i64,
        limit: usize,
        trace: &mut Vec<String>,
    ) {
        let page = store.machines(stream, since, limit).unwrap();
        let rows: Vec<String> = page
            .records
            .iter()
            .map(|r| {
                format!(
                    "{}={}@{}/{}",
                    r.machine_id, r.name, r.last_seen, r.interval_secs
                )
            })
            .collect();
        let more = if page.truncated { " +more" } else { "" };
        trace.push(format!(
            "{} since {since} limit {limit}: [{}]{more}",
            stream.as_str(),
            rows.join(", ")
        ));
    }
    let mut trace = Vec::new();
    store
        .announce(&a, &hello("ma", "MacBook", 300), 100, 0)
        .unwrap();
    store
        .announce(&a, &hello("Mb", "Mac mini", 300), 100, 0)
        .unwrap();
    store
        .announce(&a, &hello("z9", "Studio", 20), 50, 0)
        .unwrap();
    store
        .announce(&b, &hello("ma", "MacBook", 300), 70, 0)
        .unwrap();
    read(store, &a, 0, 100, &mut trace);
    read(store, &a, 0, 2, &mut trace);
    read(store, &b, 0, 100, &mut trace);
    // A machine last seen before the window is not listed.
    read(store, &a, 60, 100, &mut trace);
    // The same machine again: one row, everything it said replaced.
    store
        .announce(&a, &hello("z9", "Studio (renamed)", 600), 400, 0)
        .unwrap();
    read(store, &a, 0, 100, &mut trace);
    // An announcement that forgets what is past the window (SQLite and Postgres delete it, DynamoDB
    // filters it) — read within the window, which is what every backend answers alike.
    store
        .announce(&a, &hello("new", "Workstation", 300), 500, 150)
        .unwrap();
    read(store, &a, 150, 100, &mut trace);
    read(store, &StreamId("stream-none".into()), 0, 100, &mut trace);
    trace
}

const EXPECTED_PRESENCE_TRACE: [&str; 7] = [
    "stream-a since 0 limit 100: [Mb=Mac mini@100/300, ma=MacBook@100/300, z9=Studio@50/20]",
    "stream-a since 0 limit 2: [Mb=Mac mini@100/300, ma=MacBook@100/300] +more",
    "stream-b since 0 limit 100: [ma=MacBook@70/300]",
    "stream-a since 60 limit 100: [Mb=Mac mini@100/300, ma=MacBook@100/300]",
    "stream-a since 0 limit 100: [z9=Studio (renamed)@400/600, Mb=Mac mini@100/300, ma=MacBook@100/300]",
    "stream-a since 150 limit 100: [new=Workstation@500/300, z9=Studio (renamed)@400/600]",
    "stream-none since 0 limit 100: []",
];

#[test]
fn presence_parity_postgres_matches_sqlite() {
    let Some(base) = base_url("presence_parity") else {
        return;
    };
    let sqlite_trace = presence_trace(&SqlitePrefixRegistry::open_in_memory().unwrap());
    // Pinned, so two backends that were wrong the same way could not agree their way to green.
    assert_eq!(sqlite_trace, EXPECTED_PRESENCE_TRACE);
    let pg_trace = presence_trace(&pg_registry(&base));
    assert_eq!(
        sqlite_trace, pg_trace,
        "Postgres presence must be observably identical to SQLite"
    );
}

fn hello_from(id: &str) -> MachineHello {
    MachineHello {
        machine_id: id.into(),
        name: "MacBook".into(),
        interval_secs: 300,
    }
}

#[test]
fn presence_rows_leave_the_prefix_claims_alone_postgres() {
    // The two live in one database, one struct: an announcement must not look like a claim, and a
    // claim must not look like a machine — a guard for the day somebody folds the tables together.
    let Some(base) = base_url("presence_beside_claims") else {
        return;
    };
    let reg = pg_registry(&base);
    let s = StreamId("s".into());
    reg.announce(&s, &hello_from("uuid-A"), 1, 0).unwrap();
    assert_eq!(
        reg.register(&s, "aaaa", "uuid-A").unwrap(),
        RegisterOutcome::Registered
    );
    assert_eq!(reg.machines(&s, 0, 100).unwrap().records.len(), 1);
}

#[test]
fn an_announcement_forgets_the_stream_s_machines_past_the_window_postgres() {
    // Read with no window at all, so what is observed is the TABLE: the expired row is gone, the
    // other stream's untouched.
    let Some(base) = base_url("presence_prune") else {
        return;
    };
    let reg = pg_registry(&base);
    let (a, b) = (StreamId("a".into()), StreamId("b".into()));
    reg.announce(&a, &hello_from("ghost"), 100, 0).unwrap();
    reg.announce(&b, &hello_from("elsewhere"), 100, 0).unwrap();
    reg.announce(&a, &hello_from("live"), 1_000, 500).unwrap();
    let ids = |s: &StreamId| -> Vec<String> {
        reg.machines(s, i64::MIN, 100)
            .unwrap()
            .records
            .into_iter()
            .map(|r| r.machine_id)
            .collect()
    };
    assert_eq!(ids(&a), ["live"]);
    assert_eq!(ids(&b), ["elsewhere"]);
}

#[test]
fn a_relay_database_from_before_presence_gains_the_table_and_keeps_its_claims_postgres() {
    // manufakt.io's upgrade path, against a real Postgres: a database that has only
    // `prefix_claims` is opened by a relay that knows presence.
    let Some(base) = base_url("presence_upgrade") else {
        return;
    };
    let url = isolated_url(&base, "presence_upgrade");
    let mut admin = postgres::Client::connect(&url, nxs_server::tls_pg::make_tls().expect("tls"))
        .expect("connect to seed the pre-presence schema");
    admin
        .batch_execute(
            "CREATE TABLE prefix_claims(
                 stream_id    TEXT NOT NULL,
                 prefix       TEXT NOT NULL,
                 replica_uuid TEXT NOT NULL,
                 PRIMARY KEY (stream_id, prefix),
                 UNIQUE (stream_id, replica_uuid)
             );
             INSERT INTO prefix_claims VALUES ('s', 'aaaa', 'uuid-A');",
        )
        .expect("seed the pre-presence table");
    drop(admin);

    let reg = PostgresPrefixRegistry::connect(&url).expect("connect upgrades the schema in place");
    let s = StreamId("s".into());
    assert_eq!(
        reg.register(&s, "bbbb", "uuid-A").unwrap(),
        RegisterOutcome::Reassigned {
            new_prefix: "aaaa".into()
        },
        "the claim made before the upgrade is still the replica's"
    );
    reg.announce(&s, &hello_from("m1"), 1, 0).unwrap();
    assert_eq!(reg.machines(&s, 0, 100).unwrap().records.len(), 1);
}

#[test]
fn registry_claims_are_isolated_per_stream_postgres() {
    let Some(base) = base_url("registry_isolation") else {
        return;
    };
    let reg = pg_registry(&base);
    reg.register(&StreamId("a".into()), "aaaa", "uuid-A")
        .unwrap();
    // The same prefix is free in a different stream → Registered, not Reassigned.
    assert_eq!(
        reg.register(&StreamId("b".into()), "aaaa", "uuid-B")
            .unwrap(),
        RegisterOutcome::Registered
    );
}

#[test]
fn registry_concurrent_collision_grants_exactly_one_postgres() {
    // claim-if-free under contention: N replicas race on ONE prefix; exactly one keeps it
    // (Registered), the rest Reassigned, and every resulting prefix is distinct (no
    // aliasing).
    let Some(base) = base_url("registry_concurrency") else {
        return;
    };
    let reg = Arc::new(pg_registry(&base));
    let s = StreamId("s".into());
    const REPLICAS: usize = 8;

    let mut handles = Vec::new();
    for r in 0..REPLICAS {
        let reg = Arc::clone(&reg);
        let s = s.clone();
        handles.push(thread::spawn(move || {
            match reg.register(&s, "aaaa", &format!("uuid-{r}")).unwrap() {
                RegisterOutcome::Registered => "aaaa".to_string(),
                RegisterOutcome::Reassigned { new_prefix } => new_prefix,
            }
        }));
    }
    let prefixes: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let registered = prefixes.iter().filter(|p| p.as_str() == "aaaa").count();
    assert_eq!(
        registered, 1,
        "exactly one replica keeps (Registered) the contested prefix"
    );
    let distinct: std::collections::HashSet<&String> = prefixes.iter().collect();
    assert_eq!(
        distinct.len(),
        REPLICAS,
        "every replica ends with a distinct prefix"
    );
}

// ----- Lock-domain + idempotency-under-race (closing the parity suite's blind spots) --

#[test]
fn lock_domains_are_per_stream_and_per_subsystem_postgres() {
    // The existing concurrency tests would still pass with a GLOBAL lock (a constant key),
    // because the (stream_id, seq) PK keeps sequences independent regardless. This test
    // pins the lock scheme directly: a held op-store lock on one stream must NOT block a
    // lock on a different stream, nor a registry lock on the SAME stream (distinct domain).
    let Some(base) = base_url("lock_domains") else {
        return;
    };
    let s1 = format!("lockprobe-{}-1", std::process::id());
    let s2 = format!("lockprobe-{}-2", std::process::id());

    // Same TLS connector as the backend and as `isolated_url` — a raw `NoTls` here would make
    // this test the one that still cannot reach a managed, TLS-only database.
    let mut holder = postgres::Client::connect(&base, nxs_server::tls_pg::make_tls().expect("tls"))
        .expect("connect lock holder");
    let mut probe = postgres::Client::connect(&base, nxs_server::tls_pg::make_tls().expect("tls"))
        .expect("connect lock probe");

    // Holder takes the op-store lock on s1 and keeps the transaction open (lock held).
    let mut htx = holder.transaction().expect("holder txn");
    htx.execute(
        "SELECT pg_advisory_xact_lock($1, hashtext($2))",
        &[&OPSTORE_LOCK_CLASS, &s1],
    )
    .expect("acquire op-store lock on s1");

    // Each probe is its own implicit (autocommit) transaction, so a successful try-lock is
    // released immediately and does not taint the next probe. `pg_try_advisory_xact_lock`
    // returns false iff the lock is already held by someone else — here, only the holder.
    let try_lock = |c: &mut postgres::Client, class: i32, sid: &str| -> bool {
        c.query_one(
            "SELECT pg_try_advisory_xact_lock($1, hashtext($2))",
            &[&class, &sid],
        )
        .expect("try-lock query")
        .get(0)
    };

    assert!(
        !try_lock(&mut probe, OPSTORE_LOCK_CLASS, &s1),
        "same stream + same subsystem must contend (the lock actually serialises writers)"
    );
    assert!(
        try_lock(&mut probe, OPSTORE_LOCK_CLASS, &s2),
        "a different stream must NOT be serialised by s1's lock (per-stream, not global)"
    );
    assert!(
        try_lock(&mut probe, REGISTRY_LOCK_CLASS, &s1),
        "the registry domain must NOT contend with the op-store lock on the same stream"
    );

    htx.rollback().expect("release holder lock");
}

#[test]
fn opstore_concurrent_duplicate_same_op_id_consumes_nothing_postgres() {
    // §4.4 under a real race (the parity trace only re-appends sequentially): many threads
    // append the SAME op_id at once. The idempotent-union check runs before MAX(seq)+1
    // under the lock, so every racer returns the SAME cursor, the op is stored once, and no
    // sequence value is wasted — the next distinct op is still contiguous.
    let Some(base) = base_url("opstore_dup_race") else {
        return;
    };
    let store = Arc::new(pg_opstore(&base));
    let s = StreamId("s".into());
    const THREADS: usize = 8;

    let mut handles = Vec::new();
    for _ in 0..THREADS {
        let store = Arc::clone(&store);
        let s = s.clone();
        handles.push(thread::spawn(move || {
            store.append(&s, &wire("dup")).unwrap().0
        }));
    }
    let cursors: Vec<i64> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    assert!(
        cursors.iter().all(|&c| c == 1),
        "every racing duplicate returns the one cursor (1), none a phantom seq: {cursors:?}"
    );
    let (ops, _) = store.read_since(&s, Cursor::BEGINNING, 10).unwrap();
    assert_eq!(ops.len(), 1, "the op is stored exactly once");
    assert_eq!(
        store.append(&s, &wire("next")).unwrap().0,
        2,
        "the duplicate race wasted no sequence value — next distinct op is contiguous"
    );
}
