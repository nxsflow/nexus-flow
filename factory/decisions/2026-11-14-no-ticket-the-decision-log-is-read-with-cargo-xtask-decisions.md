# 2026-11-14 · no ticket · The decision log is read with cargo xtask decisions

- Decided by: setup agent (factory setup PR)
- Why: the repository's automation is a cargo xtask so that the core workflow stays on one toolchain (xtask/Cargo.toml); the reader is a port of nocentraldb's scripts/decisions.mjs with the same options and the same checks, so both logs follow one set of rules. Its check runs in the lint job, beside the layering gate, because it reads text only.
- Instead of: copying scripts/decisions.mjs and running it with node, which would add a second toolchain to a gate.
- Topics: decisions, factory, tooling
