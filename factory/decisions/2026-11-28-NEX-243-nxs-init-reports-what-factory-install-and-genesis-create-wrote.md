# 2026-11-28 · NEX-243 · nxs init reports what factory install and genesis create wrote

- Decided by: Carsten
- Why: `nxs init` runs `nxs factory install` and then `nxs genesis create` (RFC 0006, RFC 0004), which write files into the repository that a person must commit, and no RFC said what `nxs init` prints about them. Its JSON now carries the two commands' own reports under the keys `factory` and `genesis`, exactly as each command prints them, and `--md` adds a line for each. RFC 0006, "Installing the shipped defaults", and RFC 0004 say so.
- Instead of: (b) keeping `nxs init`'s output as it was (space, created, sync, v1_workspace), which leaves `git status` as the only way to see what to commit.
- Topics: factory, genesis, init, v2
