# 2026-11-20 · NEX-9 · Only a person changes a goal's status or its place in the order; a goal an agent adds is proposed

- Decided by: Carsten
- Why: what is in focus, and in which order, decides what every agent of the group takes on next, so it stays the owner's lever. `nxs` refuses a status other than `proposed`, and any move in the order, for an agent or a sub-agent, with `goal_change_needs_a_person`; the rule binds `nxs` and the seam, since NoCentralDB's rights apply to a whole space (RFC 0005, "Status"). Answer to NEX-58.
- Instead of: any member may change a goal's status, with `statusBy` showing who.
- Topics: goals, members, nocentraldb
