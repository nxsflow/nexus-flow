# 2026-11-27 · NEX-104 · nxs space join-coordination writes only nexus-flow.toml, and checks no membership

- Decided by: coder on NEX-104; provisional until the owner approves the PR
- Why: RFC 0001 has the command record the space "once an admin of that space has added the repository's pm", which is the order of the steps, not a check: a pm added but not yet synced holds none of the space (NEX-96), so a check would refuse the right call until a sync, and the command would need the key store and a database where it writes only a committed file. So it opens no database, and its seam counterpart is the associated function `Engine::space_join_coordination(repository: &Path, space: SpaceId) -> Result<JoinReport, Error>`, after the owner's decision for seam functions of commands that open no database (NEX-232). The repository record and `--name` (RFC 0009, "Joining") open the coordination space and come with NEX-158.
- Instead of: refusing unless the member a command runs as holds the coordination space; or a method on an open `Engine`, which a CI machine without a key store could not call.
- Topics: seam, nxs, coordination, v2
