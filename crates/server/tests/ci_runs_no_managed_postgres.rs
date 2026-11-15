//! On `v2`, CI runs no proof against the managed Postgres (NEX-39).
//!
//! The `managed-postgres` job tested v1's Postgres storage against one shared Supabase project.
//! On `v2` storage moves to NoCentralDB (RFC 0001), so that proof no longer serves `v2`'s target,
//! and the owner turned it off there on 2026-11-15. `main` keeps the job: v1 releases are cut from
//! `main`, and its Postgres relay still ships.
//!
//! `main` is merged into `v2` every week. A merge that brought the job back would go unnoticed
//! until the shared project's rate limit turned unrelated pull requests red again, so the rule is
//! pinned here, against the workflow file GitHub actually runs. The `managed_pg` module and its
//! `managed-pg` example stay in the crate, unit-tested there: `main` still runs them.
//!
//! `cargo test --all` in `quality-gates` runs this file; it needs no feature and no database.

/// The workflow, read from the repository rather than embedded, so it cannot drift from the file
/// GitHub actually runs.
fn ci_workflow() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows/ci.yml")
        .canonicalize()
        .expect(".github/workflows/ci.yml must exist — this test is about that file");
    std::fs::read_to_string(path).expect("read .github/workflows/ci.yml")
}

/// The job ids under `jobs:`: the keys indented by exactly two spaces after that line.
fn job_ids(workflow: &str) -> Vec<String> {
    workflow
        .lines()
        .skip_while(|line| *line != "jobs:")
        .skip(1)
        .take_while(|line| line.is_empty() || line.starts_with(' ') || line.starts_with('#'))
        .filter_map(|line| {
            let key = line.strip_prefix("  ")?.strip_suffix(':')?;
            let is_id = !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            is_id.then(|| key.to_string())
        })
        .collect()
}

#[test]
fn the_job_ids_are_read_from_the_real_workflow() {
    let ids = job_ids(&ci_workflow());
    for expected in ["lint", "quality-gates", "postgres-parity", "postgres-tls"] {
        assert!(
            ids.iter().any(|id| id == expected),
            "found no `{expected}` job in ci.yml (read: {ids:?}) — this test reads the wrong file \
             or reads it wrongly, and the checks below would pass for nothing"
        );
    }
}

#[test]
fn ci_on_v2_has_no_managed_postgres_job() {
    let ids = job_ids(&ci_workflow());
    assert!(
        !ids.iter().any(|id| id == "managed-postgres"),
        "ci.yml has a `managed-postgres` job again. The owner turned it off on v2 on 2026-11-15 \
         (NEX-39); a main -> v2 merge must keep it deleted"
    );
}

/// No other job takes over the proof under a new name: nothing builds or runs the `managed-pg`
/// example, and nothing reads the Supabase secret.
#[test]
fn no_job_runs_the_managed_postgres_proof() {
    let workflow = ci_workflow();
    for trace in [
        "--example managed-pg",
        "NXF_SUPABASE_DATABASE_URL",
        "steps.proof.outputs.mode",
    ] {
        assert!(
            !workflow.contains(trace),
            "ci.yml contains `{trace}`: some job runs the managed-Postgres proof, which v2 turned \
             off (NEX-39)"
        );
    }
}
