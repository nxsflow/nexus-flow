//! The workflow's `if:` guards and `ProofMode::token()` must spell the mode the same way
//! (PR #321 review, Test Quality #3).
//!
//! Every managed step in the `managed-postgres` job is guarded on
//! `steps.proof.outputs.mode == 'managed'`. Rename the token in Rust and nothing breaks: the
//! binary publishes `mode=<new>`, every guard compares it against `'managed'`, every comparison
//! is false, every managed step is **skipped** — and a job whose every step is skipped reports
//! **GREEN**. That is the same silent pass the whole ticket exists to close, arriving through a
//! refactor instead of through a fork, and no compiler can see it: one side is Rust, the other is
//! YAML.
//!
//! So the two are pinned to each other here. The repository already does this for a string that
//! has to be spelled the same in six places (`every_place_that_ships_the_sidecar_spells_it_the_same`,
//! `crates/chat/tests/worker.rs`); this is the same idea for a value that decides whether a
//! blocking check actually checks anything.

#![cfg(feature = "postgres")]

use nxs_server::managed_pg::ProofMode;

/// The workflow, read from the repository rather than embedded, so it cannot drift from the file
/// GitHub actually runs.
fn ci_workflow() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows/ci.yml")
        .canonicalize()
        .expect(".github/workflows/ci.yml must exist — this test is about that file");
    std::fs::read_to_string(path).expect("read .github/workflows/ci.yml")
}

/// Every guard in the workflow compares against a token some `ProofMode` actually produces.
///
/// This is the direction that matters: a guard naming a mode that no longer exists is a guard
/// that is always false, and a step that is always skipped in a job that is always green.
#[test]
fn every_proof_mode_guard_in_ci_names_a_mode_that_exists() {
    let workflow = ci_workflow();
    let known = [
        ProofMode::Managed.token(),
        ProofMode::ForkPullRequest.token(),
        ProofMode::NotConfigured.token(),
    ];

    const COMPARISON: &str = "steps.proof.outputs.mode == '";
    let mut guards = 0;
    for (at, _) in workflow.match_indices(COMPARISON) {
        let quoted = workflow[at + COMPARISON.len()..]
            .split('\'')
            .next()
            .expect("a quoted comparison always has a closing quote");
        guards += 1;
        assert!(
            known.contains(&quoted),
            "ci.yml guards on mode == '{quoted}', which no ProofMode produces (known: {known:?}). \
             Every step behind that guard is skipped, and a job whose steps are all skipped \
             reports green."
        );
    }

    assert!(
        guards > 0,
        "no `steps.proof.outputs.mode` guard left in ci.yml — either the managed-postgres job \
         stopped gating its suites, or this test is watching the wrong file"
    );
}

/// …and the mode that lets the real proof through is actually used.
///
/// The check above passes vacuously if every guard were rewritten to some *other* valid token, so
/// this pins the one that matters: without a `managed` guard, nothing gates the Supabase suites.
#[test]
fn the_managed_token_is_the_one_ci_gates_the_supabase_suites_on() {
    let workflow = ci_workflow();
    let guard = format!(
        "steps.proof.outputs.mode == '{}'",
        ProofMode::Managed.token()
    );
    assert!(
        workflow.contains(&guard),
        "ci.yml no longer contains `{guard}` — if ProofMode::Managed was renamed, the workflow \
         has to be renamed with it in the same commit"
    );
}

/// The tamper detector needs its `env:` line, and the name of that variable is a contract between
/// this crate and the workflow in exactly the same way the token is.
#[test]
fn the_fork_cross_check_signal_is_wired_into_the_job() {
    let workflow = ci_workflow();
    assert!(
        workflow.contains("NXF_CI_HEAD_IS_FORK:"),
        "the mode step must be handed the runner's own view of fork-ness; without it \
         `cross_check_fork_signal` fails the job by design"
    );
    assert!(
        workflow.contains("github.event.pull_request.head.repo.fork"),
        "the cross-check is only worth anything if it comes from the runner's context — a value \
         re-derived inside the job could be tampered with by the same code it guards against"
    );
}
