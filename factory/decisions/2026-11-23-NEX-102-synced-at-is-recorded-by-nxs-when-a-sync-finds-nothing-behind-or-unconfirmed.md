# 2026-11-23 · NEX-102 · synced_at is recorded by nxs when a sync finds nothing behind or unconfirmed

- Decided by: coder on NEX-102, as pm directed on the ticket
- Why: RFC 0001 has `nxs sync --status` report `synced_at`, but NoCentralDB 0.1.0-beta.1's `SyncStatus` and `SpaceStatus` carry no time of the last sync. So nxs records it in `device.toml`: `Engine::sync` writes it each time it finds nothing behind or unconfirmed. A plain command or a sync that times out leaves it as it was, and it is `null` until the first full sync.
- Instead of: waiting for NoCentralDB to report the time, which would leave `synced_at` always `null`; or stamping it whenever a database is open and connected, which would claim a sync that never caught up.
- Topics: sync, store
