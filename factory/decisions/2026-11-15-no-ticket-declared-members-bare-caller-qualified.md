# 2026-11-15 · no ticket · Channel members are recorded BARE while a caller arrives QUALIFIED — compare through facade::same_identity, never == or a SQL handle join.

- Decided by: recorded in the project memory (`nxm`, key `declared-members-bare-caller-qualified`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, identity

## The record

A declared channel's `members:` are recorded BARE (`dev`) while every caller's own identity is
QUALIFIED (`<origin>/<actor>`, e.g. `yyb8/dev`). `ensure_declared_channel` materialises the bare
members plus the qualified AUTHOR of the send, and nothing else.

So any comparison of a caller handle against a declaration MUST go through
`facade::same_identity` (origin-blind: equal, or the qualified identity's bare tail) — never `==`
and never a SQL `WHERE handle = ?` join. A string-equality comparison silently answers "not a
member" to every member that has not itself SENT to the channel, which reads as an empty result
rather than as an error.

Found twice, and cheaply the second time only because the first was written down: 6j6v.v39s (the
thread gate, where the doc comment described a bare-only comparison the code never did) and
6j6v.cs03 (`nxc search` answered "no matches" to every declared member that had only ever replied —
including for its own words).

WHERE THE ONE RULE LIVES NOW, re-measured 2026-09-18 because this entry used to name the wrong
file: the rule is `facade::declared_seat` (`crates/chat/src/facade.rs:1147`) and its two readers are
`require_thread_readable` (through `is_channel_member`) and `facade::search` (through
`searchable_channels`). `ChatStore::search_messages` is NOT one of them any more and says so in its
own source — "WHO may search WHAT is not decided here any more" (`store.rs:2400`): it takes a
channel list that `facade::search` has already resolved. Anyone adding a third reader resolves the
scope in the facade and hands the store a list, rather than teaching the store the rule a second
time.
