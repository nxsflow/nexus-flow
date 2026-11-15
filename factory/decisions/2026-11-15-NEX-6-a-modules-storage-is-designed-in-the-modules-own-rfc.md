# 2026-11-15 · NEX-6 · A module's storage in NoCentralDB is designed in the module's own RFC

- Decided by: pm
- Why: `docs/rfc/README.md` gives each area of GENESIS.md one RFC; for priority 1 each module's RFC (0002 to 0007) is written first for what the module stores in NoCentralDB and the v1 behaviour it carries over, and the behaviour priority 2 adds is later written into the same RFC by rewriting it, so an area never has two RFCs. RFC 0001 sets the frame they share (spaces, members, model, read path, sync); RFC 0008 designs the move from v1. Tickets: NEX-5 to NEX-12.
- Instead of: separate storage RFCs beside each module's behaviour RFC, which would split one area across two texts; or one storage RFC for all six modules, which would design the new modules' data before anyone owns their design.
- Topics: rfc, nocentraldb, storage, factory
