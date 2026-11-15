# 2026-11-15 · NEX-17 · v1's own sync retires for NoCentralDB's relay, which manufakt.io hosts

- Decided by: Carsten
- Why: in v2 the data lives in NoCentralDB spaces, whose relay cannot read content; `nxf-relay`, the sync streams and the background sync service have nothing left to carry. Island mode works without a relay; manufakt.io hosts the relay for those who sync.
- Instead of: keeping `nxf-relay` beside NoCentralDB, or nexus-flow hosting a relay itself.
- Topics: nocentraldb, sync, relay, manufakt
