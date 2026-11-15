# 2026-11-15 · NEX-22 · A workspace moves to NoCentralDB with an explicit `nxs migrate`

- Decided by: Carsten
- Why: the team chooses the moment; the command leaves the v1 data untouched on disk, which is the rollback, writes a marker so a v1 binary refuses further writes and says why, and is safe to run again.
- Instead of: moving automatically on the first v2 run, or keeping both copies writable for a while.
- Topics: nocentraldb, migration, cli
