# 2026-11-27 · NEX-108 · Until the owner decides, a tag only removed members held is not handed out again

- Decided by: coder on NEX-108; provisional until the owner decides how a freed tag passes on
- Why: RFC 0002 lets a freed tag pass on, but "only to a member whose first sync has completed". A member being added cannot have synced the space, and NoCentralDB gives the adder nothing about another member's syncs, so `nxs member add` cannot apply the rule as written (asked of pm on NEX-108). The safe subset: a tag on a removed member's record and on no current member's is neither picked nor taken by `--tag` or `nxs member retag` (`validation`), and when no other tag is left, `no_free_tag`. A member added again gets its own earlier tag back, since the keys with it are its own. A tag `retag` replaced is on no record any more, so it counts as free again.
- Instead of: handing a freed tag out freely, which could repeat a key if the new member mints before its first sync; or refusing every add once all never-used tags are gone, which the subset already does.
- Topics: members, tickets, v2
