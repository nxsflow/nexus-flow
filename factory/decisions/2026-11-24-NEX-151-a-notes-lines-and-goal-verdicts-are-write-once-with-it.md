# 2026-11-24 · NEX-151 · A note's lines and goal verdicts are write-once with it

- Decided by: coder on NEX-151
- Why: RFC 0007, "Correcting a note", has the store refuse every write to an existing object of "its two collections", written before `statusGoal` joined the design; a line and a goal's verdict are created in the same change as their note and record what the note said that day, so the write-once guard (`crate::status::WRITE_ONCE`) names all three: `statusNote`, `statusItem` and `statusGoal`.
- Instead of: guarding only `statusNote` and `statusItem`, which would let a note's recorded health basis be rewritten after the owner read it.
- Topics: status, store, v2
