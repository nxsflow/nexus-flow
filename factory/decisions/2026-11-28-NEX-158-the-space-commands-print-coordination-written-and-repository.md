# 2026-11-28 · NEX-158 · nxs space create-, join- and leave-coordination print coordination, written and repository

- Decided by: coder on NEX-158; provisional until the owner approves the PR
- Why: RFC 0009 names no JSON for the three commands. Each prints the same object: `coordination`, the space created, joined or left; `written`, whether the run wrote anything, the repository record or `nexus-flow.toml`, so a join that renames only the record reads as written and a repeat as not; and `repository`, the checkout's record as `nxs repository list` prints it (`name`, `space`, `left`), or `null` after leaving a space that held no record of it. `nxs repository list` prints the records in name order, and refuses with `validation` in a repository that names no coordination space; a space not held yet lists nothing. A name another repository's record holds is `conflict`, as a second coordination space is (NEX-104); a database failure is `io`, every other refusal `validation`.
- Instead of: a different object per command, which an agent would have to tell apart; or `written` for the file alone, which reads a rename as "nothing was written".
- Topics: nxs, output, errors, coordination, v2
- Supersedes: 2026-11-27-NEX-104-join-coordination-prints-coordination-and-written.md
