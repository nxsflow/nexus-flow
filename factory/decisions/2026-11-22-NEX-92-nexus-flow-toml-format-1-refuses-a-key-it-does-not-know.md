# 2026-11-22 · NEX-92 · nexus-flow.toml format 1 refuses a key it does not know

- Decided by: coder on NEX-92
- Why: a misspelt key, such as `[git] bsae = "develop"`, would otherwise be dropped in silence and the base branch fall back to `main`, so "on main" would be checked against the wrong branch. `format` is checked first, so a file of a future format is refused as that format, not for its new keys. A new key therefore needs a new format number, or a change to format 1's list in `crates/store/src/config.rs`.
- Instead of: ignoring unknown keys, as many TOML readers do.
- Topics: storage, v2
