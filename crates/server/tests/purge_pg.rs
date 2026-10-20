//! `nxf-relay purge` on Postgres (6j6v.f6yb): every DoD line Postgres can show, each run as a real
//! `nxf-relay purge` process against a schema the server's own Postgres stores wrote. Keyed on
//! `DATABASE_URL` like the parity suite (CI: a hard failure when it is missing); the TLS job runs
//! this file over `sslmode=require` too, so the purge's own connection is proven through `tls_pg`.
#![cfg(feature = "postgres")]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::json;

use nxs_server::presence::PresenceStore;
use nxs_server::registry::PrefixRegistry;
use nxs_server::registry_pg::PostgresPrefixRegistry;
use nxs_server::store::OpStore;
use nxs_server::store_pg::PostgresOpStore;
use nxs_sync::protocol::{MachineHello, StreamId};
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

mod common;
use common::{base_url, drop_schema, isolated_schema};

#[path = "support/purge.rs"]
mod support;
use support::{purge, Purged, C9_KEY};

const H: &str = "fedcba9876543210fedcba9876543210";

fn target() -> String {
    format!("n-{H}-1")
}
fn neighbour() -> String {
    format!("n-{H}-10")
}
fn prefix_neighbour() -> String {
    format!("n-{H}-1-x")
}

/// A plain admin client on its own thread (no runtime here), through the same TLS connector.
fn admin(url: &str) -> postgres::Client {
    let url = url.to_string();
    std::thread::spawn(move || {
        postgres::Client::connect(&url, nxs_server::tls_pg::make_tls().unwrap()).unwrap()
    })
    .join()
    .unwrap()
}

fn on_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::spawn(f).join().unwrap()
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

struct Fixture {
    base: String,
    schema: String,
    url: String,
    tag: String,
    value_bytes: BTreeMap<String, u64>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        drop_schema(&self.base, &self.schema);
    }
}

impl Fixture {
    /// An isolated schema with the relay's tables, written by the server's own init_schema.
    fn new(test: &str) -> Option<Fixture> {
        let base = base_url(test)?;
        let (schema, url) = isolated_schema(&base, "purge");
        let u = url.clone();
        on_thread(move || {
            PostgresOpStore::connect(&u).unwrap();
            PostgresPrefixRegistry::connect(&u).unwrap();
        });
        Some(Fixture {
            base,
            schema,
            url,
            tag: format!(
                "{:x}",
                std::process::id() as u64 * 7919 + Instant::now().elapsed().as_nanos() as u64
            ),
            value_bytes: BTreeMap::new(),
        })
    }

    fn seeded(test: &str, ops: usize) -> Option<Fixture> {
        let mut f = Fixture::new(test)?;
        for s in [target(), neighbour(), prefix_neighbour()] {
            f.seed(&s, ops, 30);
        }
        Some(f)
    }

    fn seed(&mut self, stream: &str, ops: usize, machines: usize) {
        let (url, stream_s, tag) = (self.url.clone(), stream.to_string(), self.tag.clone());
        let bytes = on_thread(move || {
            let store = PostgresOpStore::connect(&url).unwrap();
            let reg = PostgresPrefixRegistry::connect(&url).unwrap();
            let s = StreamId(stream_s.clone());
            let mut bytes = 0u64;
            for i in 1..=ops {
                let value = format!(
                    "MARKER{tag}value<{stream_s}>{i} {}",
                    "y".repeat(i % 3 * 100)
                );
                bytes += value.len() as u64;
                let clock = format!("2026-10-18T20:00:{:02}Z", i % 60);
                store
                    .append(
                        &s,
                        &op(&format!("01J{tag}{stream_s}{i:05}"), &value, &clock),
                    )
                    .unwrap();
            }
            for (i, prefix) in ["ab12", "cd34"].iter().enumerate() {
                reg.register(&s, prefix, &format!("uuid-{tag}-{stream_s}-{i}"))
                    .unwrap();
            }
            for i in 0..machines {
                reg.announce(
                    &s,
                    &MachineHello {
                        machine_id: format!("m-{tag}-{stream_s}-{i}"),
                        name: format!("laptop {i}"),
                        interval_secs: 300,
                    },
                    1_760_000_000 + i as i64,
                    i64::MIN,
                )
                .unwrap();
            }
            bytes
        });
        self.value_bytes.insert(stream.to_string(), bytes);
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("NXF_RELAY_BACKEND", "postgres".into()),
            ("NXF_RELAY_PG_URL", self.url.clone()),
            ("NXS_LOG_HMAC_KEY", C9_KEY.into()),
        ]
    }

    fn run_with(&self, args: &[&str], extra: &[(&str, &str)]) -> Purged {
        let mut env = self.env();
        for (k, v) in extra {
            env.retain(|(name, _)| name != k);
            env.push((k, v.to_string()));
        }
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        purge(args, &env)
    }

    fn dry(&self, stream: &str) -> Purged {
        self.run_with(&[stream, "--dry-run"], &[])
    }

    fn confirm(&self, stream: &str) -> Purged {
        self.run_with(&[stream, "--dry-run", "--expect-empty"], &[])
    }

    fn yes(&self, stream: &str, digest: &str) -> Purged {
        self.run_with(&[stream, "--yes", "--plan", digest], &[])
    }

    /// Every row of every table in the schema as JSON text, sorted, without `without`'s rows.
    fn dump(&self, without: Option<&str>) -> Vec<String> {
        let (url, schema) = (self.url.clone(), self.schema.clone());
        let without = without.map(str::to_string);
        on_thread(move || {
            let mut c = admin(&url);
            let tables: Vec<String> = c
                .query(
                    "SELECT tablename FROM pg_tables WHERE schemaname = $1 ORDER BY tablename",
                    &[&schema],
                )
                .unwrap()
                .iter()
                .map(|r| r.get(0))
                .collect();
            let mut rows = Vec::new();
            for table in tables {
                for r in c
                    .query(
                        &format!("SELECT row_to_json(t)::text FROM \"{schema}\".\"{table}\" t"),
                        &[],
                    )
                    .unwrap()
                {
                    let text: String = r.get(0);
                    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                    if without.is_some() && v["stream_id"].as_str() == without.as_deref() {
                        continue;
                    }
                    rows.push(format!("{table}|{text}"));
                }
            }
            rows.sort();
            rows
        })
    }

    fn rows_of(&self, stream: &str) -> usize {
        self.dump(None).len() - self.dump(Some(stream)).len()
    }

    fn sql(&self, statement: &str) {
        let (url, statement) = (self.url.clone(), statement.to_string());
        on_thread(move || admin(&url).batch_execute(&statement).unwrap());
    }
}

macro_rules! fixture {
    ($test:expr, $ops:expr) => {
        match Fixture::seeded($test, $ops) {
            Some(f) => f,
            None => return,
        }
    };
}

#[test]
fn init_schema_writes_the_store_id_once_and_never_changes_it() {
    let Some(f) = Fixture::new("purge_pg_identity_row") else {
        return;
    };
    let read = |url: String| -> Vec<String> {
        on_thread(move || {
            admin(&url)
                .query("SELECT store_id FROM relay_store_identity", &[])
                .unwrap()
                .iter()
                .map(|r| r.get(0))
                .collect()
        })
    };
    let first = read(f.url.clone());
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].len(), 32);
    let u = f.url.clone();
    on_thread(move || {
        PostgresOpStore::connect(&u).unwrap();
        PostgresPrefixRegistry::connect(&u).unwrap();
    });
    assert_eq!(read(f.url.clone()), first, "a reboot keeps the id");
}

#[test]
fn a_dry_run_a_bound_yes_and_a_confirm_remove_the_target_and_only_it() {
    let f = fixture!("purge_pg_cycle", 25);
    let all_before = f.dump(None);
    let outside = f.dump(Some(&target()));
    let dry = f.dry(&target());
    dry.expect(0, "dry run");
    assert_eq!(f.dump(None), all_before, "a dry run deletes nothing");
    assert_eq!(dry.found("ops"), 25);
    assert_eq!(dry.found("registry"), 32);
    assert_eq!(dry.kind("ops")["value_bytes"], f.value_bytes[&target()]);
    assert_eq!(dry.kind("ops")["max_seq"], 25);
    assert_eq!(dry.kind("ops")["newest_wall_clock"], "2026-10-18T20:00:25Z");
    assert_eq!(
        dry.kind("registry")["by_prefix"],
        json!({"machine_presence": 30, "prefix_claims": 2})
    );
    assert_eq!(dry.report["store"]["backend"], "postgres");
    assert_eq!(dry.report["lease"], serde_json::Value::Null);
    let yes = f.yes(&target(), &dry.digest());
    yes.expect(0, "--yes");
    assert_eq!(yes.deleted("ops"), 25);
    assert_eq!(yes.deleted("registry"), 32);
    assert_eq!(yes.report["physical"], "not_attempted");
    assert_eq!(f.rows_of(&target()), 0);
    assert_eq!(
        f.dump(Some(&target())),
        outside,
        "neighbours byte-identical"
    );
    f.confirm(&target()).expect(0, "confirm");
}

#[test]
fn a_stale_plan_and_another_streams_digest_are_exit_4_with_nothing_deleted() {
    let f = fixture!("purge_pg_stale", 8);
    let dry = f.dry(&target());
    let neighbour_digest = f.dry(&neighbour()).digest();
    let before = f.dump(None);
    f.yes(&target(), &neighbour_digest)
        .expect(4, "another stream's digest");
    assert_eq!(f.dump(None), before);
    let u = f.url.clone();
    on_thread(move || {
        PostgresOpStore::connect(&u)
            .unwrap()
            .append(&StreamId(target()), &op("late", "late", ""))
            .unwrap()
    });
    let before = f.dump(None);
    f.yes(&target(), &dry.digest())
        .expect(4, "a push since the dry run");
    assert_eq!(f.dump(None), before);
}

#[test]
fn a_push_held_uncommitted_while_the_purge_waits_for_its_locks_is_exit_4() {
    // MF-1: a second connection appends through the store's own path — its advisory lock and its
    // insert — inside an open transaction, holds it until the purge's session waits for its
    // advisory lock (seen in pg_locks), then commits.
    let f = fixture!("purge_pg_held_push", 6);
    let dry = f.dry(&target());
    let (url, schema) = (f.url.clone(), f.schema.clone());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
    let (commit_tx, commit_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let mut c = admin(&url);
        let mut tx = c.transaction().unwrap();
        tx.execute(
            "SELECT pg_advisory_xact_lock($1, hashtext($2))",
            &[&nxs_server::store_pg::OPSTORE_LOCK_CLASS, &target()],
        )
        .unwrap();
        tx.execute(
            &format!(
                "INSERT INTO \"{schema}\".stream_ops (stream_id, seq, envelope_version, op_id,
                 lamport, site, domain, target_kind, target_id, field, op_type, value, author,
                 wall_clock) VALUES ($1, 7, 1, 'held-push', 1, 1, 'task', 'item', 'x', 'title',
                 'set', 'held', 'seed', '')"
            ),
            &[&target()],
        )
        .unwrap();
        ready_tx.send(()).unwrap();
        commit_rx.recv().unwrap();
        tx.commit().unwrap();
    });
    ready_rx.recv().unwrap();
    let env: Vec<(String, String)> = f
        .env()
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let digest = dry.digest();
    let purge_run = std::thread::spawn(move || {
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        purge(&[&target(), "--yes", "--plan", &digest], &env)
    });
    // Wait until a session waits for an advisory lock: the purge, queued behind the push.
    let url = f.url.clone();
    let waited = on_thread(move || {
        let mut c = admin(&url);
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let waiting: i64 = c
                .query_one(
                    "SELECT COUNT(*) FROM pg_locks WHERE locktype = 'advisory' AND NOT granted",
                    &[],
                )
                .unwrap()
                .get(0);
            if waiting > 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    });
    assert!(waited, "the purge must have queued behind the held push");
    commit_tx.send(()).unwrap();
    holder.join().unwrap();
    let out = purge_run.join().unwrap();
    out.expect(4, "the held push is in the re-read");
    assert_eq!(f.rows_of(&target()), 6 + 32 + 1, "nothing deleted");
}

fn finish(f: &Fixture, outside: &[String], what: &str) {
    let dry = f.dry(&target());
    dry.expect(0, &format!("{what}: fresh dry run"));
    f.yes(&target(), &dry.digest())
        .expect(0, &format!("{what}: bound --yes"));
    f.confirm(&target()).expect(0, &format!("{what}: confirm"));
    assert_eq!(f.dump(Some(&target())), outside, "{what}: neighbours");
}

#[test]
fn a_run_aborted_at_every_postgres_point_completes_on_rerun() {
    for point in ["P0", "P1-tx", "V0"] {
        let f = fixture!("purge_pg_crash", 6);
        let outside = f.dump(Some(&target()));
        let dry = f.dry(&target());
        let crashed = f.run_with(
            &[&target(), "--yes", "--plan", &dry.digest()],
            &[("NXF_RELAY_TEST_PURGE_STOP_AFTER", point)],
        );
        assert_ne!(crashed.code, Some(0), "{point}");
        let left = f.rows_of(&target());
        match point {
            "V0" => assert_eq!(left, 0, "{point}: committed"),
            // P1-tx: stream_ops, prefix_claims and machine_presence all untouched.
            _ => assert_eq!(left, 6 + 32, "{point}: nothing deleted"),
        }
        finish(&f, &outside, point);
    }
}

#[test]
fn each_injection_gives_exactly_its_exit_and_residue() {
    for (point_write, code, residue) in [
        ("P0:op", 4, BTreeMap::new()),
        ("P0:machine", 4, BTreeMap::new()),
        ("P0:prefix", 4, BTreeMap::new()),
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
        let f = fixture!("purge_pg_inject", 5);
        let outside = f.dump(Some(&target()));
        let dry = f.dry(&target());
        let out = f.run_with(
            &[&target(), "--yes", "--plan", &dry.digest()],
            &[("NXF_RELAY_TEST_PURGE_INJECT_AT", point_write)],
        );
        out.expect(code, point_write);
        if code == 3 {
            assert_eq!(out.residue(), residue, "{point_write}");
        } else {
            assert_eq!(
                f.rows_of(&target()),
                5 + 32 + 1,
                "{point_write}: nothing deleted"
            );
        }
        assert_eq!(f.dump(Some(&target())), outside, "{point_write}");
        finish(&f, &outside, point_write);
    }
}

#[test]
fn what_is_not_a_relay_store_is_78_naming_what_is_missing() {
    let Some(base) = base_url("purge_pg_not_a_store") else {
        return;
    };
    for (setup, missing) in [
        ("", "stream_ops"),
        (
            "CREATE TABLE stream_ops(stream_id TEXT, seq BIGINT, op_id TEXT, wall_clock TEXT, value TEXT);
             CREATE TABLE prefix_claims(stream_id TEXT, prefix TEXT, replica_uuid TEXT);",
            "machine_presence",
        ),
        (
            "CREATE TABLE stream_ops(stream_id TEXT, seq BIGINT, op_id TEXT, wall_clock TEXT, value TEXT);
             CREATE TABLE prefix_claims(stream_id TEXT, prefix TEXT, replica_uuid TEXT);
             CREATE TABLE machine_presence(stream_id TEXT, machine_id TEXT, last_seen BIGINT);",
            "relay_store_identity",
        ),
    ] {
        let (schema, url) = isolated_schema(&base, "purge_nas");
        if !setup.is_empty() {
            let (u, s) = (url.clone(), setup.to_string());
            on_thread(move || admin(&u).batch_execute(&s).unwrap());
        }
        let out = purge(
            &[&target(), "--dry-run"],
            &[("NXF_RELAY_BACKEND", "postgres"), ("NXF_RELAY_PG_URL", &url)],
        );
        out.expect(78, missing);
        assert!(
            out.report["error"]["message"].as_str().unwrap().contains(missing),
            "{missing}: {}",
            out.stdout
        );
        let u = url.clone();
        let tables: i64 = on_thread(move || {
            admin(&u)
                .query_one(
                    "SELECT COUNT(*) FROM pg_tables WHERE schemaname = current_schema()",
                    &[],
                )
                .unwrap()
                .get(0)
        });
        let expected = setup.matches("CREATE TABLE").count() as i64;
        assert_eq!(tables, expected, "the purge created nothing");
        drop_schema(&base, &schema);
    }
}

#[test]
fn the_url_comes_from_nxf_relay_pg_url_only() {
    let f = fixture!("purge_pg_no_default", 1);
    let out = purge(
        &[&target(), "--dry-run"],
        &[("NXF_RELAY_BACKEND", "postgres"), ("DATABASE_URL", &f.url)],
    );
    out.expect(78, "DATABASE_URL is not read");
    assert!(out.report["error"]["message"]
        .as_str()
        .unwrap()
        .contains("NXF_RELAY_PG_URL"));
}

#[test]
fn an_unknown_table_hidden_from_a_least_privilege_role_still_refuses_every_mode() {
    // The guard reads pg_catalog: a table the purge's role holds no privilege on is invisible to
    // information_schema but not to pg_catalog.
    let f = fixture!("purge_pg_unknown_table", 4);
    let role = format!(
        "nxf_purge_{}",
        f.schema
            .replace("nxf_", "")
            .chars()
            .rev()
            .take(12)
            .collect::<String>()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "")
    );
    let s = f.schema.clone();
    f.sql(&format!(
        "CREATE ROLE {role} LOGIN PASSWORD 'purge';
         GRANT USAGE ON SCHEMA \"{s}\" TO {role};
         GRANT SELECT, DELETE ON \"{s}\".stream_ops, \"{s}\".prefix_claims, \"{s}\".machine_presence TO {role};
         GRANT SELECT ON \"{s}\".relay_store_identity TO {role};
         CREATE TABLE \"{s}\".zzz_extra(stream_id TEXT, v TEXT);
         INSERT INTO \"{s}\".zzz_extra VALUES ('{t}', 'kept');",
        t = target()
    ));
    // The role's URL: the base URL with this role's credentials, the schema as search_path.
    let mut cfg: postgres::Config = f.base.parse().unwrap();
    cfg.user(&role).password("purge");
    let host = match &cfg.get_hosts()[0] {
        postgres::config::Host::Tcp(h) => h.clone(),
        #[allow(unreachable_patterns)]
        _ => "localhost".into(),
    };
    let port = cfg.get_ports().first().copied().unwrap_or(5432);
    let db = cfg.get_dbname().unwrap_or("postgres").to_string();
    let query = f
        .base
        .split_once('?')
        .map(|(_, q)| format!("?{q}"))
        .unwrap_or_default();
    let role_url = nxs_server::managed_pg::url_with_search_path(
        &format!("postgres://{role}:purge@{host}:{port}/{db}{query}"),
        &f.schema,
    );
    let env = [
        ("NXF_RELAY_BACKEND", "postgres"),
        ("NXF_RELAY_PG_URL", role_url.as_str()),
    ];
    let before = f.dump(None);
    for args in [
        vec![target(), "--dry-run".into()],
        vec![target(), "--dry-run".into(), "--expect-empty".into()],
        vec![target(), "--yes".into(), "--plan".into(), "00".repeat(32)],
    ] {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = purge(&args, &env);
        out.expect(78, &format!("{args:?}"));
        assert!(out.report["error"]["message"]
            .as_str()
            .unwrap()
            .contains("zzz_extra"));
    }
    assert_eq!(f.dump(None), before);
    // Without the foreign table, the least-privilege role runs the whole purge.
    f.sql(&format!("DROP TABLE \"{s}\".zzz_extra;"));
    let dry = purge(&[&target(), "--dry-run"], &env);
    dry.expect(0, "least privilege dry run");
    purge(&[&target(), "--yes", "--plan", &dry.digest()], &env).expect(0, "least privilege --yes");
    f.sql(&format!(
        "REVOKE ALL ON ALL TABLES IN SCHEMA \"{s}\" FROM {role}; REVOKE USAGE ON SCHEMA \"{s}\" FROM {role}; DROP ROLE {role};"
    ));
}

#[test]
fn the_identity_is_database_schema_and_store_id_and_host_is_only_information() {
    let f = fixture!("purge_pg_identity", 2);
    let dry = f.dry(&target());
    let identity = &dry.report["store"]["identity"];
    let keys: Vec<&str> = identity
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["database", "schema", "store_id"]);
    assert_eq!(identity["schema"], f.schema);
    assert!(dry.report["store"]["host"].is_string());
    // MF-10: the same database through another name for the host is the same store.
    let other_host = if f.url.contains("localhost") {
        f.url.replacen("localhost", "127.0.0.1", 1)
    } else {
        f.url.replacen("127.0.0.1", "localhost", 1)
    };
    let again = f.run_with(
        &[&target(), "--dry-run"],
        &[("NXF_RELAY_PG_URL", &other_host)],
    );
    again.expect(0, "other host name");
    assert_eq!(again.identity_digest(), dry.identity_digest());
}

#[test]
fn every_table_of_a_relay_schema_is_purged_or_allowed() {
    let Some(f) = Fixture::new("purge_pg_schema") else {
        return;
    };
    let mut tables = f
        .dump(None)
        .iter()
        .map(|r| r.split('|').next().unwrap().to_string())
        .collect::<Vec<_>>();
    let (url, schema) = (f.url.clone(), f.schema.clone());
    tables.extend(on_thread(move || {
        admin(&url)
            .query(
                "SELECT tablename FROM pg_tables WHERE schemaname = $1",
                &[&schema],
            )
            .unwrap()
            .iter()
            .map(|r| r.get::<_, String>(0))
            .collect::<Vec<_>>()
    }));
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

#[test]
fn an_empty_store_needs_the_pin() {
    let Some(f) = Fixture::new("purge_pg_empty") else {
        return;
    };
    let dry = f.dry(&target());
    dry.expect(78, "empty store");
    assert_eq!(dry.report["error"]["kind"], "EmptyStore");
    let pin = dry.identity_digest();
    f.run_with(
        &[
            &target(),
            "--dry-run",
            "--expect-empty",
            "--expect-identity",
            &pin,
        ],
        &[],
    )
    .expect(0, "with the pin");
}
