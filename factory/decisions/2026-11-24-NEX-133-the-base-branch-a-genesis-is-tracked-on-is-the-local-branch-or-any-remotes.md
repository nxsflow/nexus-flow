# 2026-11-24 · NEX-133 · The base branch a GENESIS is tracked on is the local branch or any remote's

- Decided by: coder on NEX-133
- Why: RFC 0004's `tracked` state asks whether git tracks `factory/GENESIS.md` "on the base branch" without saying which ref that is. A fresh clone often has only the remote's tracking branch (`origin/v2`), not a local one, so `create` looks in `refs/heads/<base>` and in every `refs/remotes/*/<base>`, as well as in the index and `HEAD`, by running the local `git` (`ls-files`, `rev-parse`, `ls-tree`), never fetching. A ref that names no commit, such as an unborn `HEAD`, holds nothing. Outside a git repository nothing is tracked, so the template is written; any other git failure, such as git missing or a repository git refuses to read, is an error and nothing is written. Looking in more refs can only refuse more, never overwrite more.
- Instead of: the local branch alone, which misses the base branch in a fresh clone and would let the template replace the team's text in the next commit; or a git library crate, a new dependency for three read-only plumbing calls.
- Topics: genesis, git, seam, v2
