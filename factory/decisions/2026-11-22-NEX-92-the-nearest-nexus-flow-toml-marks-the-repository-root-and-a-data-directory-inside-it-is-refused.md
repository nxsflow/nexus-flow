# 2026-11-22 · NEX-92 · The nearest nexus-flow.toml marks the repository root, and a data directory inside it is refused

- Decided by: coder on NEX-92
- Why: RFC 0001, "Reading and writing", finds the root from the working directory by the file, so the first folder upward that holds `nexus-flow.toml` as a file is the root, and a nested repository with a file of its own wins over one further up; the walk does not stop at a `.git` folder, which the RFC does not name. Once the root is found, a data directory that is the root or lies inside it is refused ("The local data on disk"), whatever variable it came from. Both paths are compared as the filesystem resolves them (the longest existing ancestor canonicalized, then `.` and `..` applied), so a symlink or a dotted path back into the root is refused, and `repo/../data` is not.
- Instead of: stopping the walk at the git root, or comparing the paths as written, which a symlink or `..` slips past and which refuses `repo/../data`.
- Topics: storage, v2
