# 2026-11-15 · NEX-36 · CI skips the heavy jobs on a docs-only change

- Decided by: the coding agent on NEX-36, on Carsten's request of 2026-11-15 (seen on #148)
- Why: a pull request that changes only RFCs, factory files, decisions, research notes or change fragments ran every heavy job for tens of runner minutes, though nothing those jobs build or test reads those files.
- Instead of: a job-level `if:` or a `paths:` filter. A skipped required check reports green with no reason, and a `paths:` filter names the paths that matter, so it fails open as the code grows.
- Topics: ci, gates, docs

## The rule

`.github/scripts/docs-only.sh` decides, and is the only place the rule is written; `tests/docs-only.test.sh` tests it. Every heavy job in `ci.yml` (`quality-gates`, both platform gates, the macOS test lane, `postgres-parity`, `postgres-tls`, `managed-postgres`, `dynamodb-local`, `relay-install-dynamodb`) and `facade-semver.yml` calls it as its first step after the checkout. On a docs-only change the later steps are skipped by that step's output, and the job ends green under its own name, with a `::notice::` and a job-summary line naming what was not run.

A change is docs-only when every path in its diff (renames counted as both paths) is under one of:

- `docs/rfc/`
- `docs/research/`
- `factory/`
- `changes/`

No `include_str!`, `include_bytes!` or `include_dir!`, no build script, no test, no golden harness and no xtask check run by a heavy job reads them. Two cheap checks read two of them and keep running on every change: `lint`'s `cargo xtask decisions --check` (`factory/decisions/`) and `changelog-check` (`changes/`). The rest of `docs/` stays out: `docs/generated/` is checked against the clap tree, and nothing proves the others unread. A path is added to the list only with the same proof.

Only a `pull_request` (the merge commit against its first parent), a push to `v2` (against the previous commit) and a `workflow_dispatch` on any branch but `main` and `release/*` (against the tip of `v2`) may take the docs-only path. A push to `main` or a maintenance line, `full=true`, an empty change, an unknown event and any git failure run in full. `lint`, `changelog-check`, `version-check` and `dependency-audit` always run.
