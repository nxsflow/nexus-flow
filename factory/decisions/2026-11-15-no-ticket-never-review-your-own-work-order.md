# 2026-11-15 · no ticket · The implementing agent NEVER starts the review — not for safety, not when the gates are green; the owner or PM starts it in a fresh session.

- Decided by: recorded in the project memory (`nxm`, key `never-review-your-own-work-order`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: review, process

## The record

THE REVIEW IS NEVER STARTED BY THE IMPLEMENTING AGENT. Whoever received a coding assignment (for
example through a `pm-next-task` brief) must NOT invoke `review-pr` / `/nxf-workflow:review-pr`
themselves — not "just to be safe", not even when the quality gates are green and the pull request
is open. Owner instruction of 2026-08-07, explicit and without exception.

The review is started by the owner or the PM in their OWN, FRESH session: only a session that did
not write the code can read it without prejudice — one that built it re-checks its own assumptions
and will fail to notice in exactly the places where it already failed to notice while building.

What the implementing agent does instead: open the pull request, report the state honestly
(including the open points), and offer the handover.

CONSEQUENCE FOR `.nxf/status.jsonl`: `review_requested` and `review_completed` are written by the
REVIEW session, not by the coding session. A `review_requested` entry set by the coding session is
wrong and belongs removed (it happened once and was taken out again).

HOW TO GET HOLD OF A FINISHED REPORT: do not start reviewers yourself — look in `.nxf/review/`, or
resolve the last `review_completed` entry via `pm-work-status`, which carries the `report_path`.
That is exactly what went wrong on 2026-08-07: the owner's report was already sitting in
`.nxf/review/` while the coding session was spinning up reviewers of its own in parallel.

See also [review-session-in-a-shared-checkout-mistakes-the-work-for-its-own](2026-11-15-no-ticket-review-session-in-a-shared-checkout-mistakes-the-work-for-its-own.md) for the failure on
the other side of this rule.
