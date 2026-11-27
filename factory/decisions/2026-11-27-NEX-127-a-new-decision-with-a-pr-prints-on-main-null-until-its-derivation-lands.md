# 2026-11-27 · NEX-127 · A new decision with a pr prints on_main null until its derivation lands

- Decided by: coder on NEX-127
- Why: `nxs decision add` prints the decision in full, which includes the derived `in_force`, `superseded_by` and `on_main`. A decision that was just written is in force, and nothing supersedes it yet. Without `pr` it is on main once it is made (RFC 0003, "Derived state"). With `pr`, it is on main only once its pull request's commit or its triggering ticket is on main, and NEX-128 derives that from the local git. Until then `on_main` is `null`, the RFC's "not known". `true` or `false` would be a guess.
- Instead of: reading the local git for the PR's commit in this ticket, which is NEX-128's derivation; or printing `false`, which is wrong for a decision recorded after its PR merged.
- Topics: decisions, derived-state, nxs, v2
