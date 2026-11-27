# 2026-11-27 · NEX-236 · The guard for a freed member tag sits at minting

- Decided by: Carsten
- Why: RFC 0002 said `nxs member add` hands out a freed tag only to a member whose first sync has completed, but a member being added has not synced and cannot have, and NoCentralDB 0.1.0-beta.1 tells the adder nothing about another member's syncs. The risk of a freed tag is that its new holder repeats a key the old holder created; the rule that a member waits for its first sync before it creates a ticket (NEX-109) already prevents that. So `member add` and `member retag` may hand out a freed tag freely. RFC 0002, "Ticket ids", says so.
- Instead of: (a) handing out a freed tag only when the adder's own copy has completed a sync; (c) never handing out a freed tag through `add`, only through `retag`.
- Topics: tickets, members, v2
