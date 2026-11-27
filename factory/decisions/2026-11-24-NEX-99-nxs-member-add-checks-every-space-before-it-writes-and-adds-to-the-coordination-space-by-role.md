# 2026-11-24 · NEX-99 · nxs member add checks every space before it writes, and adds to the coordination space by role

- Decided by: coder on NEX-99
- Why: RFC 0001's "Roles in a space" gives each role a NoCentralDB role per kind of space, and "Starting pm" adds pm to both spaces with one `nxs member add`. So the store adds a member to each open space its role belongs in (a role session and the migration member to the repository space only), and first checks in every one that the space is held and that this member is an admin there, so a refusal changes nothing. NoCentralDB adds the member and the record in two changes; the member comes first, so a refused add never leaves a record behind.
- Instead of: adding to the repository space only, which leaves pm out of the coordination space RFC 0001 adds it to; or checking each space as it is reached, which can add a member to one space and then refuse in the next.
- Topics: members, store, v2
