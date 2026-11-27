# 2026-11-27 · NEX-240 · nxs init gives the device that creates the space a record and a tag

- Decided by: Carsten
- Why: every member needs a tag before it can mint a ticket key (RFC 0002, "Ticket ids"), and the device that runs `nxs init` is never added through `nxs member add`, so it had neither a `spaceMember` record nor a tag. `nxs init` now writes that device's record, with role `owner` and a free tag, so the first device can create tickets at once. RFC 0001's role record and RFC 0002's "Ticket ids" say so.
- Instead of: (b) the owner running `nxs member retag <device> <tag>` once after `init`, with minting refusing until then; (c) minting giving a member without a tag one on its first ticket.
- Topics: members, tags, init, v2
