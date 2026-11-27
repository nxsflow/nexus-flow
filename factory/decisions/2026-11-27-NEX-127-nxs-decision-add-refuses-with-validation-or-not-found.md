# 2026-11-27 · NEX-127 · nxs decision add refuses with validation, or not_found for an unknown decision to supersede

- Decided by: coder on NEX-127; provisional until the owner approves the PR
- Why: the error object keeps v1's codes (NEX-97), and RFC 0003 asks only that a refusal names its problem. Every check of "Writing a decision" fails on what the writer gave, so it is `validation`. This includes an ambiguous handle, a decision dated after this one, and a missing `--decided-by` with no role record. The one exception is a decision to supersede that the local copy does not hold: that is `not_found`, as `nxs member remove` reports a member that is not one (NEX-99). A database failure is `io`. Leaving out `--title`, `--why`, `--instead-of` or `--topic` gets the same message as giving it empty, because the seam checks both.
- Instead of: a code per refusal, such as `title_too_long`, which RFC 0003 does not name and which would add to the output contract without the owner.
- Topics: decisions, errors, nxs, v2
