# 2026-11-20 · NEX-52 · A decision's PR is on main by its merge commit, or else by its triggering ticket

- Decided by: Carsten
- Why: a commit on the base branch whose title ends with `(#<n>)` or starts with `Merge pull request #<n>` is precise for GitHub's squash and merge commits, which this factory uses; where none is found, following the triggering ticket still ends in "on main" for a rebase merge or another forge. The cost is that a ticket with several PRs is on main after the first. RFC 0003, "Derived state", option (c).
- Instead of: the commit rule alone, which leaves a decision "not yet on main" for ever after a rebase merge or on another forge; or the ticket rule alone, which is wrong for every ticket with several PRs.
- Topics: decisions, rfc, git, on-main
