# 2026-11-27 · NEX-238 · A writer with no role record must say who decided

- Decided by: Carsten
- Why: RFC 0003 lets `decidedBy` default to the role of the writing member from its role record, but did not say what happens when there is no record (such as the owner's own device) or the record's ticket cannot be read. A decision record must never guess who decided, so the write is refused with `validation`, naming `--decided-by`. The cost is that the owner types `--decided-by` on their own device. RFC 0003, "Who decided", says so.
- Instead of: (b) defaulting to the account's display name, which NoCentralDB may not expose; (c) defaulting to the literal `owner` for the space's creator and refusing otherwise.
- Topics: decisions, v2
