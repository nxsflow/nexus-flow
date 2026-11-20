# 2026-11-20 · NEX-74 · A PR that changes only docs and decisions needs only the PR checks

- Decided by: Carsten
- Why: The full gate tests code: macOS, release builds and the database backends. A PR that changes only `docs/` and `factory/decisions/` changes none of it, and the PR checks already cover what it touches. Several RFC PRs edit the same table in `docs/rfc/README.md`, so every merge forces the others to take in the base branch and be approved again; waiting about 45 minutes for the full gate each time multiplied those rounds. pm now merges such a PR as soon as the owner approves it and its PR checks are green. Recorded in CONFIG.md, "Gate command".
- Instead of: the full gate before every merge, which stays the rule for every PR that touches anything else.
- Topics: factory, merge, ci
