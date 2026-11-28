# 2026-11-28 · NEX-173 · The seam's decide tests live in nxs-v2's suite

- Decided by: coder on NEX-173
- Why: the tests of `Engine::merge_gate_decide` build verdicts, comments and reviews as JSON and change them case by case, which needs `serde_json`. The seam does not link it, and a dev-dependency would change the code-owned `Cargo.lock` (NEX-212). `nxs-v2` links `serde_json` and the seam already, so `crates/nxs-v2/tests/engine_merge_gate_decide*.rs` call the seam function directly, beside the command's own tests in `merge_gate_decide.rs`.
- Instead of: a dev-dependency on `serde_json` in the seam, which would make this pull request wait for the owner's review of the lockfile; JSON written by hand in format strings, which hides each case's change in string surgery.
- Topics: testing, merge-gate, dependencies, v2
