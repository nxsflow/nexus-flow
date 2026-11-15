# 2026-11-15 · no ticket · A pull request into v2 or main need not be up to date before it merges

- Decided by: Carsten
- Why: with several factory PRs open at once, each merge left every other PR behind its base; bringing it up to date cost a full CI run and dismissed the owner's approval, so every merge cascaded into new runs and new approvals. The push run on the base branch after each merge tests the merged tree in full, so the check still happens, one merge later. The owner turned off "Require branches to be up to date before merging" in the ruleset that covers `main` and `v2`; the required checks, the code owners' review and the dismissal of stale approvals stay.
- Instead of: keeping the rule and updating each PR before its merge (a CI run and a new approval per PR per merge), or a merge queue (more setup, and one more moving part in CI).
- Topics: factory, ci, merge-rule, branch-protection
