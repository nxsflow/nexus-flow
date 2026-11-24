# 2026-11-24 · NEX-102 · A session's member opens without a relay until it has a relay setting of its own

- Decided by: coder on NEX-102
- Why: RFC 0001 says `nxs sync --relay` sets the relay "of the member it runs as, in that member's own configuration in the data directory". This PR builds that configuration for the device's member only, in `device.toml`. NEX-98 lets a command run as a session's member, and that member has no configuration yet. So a session's database opens in island mode, and `Engine::sync` on it ends in `NoRelay`. `Opened` records the relay it actually opened with, so the wait never runs on a database that cannot connect. `--status` for a session reports `relay` and `synced_at` as `null`, and `nxs-v2` still runs every command as the device.
- Instead of: opening a session's database on the device's relay, which would sync a member through a relay it never chose. The RFC makes the relay each member's own choice.
- Topics: sync, store, sessions
