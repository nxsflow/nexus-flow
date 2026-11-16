# 2026-11-16 · no ticket · v2 has its own ruleset that requires commit-roles

- Decided by: Carsten
- Why: `commit-roles` runs only on PRs against `v2`, so once it was required in the shared ruleset "main and v2: every change through a pull request", a PR against `main` (#146) waited for a check that never runs. As decided on 2026-11-15, the ruleset was fixed when that happened: a new ruleset for `v2` alone requires `commit-roles` along with the other checks, and the shared ruleset no longer requires it, so `main` needs its previous six checks. `managed-postgres` stays off the required checks, and a PR still need not be up to date before it merges.
- Instead of: keeping one ruleset for both branches, which blocks every PR against `main`.
- Topics: factory, ci, branch-protection, merge-rule
- Supersedes: 2026-11-15-no-ticket-commit-roles-is-a-required-check-and-managed-postgres-is-not.md
