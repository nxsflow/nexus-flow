# 2026-11-23 · NEX-102 · Engine::sync_status opens the database off the network, and set_relay sets the relay

- Decided by: coder on NEX-102
- Why: RFC 0001's table names `Engine::sync` and `Engine::sync_status`.
  - **`Engine::sync(&mut self, timeout)`** waits on an open engine, since an open database is what syncs.
  - **`Engine::sync_status(repository)`, with `sync_status_with(repository, options)`,** opens the database itself, without the relay. `nxs sync --status` must make no network call, and an engine opened as usual would start its relay connection. It takes the same arguments as `Engine::open` and `open_with`.
  - **`set_relay(&options, &relay)`,** a free function as `init` is (NEX-97), sets the device's relay for `nxs sync --relay`, since `nxs-v2` links only the seam.
  - **Re-exports:** the seam re-exports `RelayUrl`, `NotARelayUrl`, `SettingsError`, `SyncStatus`, `SyncError`, `Behind` and `SpaceBehind`.
  - **`Error::Sync`** carries the two sync failures that are not an open's: no relay, and a timeout.
- Instead of: `sync_status(&self)` on an open engine, which would go online first; or an `Options` field to open offline, which would change every caller of `Options` for one command.
- Topics: seam, sync
