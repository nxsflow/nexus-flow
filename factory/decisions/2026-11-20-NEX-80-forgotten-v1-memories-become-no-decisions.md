# 2026-11-20 · NEX-80 · Forgotten v1 memories become no decisions

- Decided by: Carsten
- Why: a memory forgotten at the time of the move is one the team took back; its ops are replayed into migration records like every other op, so its text and history stay readable, but no decision is written for it: `nxs decision list` does not show it, and a reader finds it in the move's stored report (`nxs migrate --check`, `forgotten_memory`) and its migration records (RFC 0008, "Memories").
- Instead of: writing them as decisions withdrawn by one superseding decision, or writing them in force.
- Topics: migration, decisions, memory
