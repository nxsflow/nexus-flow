# 2026-11-20 · NEX-9 · Goal keys take RFC 0002's form, and the goal commands are nxs goal add, list, show, edit and status

- Decided by: Carsten
- Why: one rule for readable ids across modules, `<tag>.<number>` with the creating member's tag in the coordination space, and one command per thing done (RFC 0005, "Goal ids", "Public surfaces this RFC brings"). Answer to NEX-64.
- Instead of: goal ids as object-id prefixes, or a command per status such as `nxs goal focus` and `nxs goal drop`.
- Topics: goals, ids, cli
