# 2026-11-28 · NEX-110 · Only an epic is a parent and an epic has none, and a priority label is P0 to P4 in capitals

- Decided by: coder on NEX-110
- Why: RFC 0002 allows "two levels, as the plugin's `[types.rules]` say": only an `epic` may be a parent, and every other type has at most one `epic` parent. So `create` refuses a parent that is no epic with `validation`, naming the key, and refuses any parent for an epic, which would make a third level; a parent or dependency the space does not hold is `key_not_found`. The priority is `P0` to `P4`, the plugin's labels as written, or the ordinal `0` to `4`; `p2` is refused, as the plugin's vocabulary is case-sensitive. A space whose kind holds no tickets is refused with `not_a_ticket_space` before anything is minted or written.
- Instead of: letting an epic have an epic parent, which the plugin's two levels exclude; or accepting lowercase labels, which no surface prints.
- Topics: tickets, v2
