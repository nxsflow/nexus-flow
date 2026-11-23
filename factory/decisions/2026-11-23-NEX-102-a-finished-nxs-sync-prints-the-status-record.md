# 2026-11-23 · NEX-102 · A finished nxs sync prints the --status record

- Decided by: pm, on coder's recommendation; provisional until the owner approves the PR
- Why: RFC 0001 names the JSON of `nxs sync --status` (`member`, `relay`, `unconfirmed`, `synced_at`) and none for a plain `nxs sync`. Printing the same record once the wait ends gives an agent one shape to read, with `synced_at` just recorded.
- Instead of: printing nothing and exiting 0, which would make an agent run `--status` to learn the time it synced.
- Topics: sync, nxs
