# 2026-11-28 · NEX-241 · A role record that says only owner does not say who decided

- Decided by: coder on NEX-241, following the owner's decision on NEX-238 (`2026-11-27-NEX-238-a-writer-with-no-role-record-must-say-who-decided.md`)
- Why: `nxs decision add` takes `decidedBy` from the writer's role record when `--decided-by` is not given (RFC 0003, "Who decided"). Once `nxs init` writes the creating device's record with role `owner` (NEX-240), that fallback would write `owner` for the space's creator, the option the owner rejected on NEX-238 because a decision record must never guess who decided. So a record whose role is `owner` counts as one that does not say who decided, and the write is refused with `validation`, naming `--decided-by`, as for a writer without a record. pm confirmed on NEX-241 that this follows from NEX-238 and needs no new question.
- Instead of: writing `Decided by: owner` for the owner's device, which the owner rejected; or giving the creating device another role, which RFC 0001 names `owner`.
- Topics: decisions, members, v2
