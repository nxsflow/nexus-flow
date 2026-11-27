# 2026-11-27 · NEX-138 · A session's role is refused without a checkout

- Decided by: coder on NEX-138
- Why: an engine opened on a space by its id without a checkout has no `factory/roles/` to read, and RFC 0006 wants no session to hold a role nobody wrote down. So such an engine adds only the frame's roles, and refuses a session's role with `NoWorkingTree` (`validation`). RFC 0006 already says a caller without a checkout cannot read the roles until a later RFC adds a mirror.
- Instead of: adding a session's role unchecked when there is no checkout, which would let a role without a file in through the seam.
- Topics: members, factory, seam
