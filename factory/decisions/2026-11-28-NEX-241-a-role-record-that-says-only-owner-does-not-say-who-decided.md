# 2026-11-28 · NEX-241 · A role record that says only owner does not say who decided

- Decided by: coder on NEX-241; provisional until pm or the owner confirms it
- Why: `nxs decision add` takes `decidedBy` from the writer's role record when `--decided-by` is not given (RFC 0003, "Who decided"). Once `nxs init` writes the creating device's record with role `owner` (NEX-240), that fallback would write `owner`, the option the owner rejected for the space's creator (NEX-238: "defaulting to the literal `owner` for the space's creator"). So a record whose role is `owner` counts as one that does not say who decided, and the write is refused with `validation`, naming `--decided-by`, as for a writer without a record. NEX-238's behaviour stays as decided; the question went to pm on NEX-241.
- Instead of: writing `Decided by: owner` for the owner's device, which the owner rejected; or giving the creating device another role, which RFC 0001 names `owner`.
- Topics: decisions, members, v2
