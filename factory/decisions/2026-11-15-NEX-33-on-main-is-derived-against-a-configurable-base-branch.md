# 2026-11-15 · NEX-33 · "On main" is derived against the team's configurable base branch

- Decided by: Carsten
- Why: a team may merge into another branch than `main`, as this repository does with `v2`; a commit may name tickets as a range (`NEX-13..NEX-23`), and the state is as fresh as the last fetch. Answer to the review in NEX-30.
- Instead of: deriving the state from `main` by name, from single ticket ids only.
- Topics: tickets, git, branching
