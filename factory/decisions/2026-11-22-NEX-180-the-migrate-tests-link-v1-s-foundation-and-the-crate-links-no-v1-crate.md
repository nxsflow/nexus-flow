# 2026-11-22 · NEX-180 · The migrate tests link v1's foundation, and the crate links no v1 crate

- Decided by: pm, on coder on NEX-180's proposal; the owner approves it with the PR, since it changes Cargo.lock
- Why: NEX-180's acceptance asks that a v1 build's `assess_open` refuse a marked workspace, and only v1's own code proves that, so `nxs-foundation` is a dev-dependency of `nexus-flow-migrate` (one line in Cargo.lock, no new crate). The boundary stays tested: `tests/skeleton.rs` keeps the normal dependency list as strict as before, allows `nxs-foundation` under `[dev-dependencies]` only, and checks that the crate has no normal dependency on any v1 crate. The move reads v1's data by its format, never through v1's code.
- Instead of: (a) re-stating `assess_open`'s rule in the migrate tests, which tests a copy of v1 and not v1; or (b) a test in `crates/foundation`, which cannot call the move's code without the reverse dependency and would check only a header raised by hand.
- Topics: migration, testing, dependencies
