//! A platform the adapter cannot run a session on (6j6v.6bcb): Windows has neither the process groups a session is
//! spawned into nor a process table the identity guard can read. There a start refuses with a named reason before it
//! writes or spawns anything, so nothing half-runs. Staged on unix through `with_spawning(false)`, the flag a Windows
//! build carries by default.

mod common;

use common::*;
use nexus_run_contracts::{Liveness, SessionBackend};
use nexus_run_session_claude::{ClaudeBackend, CANNOT_SPAWN_HERE};
use serde_json::json;

fn unsupported(bench: &Bench) -> ClaudeBackend {
    bench.backend().with_spawning(false)
}

#[test]
fn a_start_where_the_platform_cannot_spawn_refuses_with_the_named_reason() {
    let bench = Bench::new();
    let backend = unsupported(&bench);

    let refused = backend
        .start(&bench.spec(KEY, json!({ "steps": [{ "wait": "never" }] })))
        .expect_err("a platform that cannot spawn refuses the start");

    assert_eq!(refused.what, CANNOT_SPAWN_HERE);
}

#[test]
fn a_refused_start_leaves_no_ledger_and_no_claim_behind() {
    let bench = Bench::new();
    let backend = unsupported(&bench);

    let _ = backend.start(&bench.spec(KEY, json!({ "steps": [{ "wait": "never" }] })));

    assert!(
        !bench.ledger_dir(KEY).exists(),
        "nothing may be written for a session that never ran"
    );
}

#[test]
fn a_refused_start_is_refused_again_and_never_reads_as_a_live_session() {
    let bench = Bench::new();
    let backend = unsupported(&bench);
    let spec = bench.spec(KEY, json!({ "steps": [{ "wait": "never" }] }));

    let _ = backend.start(&spec);
    let again = backend
        .start(&spec)
        .expect_err("the second start refuses as well");

    assert_eq!(again.what, CANNOT_SPAWN_HERE);
    assert!(matches!(backend.alive(KEY), Ok(Liveness::Dead)));
}
