# 2026-11-20 · NEX-86 · pm merges an RFC PR that only writes in the owner's answers, until the merge gate runs

- Decided by: Carsten
- Why: Most of the owner's reviews on 2026-11-20 were RFC PRs that only wrote the owner's own ticket answers into the text, and every merge sent the next one back for a new approval. Until RFC 0010's merge gate runs, pm merges such a PR without the owner's review. It checks that every changed line carries an answer from a needs-owner ticket, quotes those tickets in a PR comment and lists the merge in the daily note. `/docs/rfc/` leaves `.github/CODEOWNERS` for this. An implementation PR must not touch `docs/rfc/`, so no code PR can widen the RFCs it is judged by. Recorded in CONFIG.md, "Merge rule", and roles/pm.md. Revert it once the merge gate runs.
- Instead of: the owner reviewing every RFC PR, which stays the rule for new drafts, new questions and every path under `factory/`; or the merge gate itself, which waits for NoCentralDB's 0.1 beta.
- Topics: factory, merge, review
