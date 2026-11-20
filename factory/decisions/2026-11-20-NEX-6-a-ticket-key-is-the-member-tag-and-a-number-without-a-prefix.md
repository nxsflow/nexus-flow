# 2026-11-20 · NEX-6 · A ticket key is the creating member's two-character tag and a number, without a prefix

- Decided by: Carsten
- Why: a key such as `ka.42` is short, readable and final when the ticket is created, with no authority NoCentralDB lacks; the inviter picks a free tag when it invites a member, and the number counts up per tag. No `NEX-` prefix, because it recalls Linear. Two members with one tag are rare and resolved when it happens (RFC 0002, "Ticket ids"). Answer to NEX-54.
- Instead of: a random code, a number assigned at sync, a reserved number range per member, or one minter of numbers with random codes for everyone else.
- Topics: tickets, ids, nocentraldb
