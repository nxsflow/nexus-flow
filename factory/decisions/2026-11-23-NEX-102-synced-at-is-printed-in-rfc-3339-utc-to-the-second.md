# 2026-11-23 · NEX-102 · synced_at is printed in RFC 3339 UTC to the second

- Decided by: pm, on coder's recommendation; provisional until the owner approves the PR
- Why: RFC 0001 names `synced_at` but not its form. A time such as `2026-11-23T16:20:05Z` reads the same for a person and an agent, and sorts as text. It is `null` until a sync first finds nothing behind or unconfirmed. `device.toml` keeps it as seconds since the Unix epoch, and nxs formats it itself, so no date crate is added.
- Instead of: seconds since the Unix epoch in the JSON, which a person cannot read.
- Topics: sync, nxs
