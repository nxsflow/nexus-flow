# 2026-11-15 · NEX-39 · CI on v2 runs no managed-postgres job

- Decided by: Carsten
- Why: the job proves v1's Postgres relay against one shared Supabase project; on `v2` storage moves to NoCentralDB (RFC 0001), so the proof no longer serves `v2`'s target, while the project's rate limit and the job's repository-wide concurrency group turned unrelated pull requests red or cancelled.
- Instead of: keeping the job on `v2` and living with the red and cancelled checks, or disabling it with `if: false`, which still shows a skipped `managed-postgres` check on every pull request.
- Topics: ci, gates, storage, v2

## What changes, and what stays

`v2`'s `.github/workflows/ci.yml` has no `managed-postgres` job, for any event: pull requests, pushes to `v2` and dispatches with `full=true`. The job is deleted, not disabled. A comment stands at its place, and `crates/server/tests/ci_runs_no_managed_postgres.rs` (run by `quality-gates`' `cargo test --all`) fails when the job, the `managed-pg` example or the Supabase secret comes back into the file.

`main` keeps the job unchanged. v1 releases are cut from `main`, and its Postgres relay still ships. When `main` is merged into `v2`, the merge keeps the job deleted: a conflict in that block resolves to the deletion.

Also unchanged on `v2`: `postgres-parity` and `postgres-tls`, which need no external service; the `managed_pg` module and the `managed-pg` example in `crates/server`, unit-tested there, because `main` still runs them; and the runbook `docs/runbooks/managed-postgres-supabase.md`, which now says the job runs on `main` only. The NEX-36 decision's list of heavy jobs names `managed-postgres`; on `v2` that entry no longer applies, and the docs-only rule is otherwise unchanged.
