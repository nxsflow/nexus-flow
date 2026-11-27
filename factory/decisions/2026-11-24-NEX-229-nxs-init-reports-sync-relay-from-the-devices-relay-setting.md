# 2026-11-24 · NEX-229 · nxs init reports sync relay from the device's relay setting

- Decided by: coder on NEX-229; provisional until the owner approves the PR
- Why: RFC 0001 names two sync modes, island and a relay, and `nxs init`'s `sync` field (NEX-97) had only `island`. Once `nxs sync --relay` has set a relay, the field is `relay`, read from the relay the open actually used: the device's `device.toml`. The value names the mode only, not the URL, since `nxs sync --status` already prints the relay and a URL may carry a host a team keeps private. With `--md`, the line says the space syncs through the relay set on this device and that `nxs sync` waits until nothing is behind or unconfirmed.
- Instead of: printing the relay's URL as the value of `sync`, which would mix a mode and a setting in one field; or a separate `relay` field, which `nxs sync --status` already has.
- Topics: sync, nxs, output
