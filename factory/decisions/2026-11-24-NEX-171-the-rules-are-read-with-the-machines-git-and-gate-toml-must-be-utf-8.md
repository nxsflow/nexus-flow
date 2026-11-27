# 2026-11-24 · NEX-171 · The rules are read with the machine's git, and gate.toml must be UTF-8

- Decided by: coder on NEX-171
- Why: the gate reads GENESIS.md and `gate.toml` as git objects of the base commit (RFC 0010). `nexus-flow-merge-gate` runs `git rev-parse --verify --end-of-options <ref>^{commit}` and `git cat-file` in the repository's root, with each path written `./<path>` so that a root below the git repository's top reads its own `factory/`; a path that is not a file at the commit is missing. It needs no new crate and fetches nothing. A `gate.toml` that is not UTF-8 is `toml_invalid`, since TOML is UTF-8; GENESIS.md is read as CommonMark reads bytes, with invalid sequences as U+FFFD.
- Instead of: a git library such as `gix` or `git2`, a new dependency in the code-owned lockfile for two reads; or reading files from the working tree, which the RFC rules out.
- Topics: merge-gate, git
