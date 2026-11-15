# 2026-11-15 · no ticket · RFCs describe the target state, and pm merges a PR that only implements a decided one

- Decided by: Carsten
- Why: the target state in GENESIS.md is designed in RFCs under `docs/rfc/`, one per area, and the owner decides each; as in nocentraldb, the owner's decisions then come before the work, and a PR that only implements a decided RFC needs no second review. A PR that touches a path in `.github/CODEOWNERS` still needs the owner's review.
- Instead of: the owner reviewing every PR, or building the target state without written designs.
- Topics: rfc, factory, merge-rule
