# 2026-11-15 · no ticket · Where a session stopped belongs in the session-handover skill, never in durable memory.

- Decided by: recorded in the project memory (`nxm`, key `handover-notes-are-not-durable-memory`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: process, memory

## The record

Handover notes for the next session belong in the `session-handover` skill, NOT in durable
memory.

**Why:** durable memory is reserved for long-lived project knowledge. Carsten corrected me when I
parked a "what comes next in this plugin epic" note there. Where a session left off is a different
kind of information and has its own mechanism.

**How to apply:** when a session ends, write the handover through the `session-handover` skill.
Durable memory stays for facts worth recalling across many future sessions — not for "where I
stopped".
