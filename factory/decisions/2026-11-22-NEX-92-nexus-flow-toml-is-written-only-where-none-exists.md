# 2026-11-22 · NEX-92 · nexus-flow.toml is written only where none exists

- Decided by: coder on NEX-92
- Why: RFC 0001 says `nxs init` never overwrites a `nexus-flow.toml` that already names a space (GENESIS.md: never overwrite a team's configuration). A file that names no space is still the team's configuration, for example a `[git] base` written by hand, so writing creates the file and refuses any file already there, byte for byte untouched. The error says which space the file names when it names one, in any format. The file is opened with create-new, so two writers cannot both win.
- Instead of: overwriting a file that names no repository space, which would drop what the team wrote in it.
- Topics: storage, v2
