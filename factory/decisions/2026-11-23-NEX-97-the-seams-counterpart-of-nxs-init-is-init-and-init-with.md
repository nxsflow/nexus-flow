# 2026-11-23 · NEX-97 · The seam's counterpart of nxs init is the free functions init and init_with

- Decided by: coder on NEX-97
- Why: every `nxs` verb has a counterpart on the seam (the parity decision RFC 0001 keeps in force), and `nxs-v2` links only the seam, but RFC 0001's table names none for `nxs init`. `init` runs before any space exists, so it is not a method of an open `Engine`: `nexus_flow_seam::init(dir)` takes the options from the environment as `Engine::open` does, and `init_with(dir, options)` names them, as `open_with` does. Both return the store's `InitReport`, which the seam re-exports with `SyncMode`. The seam also re-exports `Platform`, without which no caller can build the `DataDir` that `Options` takes, and `RootError`, `ConfigError` and `Problem`, the types `InitError`'s variants already carry, so that `nxs` can map each error to a stable code.
- Instead of: an `Engine::init` constructor, which would live in `engine.rs` and read as if an engine were open; or `nxs-v2` reaching the store directly, which its skeleton test forbids.
- Topics: seam, v2
