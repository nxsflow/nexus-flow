# 2026-11-24 · NEX-171 · v1's init gate does not read the seam's and the merge gate's suites

- Decided by: coder on NEX-171
- Why: `every_test_that_runs_init_pins_a_home_first` reads test sources as text and counts a quoted `"init"` beside an argument call as a run of v1's `nxf init` or `nxm init`, which register with the background service under `$HOME`. The merge gate's tests build scratch git repositories with `git init`, so `crates/merge-gate/tests` and `crates/seam/tests` matched, though they run no `init` of nexus-flow's. They are v2 suites, which register with no service, and pinning a home the sanctioned way needs `nxs_test_support`, a dev-dependency that changes the code-owned lockfile (NEX-212). So, as NEX-97 did for `crates/nxs-v2/tests`, the gate's list of v2 suites names them, with this reason, and keeps reading every v1 suite.
- Instead of: spelling `git init` so that the text gate does not match it, which hides the question rather than answering it (NEX-97); or adding `nxs_test_support` to two v2 crates in a lockfile PR first.
- Topics: testing, v2
