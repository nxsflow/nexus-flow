# 2026-11-24 · NEX-99 · nxs member add and remove print the member as nxs member list prints it

- Decided by: coder on NEX-99; provisional until the owner approves the PR
- Why: RFC 0001 names the JSON of `nxs member list` (`member`, `kind`, `role`, `ticket`, `parent`, `account`) and none for `add` or `remove`. Printing the added or removed member as the same object gives an agent one shape to read, and the new member's id, which the invitation does not show, without a second command. `remove` prints the member as it was listed before it went.
- Instead of: printing only the member id, or nothing, which would make an agent run `nxs member list` to learn what it added.
- Topics: members, nxs, output
