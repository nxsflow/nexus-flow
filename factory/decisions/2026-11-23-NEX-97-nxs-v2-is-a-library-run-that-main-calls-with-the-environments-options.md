# 2026-11-23 · NEX-97 · nxs-v2 is a library run that main calls with the environment's options

- Decided by: coder on NEX-97
- Why: the binary keeps the device's keys in the key store only (NEX-224), and a CI machine has none, so a test of `nxs init` that runs the binary could never succeed there. `nxs_v2::run(args, dir, options)` parses the command line, runs it through the seam and returns stdout, stderr and the exit code; `main.rs` calls it with `Options::from_env`, and the tests call it with a key file in a scratch data directory. One binary test still runs the real binary, for a path that fails before the key store. The seam's futures run on the calling thread (`block_on`), as in the seam's tests: a database without a relay needs no runtime, and `nxs-v2` adds no dependency.
- Instead of: tests that run only the binary, which would either touch a developer's real key store or need a key-file switch on the binary that NEX-224 rules out.
- Topics: v2, testing
