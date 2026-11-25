# 2026-11-25 · NEX-232 · Seam functions of commands that open no database take the repository

- Decided by: Carsten
- Why: RFC 0010 wrote `Engine::merge_gate_rules` and `Engine::merge_gate_decide`, and RFC 0004 wrote `Engine::genesis_create`, as methods on an open `Engine`, while the same RFCs say these commands open no NoCentralDB database; the merge gate runs in CI, where there is no key store. So each is an associated function that takes the repository and opens no database: `Engine::merge_gate_rules(repository: &Path, base: Option<&str>)`, where `None` reads `[git] base` as the command does without `--base`; `Engine::merge_gate_decide(repository: &Path, base, head, comments, verdict, reviews)`; `Engine::genesis_create(repository: &Path, update: bool)`. This follows `Engine::sync_status` (NEX-102) and keeps CLI and library giving the same bytes. `Engine::factory_install` (RFC 0006) is not covered: its RFC does not say it opens no database.
- Instead of: (b) free functions in the seam outside `Engine`, after the `init` precedent (NEX-97); (c) the methods on an open `Engine` as written, which CI could not call.
- Topics: seam, merge-gate, genesis, v2
