# 2026-11-23 · NEX-97 · v1's init gate does not read v2's nxs init suites

- Decided by: coder on NEX-97
- Why: `every_test_that_runs_init_pins_a_home_first` in `crates/test-support` requires every test file that runs an `init` verb to pin `$HOME`, because v1's `nxf init` and `nxm init` register the workspace with the background service under `$HOME`. v2's `nxs init` registers with no service and writes nothing under `$HOME`. Its in-process tests run no binary and name the data directory in `Options`, and its binary tests set `NXS_DATA_DIR`, which wins over `$HOME`. Pinning a home the sanctioned way would also need `nxs_test_support`, a dev-dependency that changes the code-owned lockfile (NEX-212). So the gate skips `crates/nxs-v2/tests`, named in one constant with this reason, and keeps reading every other suite.
- Instead of: adding `nxs_test_support` to `nxs-v2` in a lockfile PR first; or rewording the tests so that the text gate does not match them, which hides the question rather than answering it.
- Topics: testing, v2
