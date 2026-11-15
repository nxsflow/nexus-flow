# 2026-11-15 · NEX-32 · The factory's rules and designs are under CODEOWNERS, and the decision log is not

- Decided by: Carsten
- Why: `factory/roles/` defines when pm may merge without the owner, and `factory/FACTORY.md` and `docs/rfc/` hold the process and the designs that exception rests on; under CODEOWNERS, branch protection enforces the owner's review of every change to them, so no agent can widen its own merge rights. Implemented in NEX-35.
- Instead of: adding `factory/decisions/` as well, which would make every decision file need the owner's review and stop pm from merging a decided RFC's implementation that records an engine-internal decision (decisions move to NoCentralDB in v2, owner on NEX-30); or leaving the merge exception resting on pm's prompt alone.
- Topics: factory, merge-rule, rfc, decisions
