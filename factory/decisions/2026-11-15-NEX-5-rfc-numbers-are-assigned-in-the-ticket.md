# 2026-11-15 · NEX-5 · RFC numbers are assigned in the ticket, not taken at PR time

- Decided by: pm
- Why: the RFCs for the move to NoCentralDB (NEX-5 to NEX-12) are drafted in parallel lanes, and "the next free number" would collide; each ticket names its RFC's number and file (0001 nexus-flow on NoCentralDB, 0002 tickets, 0003 decisions, 0004 genesis, 0005 goals, 0006 factory, 0007 status, 0008 the move from v1).
- Instead of: each draft taking the next free number when its PR opens, as `docs/rfc/README.md` describes for a single draft.
- Topics: rfc, nocentraldb, factory
