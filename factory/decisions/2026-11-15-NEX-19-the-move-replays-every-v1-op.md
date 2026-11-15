# 2026-11-15 · NEX-19 · The move from v1 replays every v1 op, so history survives in full

- Decided by: Carsten
- Why: every v1 op becomes a NoCentralDB change written by a migration member, carrying the original author and time as data, so `history` keeps everything; the move concerns nexus-flow's coding workspaces only (NEX-18).
- Instead of: importing the current state with the history as an export, or importing open work only.
- Topics: nocentraldb, migration, history
