# 2026-11-15 · no ticket · Always work on a feat/-style branch — NEVER commit to main directly unless the user has explicitly allowed it for that piece of work.

- Decided by: recorded in the project memory (`nxm`, key `branching`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: git, process

## The record

**Always work on a `feat/`-style branch — NEVER commit to `main` directly**, unless the user has
explicitly allowed it for that piece of work. Branch *before* the first change:

```bash
git checkout -b feat/<short-slug>   # at the start of the work, off an up-to-date main
```

Work lands on `main` through a pushed branch + PR (the recent history is all PR merges). This is a
hard rule: if you find yourself on `main` with uncommitted work, create the branch first, then
commit. Explicit permission is per-task, not standing — ask again next time.
