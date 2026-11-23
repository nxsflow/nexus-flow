# 2026-11-23 · NEX-96 · A checkout named to open_space must hold nexus-flow.toml

- Decided by: coder on NEX-96
- Why: "on main" is checked against the base branch `nexus-flow.toml` names (RFC 0001, "Derived state"), so `Engine::open_space(space, Some(git))` finds the repository's root from `git` as every command does, reads its base branch from the file, and refuses a data directory inside that root. A folder with no file above it is refused with the error that names `nxs init`. Without `git` the engine holds no checkout, and every "on main" value is `null`.
- Instead of: taking `main` as the base for a folder without the file, which would check "on main" against a branch the team may not use.
- Topics: storage, seam, v2
