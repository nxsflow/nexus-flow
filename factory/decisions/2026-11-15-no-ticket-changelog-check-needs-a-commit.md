# 2026-11-15 · no ticket · changelog check reads a three-dot diff, so it needs a COMMIT — a correct fragment sitting in the working tree reports as missing.

- Decided by: recorded in the project memory (`nxm`, key `changelog-check-needs-a-commit`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: changelog, gates

## The record

`cargo xtask changelog check origin/main` detects fragments via `git diff --name-only
{base}...HEAD` (three-dot, i.e. merge-base) — see `xtask/src/changelog.rs::git_changed_paths`. On a
fresh feature branch with **no commits yet**, `origin/main...HEAD` is empty, so a correctly formed
`changes/<slug>.md` in the working tree — even staged — reports "this PR touches no changes/*.md
fragment". This is not a fragment-format problem: you simply have to make the commit first, and
then the check goes green.

It matters on every PR here because the `changelog-check` CI gate is fail-closed. It is also the
reason a green `changelog check` can be TRUE AND WORTHLESS if it ran before the fragment was
committed — see [gate-evidence-is-per-commit](2026-11-15-no-ticket-gate-evidence-is-per-commit.md).
