# 2026-11-27 · NEX-172 · A verdict names every problem, and cites a decision by its plain path

- Decided by: coder on NEX-172
- Why: RFC 0010's `verdict_invalid` carries `detail`, "one line per problem", without the lines. The validator names every problem in one pass, in order: `format`, `base`, `head`, then each entry in the verdict's order, then each rule with no entry in the rules' order; the wording is for people and not a stable code. A `reason` of only whitespace is empty, as a rule's `text` is in `gate.toml` (NEX-170). A `decision` key counts as present whatever its value. A `decision.path` must be the plain path from the repository's root, with no leading `/` and no empty, `.` or `..` part, because git reads `/a`, `./a`, `a//b` and `x/../a` as the same file, and the owner should see one spelling per file; a `section` that is present must be a string. The valid verdict lists its entries in the rules' order, GENESIS's first.
- Instead of: stopping at the first problem, which would have pm fix a verdict one problem at a time; accepting any spelling git resolves.
- Topics: merge-gate, verdict
