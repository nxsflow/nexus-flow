# 2026-11-28 · NEX-248 · A test that names tags gives the device the tag zy after init

- Decided by: coder on NEX-248
- Why: `init` picks the device's tag by its random member id, so a test that names a fixed tag (`ka`, `kb`, …) failed with `TagHeld` whenever the device drew it, about 1 in 936 per tag named. The setups of the tag tests (the store's members test `Repo`, the seam's `coder_space`, the `Repo` of nxs-v2's `member_tag`) retag the device to `zy` right after `init`, through the same `retag` the tests exercise, and each has a test that the device holds `zy`. No test names `zy`; a new tag test names any other tag, or starts from one of these setups.
- Instead of: a test-only hook that makes `init` pick a given tag, which would put a path into production code that no product needs; or reading the device's tag and picking test tags around it, which makes every test's tags depend on the draw.
- Topics: members, tests, v2
