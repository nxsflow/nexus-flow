//! [`managed_pg::probe`]'s **live wiring**, deterministically (PR #320 review, Test Quality #1).
//!
//! The decision tables (`classify_text`, `classify_sqlstate`) are unit-tested next to the code and
//! need no database. What they cannot cover is the part in between: that a real driver error
//! actually reaches the classifier, that the SQLSTATE branch is really taken when the server
//! answers, that the deadline is really applied to the connection, and that a successful probe
//! really walks all four of its steps.
//!
//! That wiring was exercised only by the live-Supabase job — which needs a secret, is unavailable
//! to fork pull requests, and in practice only ever proves the happy path. A regression in the
//! wiring itself (the wrong error propagated, a timeout never set) would have gone unnoticed on
//! every ordinary PR, which is the same shape of gap the whole ticket is about.
//!
//! So these run against the `postgres:16` service container `postgres-parity` already starts, and
//! three of them need no database at all. **Every case here is environment-independent on
//! purpose**: no test depends on the server's authentication method, because a wrong-password test
//! would assert one thing against CI's password-protected container and the opposite against the
//! trust-auth cluster a contributor is likely to be running. The SQLSTATE path is proven with a
//! database that does not exist instead — same branch, same classifier, no such dependency.
#![cfg(feature = "postgres")]

use std::time::{Duration, Instant};

use nxs_server::managed_pg::{self, Cause, Verdict};

mod common;

use common::base_url;

/// Add a libpq connection parameter to a URL, whatever shape it already has.
fn with_param(base: &str, param: &str) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}{param}")
}

/// The whole point of the preflight: against a working endpoint it must walk connect → read →
/// `CREATE SCHEMA` → startup-`options` round-trip and come back clean. A probe that reported
/// `Reachable` without doing those would let every later failure be blamed on the integration.
#[test]
fn the_configured_endpoint_probes_as_reachable() {
    let Some(base) = base_url("probe_reachable") else {
        return;
    };
    let report = managed_pg::probe(&base, Duration::from_secs(30));
    assert_eq!(
        report.verdict,
        Verdict::Reachable,
        "the configured endpoint must probe clean: {}",
        report.banner()
    );
    assert_eq!(report.cause, Cause::None);
}

/// The SQLSTATE branch, end to end: the server answers, and the answer is about configuration.
/// This is what proves `classify_error` really consults `e.code()` on a live error — a unit test
/// on `classify_sqlstate` alone cannot, because it never sees a `postgres::Error`.
#[test]
fn a_database_that_does_not_exist_is_a_configuration_answer() {
    let Some(base) = base_url("probe_bad_database") else {
        return;
    };
    let report = managed_pg::probe(
        &with_param(&base, "dbname=nxf_no_such_database_exists"),
        Duration::from_secs(30),
    );
    assert_eq!(
        report.verdict,
        Verdict::Misconfigured,
        "a server that answers 'no such database' is not an outage: {}",
        report.banner()
    );
    assert_eq!(report.cause, Cause::Database, "{}", report.banner());
    assert!(
        report.detail.contains("nxf_no_such_database_exists"),
        "the driver's own words must survive to the reader: {}",
        report.detail
    );
}

/// The transport branch, end to end. Needs no database — port 1 is not listening anywhere.
#[test]
fn a_closed_port_is_an_availability_answer() {
    let report = managed_pg::probe(
        "postgres://postgres@127.0.0.1:1/postgres",
        Duration::from_secs(10),
    );
    assert_eq!(
        report.verdict,
        Verdict::Unreachable,
        "a refused connection is availability, not configuration: {}",
        report.banner()
    );
    assert_eq!(report.cause, Cause::Connect, "{}", report.banner());
}

/// A malformed connection string must be answered without a packet leaving the machine.
#[test]
fn an_unparseable_url_never_leaves_the_machine() {
    let report = managed_pg::probe("this is not a connection string", Duration::from_secs(10));
    assert_eq!(report.verdict, Verdict::Misconfigured);
    assert_eq!(report.cause, Cause::Url, "{}", report.banner());
}

/// The deadline is really wired to the connection.
///
/// Asserted as a **bound on elapsed time**, not as a particular cause: `192.0.2.1` is TEST-NET-1
/// (RFC 5737), which is guaranteed to route nowhere, but whether a given runner black-holes it
/// (→ `Timeout`) or answers "network unreachable" (→ `Connect`) is the network's business, not
/// ours. Both are `Unreachable`, and the property that actually matters — a probe cannot hang past
/// its budget — holds either way. Pinning the cause instead would be a test about the runner's
/// firewall.
#[test]
fn the_probe_returns_within_its_deadline() {
    let budget = Duration::from_secs(3);
    let started = Instant::now();
    let report = managed_pg::probe("postgres://postgres@192.0.2.1:5432/postgres", budget);
    let elapsed = started.elapsed();

    assert_eq!(
        report.verdict,
        Verdict::Unreachable,
        "an unroutable address is an availability answer: {}",
        report.banner()
    );
    assert!(
        elapsed < budget * 4,
        "the probe must respect its connect deadline — {budget:?} budgeted, {elapsed:?} taken. \
         A probe that can hang would stall the blocking merge gate this job is meant to become."
    );
}
