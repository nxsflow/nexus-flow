# 2026-11-15 · no ticket · A review session in a shared checkout can mistake foreign commits for its own work and refuse — say so up front, and forbid cargo there.

- Decided by: recorded in the project memory (`nxm`, key `review-session-in-a-shared-checkout-mistakes-the-work-for-its-own`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: review, process

## The record

A review session started in the SAME checkout as the coding session can mistake itself for the
author of the code under review and refuse the review, citing
the v1 memory `never-review-your-own-work-order` (dropped with the rules for using nxs).

MEASURED 2026-09-05 on the pull request for the escalation-parking feature: a freshly started
review session declined the assignment with "this is my own work from today". Its evidence: the
reflog showed a commit plus two amends from today on exactly the commit under review, and the board
item already carried detailed design notes on exactly the points to be reviewed.

BOTH PIECES OF EVIDENCE ARE PROPERTIES OF THE SHARED WORKING DIRECTORY, NOT OF THE SESSION'S
MEMORY. A fresh session in an existing repo always finds somebody's recent commits there. The
fallacy is natural and it is structural: the session has no access to the other's transcript but
sees its traces in the file system.

WHAT RESOLVES IT, in this order:

1. ITS OWN TRANSCRIPT. If you do not remember writing the file, you did not write it. This is the
   only check the session alone can make, and the decisive one.
2. `ps`. Two separate PIDs with separate start times; commits created AFTER the review session
   started cannot be its own if it has done nothing since. The review session ran this itself and
   thereby confirmed the correction independently — exactly right: do not cave on assertion, look.
3. WHO COMMISSIONED IT. The rule forbids the IMPLEMENTING agent from starting the review. An
   assignment issued by the owner and merely relayed by the coding session is the intended form,
   not a circumvention of it.

WHOEVER COMMISSIONS A REVIEW SESSION says this UP FRONT — otherwise it costs two rounds. And they
say the second half straight away, which the session cannot see by itself: BOTH SHARE ONE
`target/`. Two concurrent cargo runs in it destroy each other (see
[no-concurrent-lean-build-during-tests](2026-11-15-no-ticket-no-concurrent-lean-build-during-tests.md)). The review session should read the diff and take the
gate results from `gh pr checks <pr>`, not start `cargo test --all` alongside.

THE CLEAN SOLUTION if this recurs: start the review session in its own git worktree instead of the
shared checkout. Both problems disappear at once — the confusion and the `target/` collision.
