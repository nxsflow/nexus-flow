//! Shared harness for the Postgres-backed test binaries (`postgres_parity`,
//! `postgres_in_runtime`, `postgres_tls`).
//!
//! Extracted when the third file would have carried a third copy (PR #305 review, Code Quality
//! #3). What stays per-file is what each file exists to pin — the parity scripts, the calling
//! contexts, the TLS handshake; what moves here is only the plumbing every one of them needs:
//! *which* database, and *where* its tables live.
#![allow(dead_code)] // each test binary uses a different subset

use nxs_server::managed_pg::{configured_url, scratch_schema_name, url_with_search_path};
use nxs_sync::wire::{WireOp, ENVELOPE_VERSION};

/// The base libpq URL, or `None` (with a loud skip) when `DATABASE_URL` is unset.
///
/// Locally the suites skip when no database is configured, keeping `cargo test --features
/// postgres` green with zero setup. **In CI they must never silently skip**: if a misconfig ever
/// dropped `DATABASE_URL` while the `postgres` feature is built, a convention-only gate would
/// revert to all-skip on a green build. So under `CI` a missing `DATABASE_URL` is a hard failure.
pub fn base_url(test: &str) -> Option<String> {
    // "Empty counts as unset, and trim what is there" is `managed_pg::configured_url`'s rule, not
    // a second copy of it here — a fork PR's absent secret arrives as an empty string, and the two
    // places that ask the question must not be able to answer it differently.
    match configured_url(std::env::var("DATABASE_URL").ok().as_deref()) {
        Some(url) => Some(url),
        None => {
            assert!(
                std::env::var("CI").is_err(),
                "{test}: DATABASE_URL must be set under --features postgres in CI — \
                 the Postgres suites must fail loud, not silently skip"
            );
            eprintln!("SKIP {test}: DATABASE_URL unset (set it to run the Postgres suite)");
            None
        }
    }
}

/// Create an isolated schema and return a URL whose connections default `search_path` to it — so
/// the backend's `CREATE TABLE IF NOT EXISTS` and every query land in that schema.
///
/// Two deliberate choices, both learned the hard way:
///
/// * The admin connection dials through the **same** TLS connector the backend uses
///   (`tls_pg::make_tls`), never a hand-rolled `NoTls`. With `NoTls` the suites failed before their
///   first assertion against precisely the databases most worth proving ourselves against — any
///   managed, TLS-only Postgres — while staying green against a plaintext local cluster.
/// * It runs on a **plain thread**, so the harness works identically whether or not the caller is
///   inside a Tokio runtime. `postgres_in_runtime` exists to test exactly that distinction; its
///   setup must not depend on the thing under test.
///
/// The NAME comes from `managed_pg::scratch_schema_name`, which stamps the creation time into it.
/// That is what lets the sweep drop these again (6j6v.da42): against a per-run service container a
/// left-behind schema is free, but against a persistent managed project — the whole point of that
/// ticket — every test would leave one behind forever.
pub fn isolated_url(base: &str, tag: &str) -> String {
    isolated_schema(base, tag).1
}

/// [`isolated_url`], with the schema's NAME as well — for a caller that has somewhere to drop it.
pub fn isolated_schema(base: &str, tag: &str) -> (String, String) {
    let schema = scratch_schema_name(tag);
    let base_owned = base.to_string();
    let schema_owned = schema.clone();
    std::thread::spawn(move || {
        let mut admin =
            postgres::Client::connect(&base_owned, nxs_server::tls_pg::make_tls().expect("tls"))
                .expect("connect to create schema");
        admin
            .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema_owned}"))
            .expect("create test schema");
    })
    .join()
    .expect("schema setup thread");
    let url = url_with_search_path(base, &schema);
    (schema, url)
}

/// Drop a schema created by [`isolated_schema`], best effort.
///
/// Best effort on purpose: a cleanup failure must not turn a passing test red — the sweep is what
/// actually guarantees the project does not grow, and it does not depend on any test finishing.
pub fn drop_schema(base: &str, schema: &str) {
    let base_owned = base.to_string();
    let schema_owned = schema.to_string();
    let _ = std::thread::spawn(move || {
        let Ok(tls) = nxs_server::tls_pg::make_tls() else {
            return;
        };
        if let Ok(mut admin) = postgres::Client::connect(&base_owned, tls) {
            let _ = admin.batch_execute(&format!("DROP SCHEMA IF EXISTS {schema_owned} CASCADE"));
        }
    })
    .join();
}

/// A minimal valid wire op. Field values are irrelevant to every suite that uses it — what
/// matters is that the relay stores and returns it verbatim.
pub fn wire(op_id: &str) -> WireOp {
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
        value: Some("t".into()),
        author: "a".into(),
        wall_clock: String::new(),
        extra: Default::default(),
    }
}

/// [`wire`], as a payload from a build that knows a field this one does not — the shape 6j6v.5crb
/// is about. Parsed from JSON on purpose: that is the only way an unknown field can enter, and it
/// proves the entry point (serde) and the storage layer agree.
pub fn wire_from_a_newer_client(op_id: &str) -> WireOp {
    let mut raw = serde_json::to_value(wire(op_id)).expect("a wire op serializes");
    raw["sig"] = serde_json::Value::String("ed25519:deadbeef".into());
    serde_json::from_value(raw).expect("an unknown field parses into the catch-all")
}
