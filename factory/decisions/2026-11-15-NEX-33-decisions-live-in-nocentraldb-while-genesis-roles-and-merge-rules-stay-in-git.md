# 2026-11-15 · NEX-33 · Decisions live in NoCentralDB, while GENESIS, roles and merge rules stay in git

- Decided by: Carsten
- Why: NoCentralDB is the centre of nexus-flow, and its team develops NoCentralDB itself, so the dependency is under its control; tickets, decisions, goals and status notes live there, and only what the owner reviews in a PR (GENESIS and the factory's files, as NEX-14) stays in git. Answer to the review in NEX-30.
- Instead of: keeping decisions as files in git and putting NoCentralDB behind a storage seam to adopt later, as the NEX-30 review proposed.
- Topics: nocentraldb, decisions, storage, genesis
