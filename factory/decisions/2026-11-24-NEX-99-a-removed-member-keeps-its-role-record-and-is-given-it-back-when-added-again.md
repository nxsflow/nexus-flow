# 2026-11-24 · NEX-99 · A removed member keeps its role record, and is given it back when added again

- Decided by: coder on NEX-99
- Why: RFC 0001 says the member that adds another writes its record, but not what removing does to it. "Who decided" needs the role of a member that has since gone, once NoCentralDB can say who signed a change (R6), so `nxs member remove` leaves the record, and `nxs member list` shows only current members. A member added again gets its record rewritten with the new role and ticket rather than a second one, so a space holds one record per member.
- Instead of: deleting the record on removal, which loses the role of a removed member's past changes; or a new record per add, which leaves several roles for one member.
- Topics: members, store, v2
