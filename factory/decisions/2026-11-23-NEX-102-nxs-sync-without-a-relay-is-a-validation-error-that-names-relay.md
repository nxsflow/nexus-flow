# 2026-11-23 · NEX-102 · nxs sync without a relay is a validation error that names --relay

- Decided by: pm, on coder's recommendation; provisional until the owner approves the PR
- Why: in island mode there is no relay to wait for. Exiting 0 would claim a sync that did not happen. The error says that no relay is set and that `nxs sync --relay <url>` sets one, the same words `nxs init` uses. `nxs sync --status` still works without a relay, with `relay` `null`.
- Instead of: exiting 0 at once with `relay` `null`.
- Topics: sync, nxs, errors
