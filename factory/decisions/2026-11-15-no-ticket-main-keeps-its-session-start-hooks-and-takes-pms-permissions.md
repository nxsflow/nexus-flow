# 2026-11-15 · no ticket · main keeps its SessionStart hooks, and takes pm's cloud-session permissions as an exception

- Decided by: Carsten
- Why: v1 bug fixes on `main` still run with nxs, so the `nxf`/`nxc`/`nxm prime` hooks stay there; cloud sessions start on `main` before the setup script moves them to `v2`, so pm's permissions (sessions, scheduled tasks, PR subscriptions, GitHub) are allowed in `.claude/settings.json` on both branches. On `v2` the hooks go, since a v2 cloud session has no nxs.
- Instead of: keeping `main` free of every factory PR, as CONFIG.md says, which would leave pm unable to create its scheduled tasks if a session reads the settings of `main`.
- Topics: factory, permissions, cloud, v2
