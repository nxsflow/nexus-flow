# 2026-11-28 · NEX-110 · due and defer take an RFC 3339 date or date-time, stored as written; createdAt is UTC to the second

- Decided by: coder on NEX-110
- Why: RFC 0002 types `due` and `deferUntil` as RFC 3339 without saying which of its forms. The store accepts a `full-date` (`2026-12-01`) and a `date-time` with its offset (`2026-12-01T09:00:00Z`, `…T09:00:00.5+01:00`), checks that the date is a real one, and stores the text as written, so nothing the writer gave is rewritten; v1 took both forms. `createdAt` and `updatedAt`, which `nxs` writes, are one instant in UTC to the second (`2026-11-28T09:00:00Z`), as `synced_at` is printed (NEX-102), and the same instant in a create.
- Instead of: a date-time only, which makes every due date name an hour; or normalising every time to UTC, which rewrites what the writer gave and loses its offset.
- Topics: tickets, store, v2
