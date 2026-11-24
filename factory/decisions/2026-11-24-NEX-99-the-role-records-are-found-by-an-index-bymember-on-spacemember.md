# 2026-11-24 · NEX-99 · The role records are found by an index byMember on spaceMember

- Decided by: coder on NEX-99
- Why: NoCentralDB reads objects only through a declared index or by object id, and `nxs member list` must read each current member's role, so `spaceMember` gets `byMember` on `member`, `equal: 1`, `scope: "space"`. Adding an index is additive (model version 2, same model id). The index is inside the store: no command or seam symbol names it.
- Instead of: keeping each record's object id somewhere else, such as beside the member database, which a member that syncs a space it did not write would not have.
- Topics: model, members, store, v2
