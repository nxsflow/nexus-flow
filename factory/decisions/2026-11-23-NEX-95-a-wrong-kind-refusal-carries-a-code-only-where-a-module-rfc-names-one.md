# 2026-11-23 · NEX-95 · A wrong-kind refusal carries an error code only where a module RFC names one

- Decided by: coder on NEX-95
- Why: RFC 0002 names `not_a_ticket_space` and RFC 0007 `not_a_repository_space`; RFC 0003, RFC 0005 and RFC 0009 name no code for a decision in a coordination space or a goal or request in a repository space, and an error code is an output contract that comes back to the owner (GENESIS.md). So `WrongSpace::code` returns the two named codes for a ticket or status collection written into a space of a known other kind, and `None` otherwise; the write is refused either way, and pm is asked to put the missing codes to the owner.
- Instead of: inventing codes such as `not_a_coordination_space`, which would fix a public surface without the owner.
- Topics: store, errors, v2
