# 2026-11-24 · NEX-137 · codeowners.uncovered checks only the files the report lists

- Decided by: coder on NEX-137
- Why: RFC 0006 says `uncovered` lists paths "in the same order as `files`", and the ticket says a team's own role, such as `roles/legal.md`, is never listed. So `install` asks CODEOWNERS only about the paths in `files`: the shipped paths that have a state, a present retired one included. A retired path whose file is missing is in neither list while NEX-235 waits for the owner.
- Instead of: every file under `factory/`, which would list the team's own roles and anything else the team keeps there, against the ticket.
- Topics: factory, codeowners, v2
