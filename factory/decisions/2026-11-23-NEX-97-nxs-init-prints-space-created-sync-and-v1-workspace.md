# 2026-11-23 · NEX-97 · nxs init prints space, created, sync and v1_workspace

- Decided by: coder on NEX-97
- Why: RFC 0001 names no JSON fields for `nxs init`, and the ticket asks it to print the space id. The record carries the store's `InitReport` as it stands: `space`, the id; `created`, so a second run reads as "nothing was created"; `sync`, `island` while no relay is set; and `v1_workspace`, because RFC 0001's "Migration" has `nxs init` say that the v1 data in `.nxs/` was not moved. Keys are snake_case and in that order, as v1's are. `--md` prints the same as a list of sentences, with file names, ids and commands as code. The fields are a surface the owner reviews in this PR.
- Instead of: only `space`, which leaves an agent unable to tell a new space from an existing one or to see a v1 workspace left behind, without a second command.
- Topics: v2, output
