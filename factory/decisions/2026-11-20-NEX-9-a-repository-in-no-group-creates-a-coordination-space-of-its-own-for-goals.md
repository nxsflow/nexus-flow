# 2026-11-20 · NEX-9 · A repository in no group creates a coordination space of its own to use goals

- Decided by: Carsten
- Why: goals then have one place, and a repository that joins a group later moves nothing; the cost is a second space, with its pm a member of both. `nxs goal add` without a coordination space refuses with `no_coordination_space` (RFC 0005, "Where goals live"). Answer to NEX-62.
- Instead of: goals in the repository space while it belongs to no group, with a migration when it joins one.
- Topics: goals, spaces, nocentraldb
