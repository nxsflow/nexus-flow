//! A **live TLS handshake** against a TLS-enabled Postgres — success path and rejection path.
//!
//! Why this file exists (PR #305 review, Test Quality #1): reaching a managed Postgres is the
//! headline capability of the Postgres backend, and until now nothing drove it end to end.
//! `tls_pg.rs`'s unit tests prove certificate *loading* without opening a socket, and the
//! `postgres-parity` CI job runs against a stock `postgres:16` container with SSL **off** — so
//! `sslmode=prefer` quietly negotiates plaintext there and the whole
//! `tokio-postgres-rustls`/`ring` path stayed unexercised. A dependency bump that changed the
//! crypto provider, or a broken certificate chain, would have shipped undetected behind a green
//! board. That is the same class of failure the backend itself was fixed for.
//!
//! ## The assertion that makes this worth having
//!
//! Connecting successfully proves nothing on its own: with `sslmode=prefer` a *failed* TLS
//! negotiation falls back to plaintext and still connects. So the success test asks the server
//! what it actually got — `pg_stat_ssl` for this backend PID — and requires `ssl = true`. A
//! regression that silently disabled TLS would keep every other test in the repo green and fail
//! here, which is precisely the point.
//!
//! ## Two modes, two processes
//!
//! The trust anchor comes from `NXF_RELAY_PG_CA_FILE`, which is read from the process
//! environment, so "trusted" and "untrusted" cannot be exercised in one test binary without
//! mutating global state underneath parallel tests. Instead each test declares the mode it needs
//! via `NXF_RELAY_PG_TLS_MODE` and the CI job runs the binary **twice**:
//!
//! ```text
//! NXF_RELAY_PG_TLS_MODE=ca     NXF_RELAY_PG_CA_FILE=<ca.pem>   → the success path runs
//! NXF_RELAY_PG_TLS_MODE=no-ca  (no CA file)                    → the rejection path runs
//! ```
//!
//! Unset, both tests skip loudly — that keeps `cargo test --features postgres` green against the
//! plaintext cluster a contributor is likely to have. An *unknown* value is a hard error rather
//! than a skip, so a typo in the workflow cannot turn this suite into a no-op that still reports
//! success.
#![cfg(feature = "postgres")]

mod common;

use common::{base_url, isolated_url, wire};
use nxs_server::store::OpStore;
use nxs_server::store_pg::PostgresOpStore;
use nxs_sync::protocol::{Cursor, StreamId};

/// True when this process is set up to prove `want`; otherwise says so, so a reader of the log
/// can tell "did not run in this invocation" from "ran and passed". Both look like `ok` to the
/// test harness, and only one of them is evidence.
fn runs_in_mode(test: &str, want: &str) -> bool {
    match tls_mode(test) {
        Some(mode) if mode == want => true,
        Some(mode) => {
            eprintln!("SKIP {test}: needs NXF_RELAY_PG_TLS_MODE={want}, this run is {mode}");
            false
        }
        None => false,
    }
}

/// Which side of the handshake this process is set up to prove. `None` ⇒ not a TLS run.
fn tls_mode(test: &str) -> Option<&'static str> {
    match std::env::var("NXF_RELAY_PG_TLS_MODE") {
        Err(_) => {
            eprintln!(
                "SKIP {test}: NXF_RELAY_PG_TLS_MODE unset \
                 (set it to `ca` or `no-ca` against a TLS-enabled Postgres)"
            );
            None
        }
        Ok(m) if m == "ca" => Some("ca"),
        Ok(m) if m == "no-ca" => Some("no-ca"),
        Ok(other) => panic!(
            "NXF_RELAY_PG_TLS_MODE={other:?} is not a mode — expected `ca` or `no-ca`. \
             Failing loudly rather than skipping: a typo here would silently turn the TLS \
             suite into a no-op that still reports success."
        ),
    }
}

#[test]
fn the_session_is_really_encrypted_when_the_ca_is_trusted() {
    let test = "tls_success";
    if !runs_in_mode(test, "ca") {
        return;
    }
    let Some(base) = base_url(test) else { return };
    assert!(
        std::env::var("NXF_RELAY_PG_CA_FILE").is_ok(),
        "mode `ca` requires NXF_RELAY_PG_CA_FILE — without it this test would be asserting \
         nothing about the trust anchor"
    );
    assert!(
        base.contains("sslmode=require") || base.contains("sslmode=verify"),
        "mode `ca` requires a DATABASE_URL that MANDATES TLS (got {base:?}); under the default \
         `prefer` a broken handshake falls back to plaintext and this test would pass anyway"
    );

    // The product path: the store connects, creates its schema, and round-trips an op — all of
    // it over the TLS connection built by `tls_pg::make_tls`.
    let url = isolated_url(&base, "tls");
    let store = PostgresOpStore::connect(&url).expect("connect over TLS with the CA trusted");
    let stream = StreamId("tls-proof".into());
    let cursor = store.append(&stream, &wire("t1")).expect("append over TLS");
    assert_eq!(cursor.0, 1);
    let (ops, _next) = store
        .read_since(&stream, Cursor::BEGINNING, 10)
        .expect("read back over TLS");
    assert_eq!(ops.len(), 1, "the op must survive the encrypted round trip");

    // …and the assertion the rest of the suite cannot make: the server confirms THIS backend
    // session is encrypted. Without it, a silent fallback to plaintext would look identical.
    let mut client = postgres::Client::connect(&url, nxs_server::tls_pg::make_tls().expect("tls"))
        .expect("second connection for the pg_stat_ssl probe");
    let row = client
        .query_one(
            "SELECT ssl, version FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .expect("pg_stat_ssl");
    let ssl: bool = row.get(0);
    let version: Option<String> = row.get(1);
    assert!(
        ssl,
        "the server reports this session as UNencrypted — TLS silently fell back to plaintext"
    );
    eprintln!(
        "tls_success: negotiated {}",
        version.as_deref().unwrap_or("?")
    );
}

#[test]
fn the_handshake_is_refused_when_the_ca_is_not_trusted() {
    let test = "tls_rejection";
    if !runs_in_mode(test, "no-ca") {
        return;
    }
    let Some(base) = base_url(test) else { return };
    assert!(
        std::env::var("NXF_RELAY_PG_CA_FILE").is_err(),
        "mode `no-ca` requires NXF_RELAY_PG_CA_FILE to be UNSET — with it the server's \
         certificate would verify and there would be no rejection to observe"
    );

    // Deliberately NOT through `PostgresOpStore::connect`: the pool retries for
    // NXF_RELAY_PG_ACQUIRE_TIMEOUT_SECS (default 30 s) before surfacing the error, which would
    // make a fast, deterministic assertion into a half-minute wait. This dials with the very
    // connector the store hands to the pool, so the handshake under test is identical.
    // `Client` is not `Debug`, so `expect_err` is unavailable — match instead of unwrapping.
    let rendered =
        match postgres::Client::connect(&base, nxs_server::tls_pg::make_tls().expect("tls")) {
            Ok(_) => panic!("a server certificate signed by an untrusted CA must NOT be accepted"),
            Err(err) => format!("{err:?}"),
        };
    assert!(
        rendered.contains("InvalidCertificate") || rendered.contains("UnknownIssuer"),
        "the refusal must come from certificate verification, not from something incidental \
         like a closed port: {rendered}"
    );
}

/// The Supabase regression, reproduced with no secret and no Supabase.
///
/// The very first live run of the `managed-postgres` job died here: Supabase signs its Postgres and
/// pooler certificates with its own root, the relay carries only the bundled Mozilla roots, and the
/// handshake failed with `UnknownIssuer` — which the classifier then filed under
/// `UNREACHABLE`, sending a reader to a status page that said everything was fine.
///
/// This job already stands up exactly that situation: a real TLS server whose CA this process does
/// not trust. So the fix is pinned against a real handshake on every change, rather than against a
/// string in a unit test and a secret not every run can reach.
#[test]
fn an_untrusted_certificate_is_reported_as_a_trust_problem_not_an_outage() {
    let test = "tls_probe_verdict";
    if !runs_in_mode(test, "no-ca") {
        return;
    }
    let Some(base) = base_url(test) else { return };

    let report = nxs_server::managed_pg::probe(&base, std::time::Duration::from_secs(15));
    assert_eq!(
        report.cause,
        nxs_server::managed_pg::Cause::TlsTrust,
        "the server WAS there and presented a certificate — that is a trust problem: {}",
        report.banner()
    );
    assert_eq!(
        report.verdict,
        nxs_server::managed_pg::Verdict::Misconfigured,
        "an outage cannot produce UnknownIssuer, so this must never read as one: {}",
        report.banner()
    );
    assert!(
        report.banner().contains("NXF_RELAY_PG_CA_FILE"),
        "the verdict has to name the lever that fixes it: {}",
        report.banner()
    );
}
