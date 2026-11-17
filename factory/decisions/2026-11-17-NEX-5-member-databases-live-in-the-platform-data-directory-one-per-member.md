# 2026-11-17 · NEX-5 · Member databases live in the platform's data directory, one per member, outside the repository

- Decided by: Carsten
- Why: a branch switch, a second worktree or a fresh clone opens the same database and loses nothing; one database per member is NoCentralDB's design (RFC 0001, "The local data on disk"). Answer 2a in NEX-38.
- Instead of: the databases inside the repository's `.nxs/`, as in v1, where every fresh clone and worktree syncs again.
- Topics: nocentraldb, storage
