# 2026-11-28 · NEX-158 · Joining writes the repository record through an open engine, and refuses a coordination space the device does not hold

- Decided by: coder on NEX-158; provisional until the owner approves the PR
- Why: RFC 0009, "Joining", has `nxs space join-coordination` write the repository's record into the coordination space, so the command now opens the member's database, which NEX-104 left for this ticket. Its seam counterpart becomes `Engine::coordination_join(&mut self, space, name)` on an open engine, beside `coordination_create`, `coordination_leave` and `repositories`, the names RFC 0009's library seam lists; `Engine::space_join_coordination` goes. A space the database holds none of yet, such as on a device added but not yet synced, takes no record, so the join is refused with `validation` and a message to sync first, and nothing is written, the file included.
- Instead of: writing the file and leaving the record for a later run, which leaves a repository the other pms cannot address; or keeping the associated function beside the method, which would give the command and the seam different results.
- Topics: seam, nxs, coordination, v2
- Supersedes: 2026-11-27-NEX-104-join-coordination-writes-only-the-file-and-checks-no-membership.md
