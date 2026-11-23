# 2026-11-23 · NEX-94 · init takes the folder it runs in as the root when no nexus-flow.toml is found

- Decided by: coder on NEX-94
- Why: RFC 0001 finds the root by `nexus-flow.toml`, which a repository without a space does not have yet, so `init` first looks upward from its folder as every other command does: when a file is found, its space is the repository's, nothing is created, and the report names that id; only when none is found does the folder `init` runs in become the root (`RepoRoot::at`), where the new file is written. The data directory is checked against that root before the device's database is created. The second-admin recommendation is given only when this call created the space, since only then is the device known to be its only admin.
- Instead of: stopping at the git root to place the file (the RFC names no `.git`, and NEX-92's walk does not stop at it either), or recommending a second admin on every call without knowing the space's admins.
- Topics: storage, v2
