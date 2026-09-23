//! One test, one binary — because it mutates process-global state (PR #320 review, Code Quality
//! #2).
//!
//! `NXF_RELAY_PG_CA_FILE` is read from the environment on every `tls_pg::make_tls()` call, so a
//! test that sets it races every parallel test in the same binary that opens a connection. That is
//! the documented `det-ids-env-test-race` trap in this repo, and `postgres_tls.rs` avoids it by
//! having CI run *that* binary twice with different environments. Here the cheaper answer applies:
//! the case needs no database and no server, so it gets a test binary to itself, where "process
//! global" and "test local" are the same thing.
//!
//! What it pins: a broken CA file is a **local setup** failure, not a reachability one. It used to
//! be reported as `Cause::Tls` → `Verdict::Unreachable`, whose advice sends the reader to the
//! provider's status page — for a typo in their own environment, with no packet ever sent.
#![cfg(feature = "postgres")]

use nxs_server::managed_pg::{self, Cause, Verdict};
use std::time::Duration;

#[test]
fn a_ca_file_that_is_not_there_is_a_local_problem_not_an_outage() {
    // Not `tempfile` — the point is a path that does NOT exist.
    std::env::set_var(
        "NXF_RELAY_PG_CA_FILE",
        "/nonexistent/nxf-managed-pg/no-such-ca.pem",
    );

    // The URL is deliberately a plausible, reachable-looking one: the probe must fail on the local
    // setup BEFORE it dials, so which endpoint it names has to make no difference at all.
    let report = managed_pg::probe(
        "postgres://postgres@127.0.0.1:5432/postgres",
        Duration::from_secs(5),
    );

    assert_eq!(
        report.cause,
        Cause::TlsSetup,
        "a missing CA file is a local setup failure: {}",
        report.banner()
    );
    assert_eq!(
        report.verdict,
        Verdict::Misconfigured,
        "…and therefore never an availability verdict: {}",
        report.banner()
    );
    assert!(
        report.detail.contains("NXF_RELAY_PG_CA_FILE"),
        "the message must name the variable that caused it: {}",
        report.detail
    );
    assert!(
        !report.banner().contains("status page"),
        "must not send a reader to a provider status page for a local typo: {}",
        report.banner()
    );

    std::env::remove_var("NXF_RELAY_PG_CA_FILE");
}
