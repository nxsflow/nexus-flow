# 2026-11-27 · NEX-104 · nxs space join-coordination prints coordination and written, and a second space is a conflict

- Decided by: coder on NEX-104; provisional until the owner approves the PR
- Why: RFC 0001 names no JSON for the command. It prints `coordination`, the space the file names now, and `written`, so that a run that found the space already recorded reads as "nothing was written", as `created` does for `nxs init` (NEX-97). A file that names another coordination space is v1's `conflict`, the code `nxs init` gives a file it will not overwrite; the repository's own space, a file layout `nxs` cannot edit and a text that is no space id are `validation`; no `nexus-flow.toml` is `no_workspace`.
- Instead of: printing nothing, or only the id, which leaves an agent unable to tell a fresh join from a repeat; or a new code such as `already_joined`, which v1 does not have.
- Topics: nxs, output, errors, coordination, v2
