# 2026-11-15 · no ticket · Always give V1Ports in a v2 host test .with_sidecar(..) - the hosted macOS lane embeds no bundle and every session start is refused there.

- Decided by: recorded in the project memory (`nxm`, key `v2-host-tests-name-a-sidecar`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, testing, ci

## The record

A test that steps a v2 run through a real Host over V1Ports must name a sidecar: V1Ports::for_root(..).with_sidecar(<any path>). Without it bundle_path falls back to the bundle build.rs embedded, and the hosted macOS CI lane builds none, so every session start is refused (start_deadline -> StartFailed, the scripted backend never spawns, the view sticks at Running { tree: Prepared }). Linux quality-gates and a developer tree with agent-sidecar/dist built stay green, so it shows only on the full macOS lane. Reproduce locally by moving agent-sidecar/dist/nxc-agent-sidecar.mjs aside and rebuilding. Found on 6j6v.y6v7, run 43417833631.
