# 2026-11-23 · NEX-95 · spaceMember records go into repository and coordination spaces, not into ad-hoc ones

- Decided by: coder on NEX-95
- Why: RFC 0001, "Members", puts a `spaceMember` collection in "each space nexus-flow uses", while RFC 0009, "Members and roles", says the storage crate accepts in an ad-hoc space "only copies of requests", and pm adds a session there with NoCentralDB's `members.add` alone. `space_kind::KINDS` takes the narrower reading, which refuses rather than writes; pm is asked whether the ad-hoc space should hold the session's `spaceMember` record, and widening the table later is additive.
- Instead of: allowing `spaceMember` in every kind of space, which would let the store write a record into an ad-hoc space that RFC 0009 says holds only copies of requests.
- Topics: store, model, members, v2
