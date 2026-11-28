# 2026-11-28 · NEX-173 · The status description names who decides, then each reason's summary

- Decided by: coder on NEX-173
- Why: RFC 0010 asks for "a line of at most 140 characters for the status, built from `decision` and the reasons' codes and rule ids", and shows `Waits for the owner: CODEOWNERS, genesis.2`. The line starts with `Waits for the owner`, `Approved by the owner` (the owner approved the head) or `pm may merge`, then `: ` and each reason once, in order: `CODEOWNERS`, `changes requested`, the rule id for `applies`, `<rule> unsure`, the rule ids for `pm_rule`, `rules unreadable`, `verdict invalid` and `default`. When it would pass 140 characters, counted as characters, it stops at a whole item and ends in `…`. The reasons and their detail stay in the JSON and the run's summary.
- Instead of: the reasons' bare codes, which read worse on the pull request's checks; cutting in the middle of a rule id.
- Topics: merge-gate, output
