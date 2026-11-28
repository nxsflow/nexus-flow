# 2026-11-28 · NEX-158 · A coordination command checks everything first, writes the record, then nexus-flow.toml

- Decided by: coder on NEX-158
- Why: the record and the file live in two places that no one change covers. So create, join and leave first check the file's edit (a second coordination space, the repository's own space, a layout nxs cannot edit), the name and that the space is held; a create checks the edit with a placeholder id before the space exists, so a refusal creates no space. Then the record is written, and the file last. A run that fails between the two leaves a record without the file line, and running the command again finds the record and writes the file. A join or a leave changes every record whose `space` is the repository's, since two developers joining offline at once can leave two.
- Instead of: the file first, which leaves a repository that names a space its record is missing from; or one record only, which leaves the second of an offline pair unrevived.
- Topics: coordination, store, v2
