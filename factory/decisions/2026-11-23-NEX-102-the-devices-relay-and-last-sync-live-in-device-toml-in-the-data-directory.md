# 2026-11-23 · NEX-102 · The device's relay and last sync live in device.toml in the data directory

- Decided by: coder on NEX-102
- Why: RFC 0001 puts the member's relay "in that member's own configuration in the data directory, not in `nexus-flow.toml`" without naming the file. `device.toml` sits beside `device/`, the database folder the SDK owns, so nxs never writes into the SDK's folder. It holds `relay` (a `ws://` or `wss://` URL, the only ones NoCentralDB opens) and `synced_at` (seconds since the Unix epoch). It refuses a key it does not know, as `nexus-flow.toml` does (NEX-92). Every database open with the network reads `relay` from it.
- Instead of: a file inside `device/`, which the SDK owns; or a second file for `synced_at`, one more file for one number.
- Topics: sync, store
