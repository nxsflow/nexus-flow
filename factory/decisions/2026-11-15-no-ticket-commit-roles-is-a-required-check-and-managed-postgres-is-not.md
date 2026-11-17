# 2026-11-15 · no ticket · commit-roles is a required check, and managed-postgres is not

- Decided by: Carsten
- Why: `commit-roles` (NEX-37) only guards anything when a missing `Role:` trailer blocks the merge. `managed-postgres` is removed from `v2`'s CI (NEX-39), and on `main` it fails or is cancelled whenever several runs hit the shared Supabase database at once, so as a required check it blocked merges for reasons unrelated to the change. The owner changed both in the ruleset "main and v2: every change through a pull request", which covers both branches.
- Instead of: a separate ruleset for `v2` (pm's recommendation, so that `commit-roles`, which runs only on PRs against `v2`, is never awaited on a PR against `main`); the owner chose to keep one ruleset and fix it if a PR against `main` waits for the check.
- Topics: factory, ci, branch-protection, merge-rule
