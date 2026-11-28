# 2026-11-28 · NEX-158 · The init gate does not read the storage crate's suites

- Decided by: coder on NEX-158
- Why: the gate that every suite running `init` pins a `$HOME` guards against v1's `nxf init` and `nxm init` registering a workspace under the home. The storage crate is v2's and links nothing of v1. Its suites call the store's `init` as a function with the data directory named, and the only `init` verb they run is `git init`, which the coordination suite needs so that a scratch repository has a remote `origin` to take its name from. So `crates/store/tests` joins the gate's list of v2 suites, as the seam's and the merge gate's did (NEX-133, NEX-171).
- Instead of: pinning a home in the store's suites, which would need `nxs_test_support`, a dependency the store does not take; or writing the scratch repository's `.git/config` by hand to dodge the text match.
- Topics: tests, ci, v2
