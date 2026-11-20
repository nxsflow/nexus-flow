# 2026-11-20 · NEX-66 · A status note's health is computed from the goals in focus, and the writer may override it with a reason

- Decided by: Carsten
- Why: the owner kept the health on every note, "but the way there must be supported deterministically, oriented on the goals" (answer (a) in NEX-66). `nxs` computes it by a fixed rule from the goals in focus and the repository's tickets that serve them, keeps the counts with the note, and records the writer's override and its reason beside the computed value (RFC 0007, "Health").
- Instead of: no health on a note; or a health the writer sets with no rule beside it, as the Linear note has it today.
- Topics: status, goals
