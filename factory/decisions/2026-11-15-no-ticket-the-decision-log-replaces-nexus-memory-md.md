# 2026-11-15 · no ticket · The decision log replaces NEXUS_MEMORY.md

- Decided by: Carsten
- Why: GENESIS.md makes a durable rule a decision in force and writes no memory file. Every memory of NEXUS_MEMORY.md is now a decision file of its own, dated 2026-11-15, with its full text under "The record"; the four written in German are translated. NEXUS_MEMORY.md and the managed nexus-flow block in AGENTS.md are removed. The rules in force are the current decisions: `cargo xtask decisions --current --index`.
- Instead of: keeping NEXUS_MEMORY.md, which nxm generates and nobody can change in a cloud session.
- Topics: decisions, memory, factory
