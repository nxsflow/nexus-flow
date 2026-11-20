# 2026-11-20 · NEX-6 · Every member gets a ticket tag, a freed tag is reused with its numbering continued, and a ticket records its creator

- Decided by: Carsten
- Why: a role session that may write notes may also create tickets, and a prompt that forbids it is no guarantee, so every member gets a tag and the ticket's `createdBy`, set at creation and never changed, names its creator; the signer of the create counts once NoCentralDB can read it (R6). A freed tag is reused and its numbering continues, so the 936 two-character tags bound the members at one time, not over a space's life, and no key repeats (RFC 0002, "Ticket ids", "Who created a ticket"). Answer to NEX-73.
- Instead of: tags only for members that create tickets, with a new pm taking over its predecessor's tag; or tags that are never reused, which one member per session would use up within months.
- Topics: tickets, ids, members, nocentraldb
