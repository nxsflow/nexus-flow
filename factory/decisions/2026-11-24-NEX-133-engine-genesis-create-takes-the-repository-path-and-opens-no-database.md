# 2026-11-24 · NEX-133 · Engine::genesis_create takes the repository path and opens no database

- Decided by: coder on NEX-133
- Why: RFC 0004 names the seam `Engine::genesis_create(update: bool)` and says `create` opens no NoCentralDB database, touching only files and the local git. A method on an open `Engine` would need the member's database open first, so it is an associated function, `Engine::genesis_create(repository: &Path, update: bool)`, which finds the root from `repository` upward as `Engine::sync_status(repository)` does, and returns the report. A `tracked` GENESIS is an `Ok` report whose `is_refused()` is true, so the library returns the same report the command prints; `nxs` turns it into exit code 1.
- Instead of: a method on an open engine, which would open a database `create` must not touch; or a free function `genesis_create` beside `init`, which leaves the name the RFC gives.
- Topics: genesis, seam, v2
