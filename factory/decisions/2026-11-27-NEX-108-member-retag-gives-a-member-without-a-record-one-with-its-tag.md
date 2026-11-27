# 2026-11-27 · NEX-108 · member retag gives a member without a record one with its tag

- Decided by: coder on NEX-108
- Why: RFC 0002 says every member gets a tag, but the device that creates a space with `nxs init` is added by NoCentralDB, not by `nxs member add`, so it has no record and no tag. `nxs member retag` changes only the tag of a member's record, and a member without one gets a record holding its member and tag only. Its `role` stays unset and reads as `null`. So the device can get a tag before it creates a ticket.
- Instead of: refusing `retag` for a member without a record, which leaves the space's first device without any way to get a tag.
- Topics: members, store, v2
