# 2026-11-17 · NEX-5 · The space id is committed in nexus-flow.toml at the repository root

- Decided by: Carsten
- Why: every clone, worktree and cloud container finds its repository space without being told; a space id grants nothing (RFC 0001, "Creating and finding a space"). Answer 1a in NEX-38.
- Instead of: a committed `.nxs/space.toml` inside the directory v1 ignores in git, or an id given to each session by hand.
- Topics: nocentraldb, spaces, storage
