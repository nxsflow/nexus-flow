# 2026-11-23 · NEX-227 · Engine::subscribe yields a Change per fold, and waits for NoCentralDB

- Decided by: Carsten
- Why: NoCentralDB 0.1.0-beta.1 does not expose a change's origin (local or remote) to embedders, and publishes no test relay; its origin and fold feed are crate-private. So nexus-flow asks NoCentralDB for a public change origin and a test relay or harness, and parks NEX-105 (`Engine::subscribe`) and NEX-228 (the end-to-end sync proof) until both ship. The stream's item type is `Change { space, origin }`, one per fold, with bursts coalesced. RFC 0001's seam table, R3 and Decisions say so.
- Instead of: building `Engine::subscribe` now on query pages without the origin, whose item type would change later; NoCentralDB's query pages as the item type; dropping the origin from the RFC.
- Topics: nocentraldb, seam, sync, v2
