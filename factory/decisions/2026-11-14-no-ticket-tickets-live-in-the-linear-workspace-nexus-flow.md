# 2026-11-14 · no ticket · Tickets live in the Linear workspace nexus-flow

- Decided by: Carsten (setup brief for the cloud environment nexus-flow, 2026-11-13)
- Why: the factory runs in cloud sessions, where nexus-flow is not installed, so its own board cannot be the tracker; Linear outlives every session. The workspace is reached through its GraphQL API at api.linear.app, because the claude.ai Linear connector points at the nocentraldb workspace.
- Instead of: the nexus-flow board (`nxf`), which is local, git-ignored and single-device; or the claude.ai Linear connector.
- Topics: tracker, factory, cloud
