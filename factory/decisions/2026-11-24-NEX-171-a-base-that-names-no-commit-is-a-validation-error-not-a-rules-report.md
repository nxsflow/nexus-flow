# 2026-11-24 · NEX-171 · A base that names no commit is a validation error, not a rules report

- Decided by: coder on NEX-171
- Why: the rules report's `base` is the full object id `--base` resolved to, and RFC 0010's error codes all name a file at that commit, so a reference that resolves to no commit has no report to print. `nxs merge-gate rules` then prints v1's error object with the kind `validation`, as `nxs merge-gate decide` cannot decide when `--base` is not a commit (RFC 0010); a `git` that cannot be run is `io`. Either way it exits 1, as unreadable rules do, so the gate never reads a missing base as rules.
- Instead of: a report with an empty `base` and a new error code, which would widen the list of codes RFC 0010 decided.
- Topics: merge-gate, output
