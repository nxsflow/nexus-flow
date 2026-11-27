# 2026-11-24 · NEX-133 · The init gate does not read the seam's suites

- Decided by: coder on NEX-133
- Why: the gate `every_test_that_runs_init_pins_a_home_first` flags any test line that hands `"init"` as an argument, to catch v1's `nxf init`/`nxm init` run without a pinned `$HOME`. The seam's `genesis_create` suite runs `git init` to build the repositories `genesis create` reads, and the seam links nothing of v1, so none of its suites can run v1's `init`. `crates/seam/tests` joins `crates/nxs-v2/tests` in `V2_INIT_SUITES`, for the same reason NEX-97 gave. The git calls in the genesis suites set `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`, so the developer's git configuration cannot change them.
- Instead of: pinning a home through `nxs_test_support`, a dev-dependency that changes `Cargo.lock` (NEX-212); or spelling `git init` so that the text gate misses it, which hides the call instead of answering the gate.
- Topics: testing, gates, seam, v2
