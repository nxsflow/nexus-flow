# 2026-11-20 · NEX-53 · nexus-flow offers no redaction of a decision before NoCentralDB can erase unmarked values (R13)

- Decided by: Carsten
- Why: GENESIS.md forbids personal data in decisions, and a redaction that overwrites a field leaves the value in every member's log, so it would look like erasure without being one and would be the one exception to "a decision is never edited away". nexus-flow states plainly that it cannot erase such a value until R13. RFC 0003, "Personal data and erasure", option (a).
- Instead of: an admin-only `nxs decision redact` that overwrites a field with "redacted" and records a decision saying so.
- Topics: decisions, rfc, privacy, append-only
