# 2026-11-27 · NEX-127 · nxs decision add refuses without --decided-by when the writer's role record does not say who decided

- Decided by: Carsten, 2026-11-27 (NEX-238), on the proposal of coder on NEX-127
- Why: RFC 0003, "Writing a decision", defaults `decidedBy` to the writer's role from its role record, `pm` or `coder on NEX-42`. It does not say what happens when there is nothing to read. The device that created the space has no record (NEX-99), and a record can name a ticket this copy cannot read. A guess, such as `owner` or the role without its ticket, would put a wrong name into a record that can never be edited. So the write is refused with `validation`, and the message asks for who decided. With a record, the default is its role, followed by ` on <ticket key>` when the record names a ticket.
- Instead of: defaulting to `owner` for a member without a record, or to the role alone when the ticket cannot be read. Both write a guess into an append-only record.
- Topics: decisions, members, nxs, v2
