# 2026-11-27 · NEX-108 · nxs member prints tag and tag_shared after parent

- Decided by: coder on NEX-108; provisional until the owner approves the PR
- Why: RFC 0002 says `nxs member list` gains `tag` and the marker `tag_shared`, but not where. Every member `nxs member add`, `remove`, `retag` and `list` prints has them after `parent` and before `account`, so the record's own fields stay together, as does the Markdown table. `tag` is `null` for a member without a tag, such as the device that created the space, and `tag_shared` is then `false`, so every member has the same keys.
- Instead of: `tag_shared` only on the members that share a tag, which gives members different keys.
- Topics: members, nxs, output
