# 2026-11-27 · NEX-108 · The tags in use are read through an index byTag on spaceMember

- Decided by: coder on NEX-108
- Why: `nxs member add` must know every tag in the space before it picks one (RFC 0002, "Ticket ids"), a removed member's included, and NoCentralDB reads objects only through a declared index. `byMember` fixes a member, so it cannot list every record. `byTag` on `tag` and `member`, `equal: 0`, `scope: "space"`, reads every record of one space in tag order. Adding an index is additive (model version 8, the same model id). No command or seam symbol names it.
- Instead of: reading only the current members' records through `byMember`, which cannot see a tag a removed member held; or one lookup per tag, 936 reads for each add.
- Topics: model, members, store, v2
