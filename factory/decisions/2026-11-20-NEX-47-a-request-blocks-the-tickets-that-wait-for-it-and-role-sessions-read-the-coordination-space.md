# 2026-11-20 · NEX-47 · A request blocks the tickets that wait for it, and role sessions read the coordination space

- Decided by: Carsten
- Why: a ticket that needs work from another repository is blocked by that request, across spaces, and everyone must see the block: pm and the people's devices edit in the coordination space, role sessions only read it, and only a pm (the waiting repository's or the receiving one's) resolves the block. Written into RFC 0009, "A request blocks the tickets that wait for it" and "Members and roles"; RFC 0001's roles table and derived state and RFC 0002's readiness follow.
- Instead of: readiness within the repository space only, with the request's status shown beside the ticket for pm (RFC 0009's draft option a), or a block only for readers that can resolve the request (option b).
- Topics: cross-repo, coordination, members, tickets, readiness
- Supersedes: 2026-11-17-NEX-5-role-sessions-are-not-members-of-the-coordination-space.md
