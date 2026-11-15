# 2026-11-15 · no ticket · The board is git-ignored, single-device and lives outside the tree (.nxs is a symlink) — and there is no task type, only the plugin set.

- Decided by: recorded in the project memory (`nxm`, key `the-board-is-local-and-not-in-git`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: flow, board, v1

## The record

This workspace's board is **fully git-ignored** and does NOT travel with the code: `nxf
close`/`create` write locally only, there is no "push the board" step at the end of a session, and
no sync stream is bound (`nxs doctor`: `sync: not bound`, measured 2026-09-18) — the board is
single-device. If you want to share the state, share it through the PR, not through the board.

WHERE IT ACTUALLY LIVES, and the shorthand hides it: `.nxs` in the repo root is a SYMLINK to
`~/.nxs-workspaces/nexus-flow/`, so the database is `~/.nxs-workspaces/nexus-flow/db.sqlite` and
sits outside the repository tree altogether. It is ignored twice over regardless — the repo
`.gitignore` names `.nxs`, and the workspace directory carries its own `.gitignore` of `*`. The
practical consequence of the link: a `git clean`, a fresh clone or a worktree does not touch the
board, and two checkouts of this repo pointing at the same link share ONE board.

The item types are declared by the plugin (`issue-tracker`: `bug|chore|decision|epic|feature`) —
**there is no `task`**. `nxf schema` names the set this workspace actually accepts.

Everything else about the board — the verbs, the core rules, this workspace's id prefix, the
session close — is in `nxf prime` and therefore does not belong here.
