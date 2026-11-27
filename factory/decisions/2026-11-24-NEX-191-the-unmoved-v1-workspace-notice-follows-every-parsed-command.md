# 2026-11-24 · NEX-191 · The unmoved v1 workspace notice follows every parsed command

- Decided by: coder on NEX-191
- Why: RFC 0008, "The installed base", has `nxs` say on stderr, not in its JSON, that a `.nxs/` without `.nxs/migrated.toml` has not moved and that `nxs migrate --check` shows the move. `nxs` adds the line after any command it parsed, a failed one included, so a v1 user who runs `nxs sync` before `nxs init` learns of the move beside the `no_workspace` error; help, the version and the usage errors get none. The root is found as RFC 0001 finds it, the nearest folder that holds `nexus-flow.toml`, else the working folder as `nxs init` takes it; `nxs-v2` walks the folders itself, a few lines, since the store's `RepoRoot` is not on the seam and the seam's public surface is the owner's. The line goes out with the rest of the outcome, so `nxs sync --watch`, which runs until it is stopped, does not print it.
- Instead of: re-exporting the store's `RepoRoot` from the seam, which widens the library seam for one check; or the notice only on success, which says nothing to the user whose first v2 command fails.
- Topics: v2, migration, output
