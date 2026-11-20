# 2026-11-20 · NEX-72 · The merge gate's decision is built into nxs, and what each format decides is fixed for good

- Decided by: Carsten
- Why: the one deterministic step is tested in nexus-flow's suite, runs the same on every platform, and an upgrade cannot change which pull requests reach the owner; the team owns the rules, schema, prompt and workflow, and may replace the decide step with its own script (RFC 0010, "nxs merge-gate decide"). Answer (a) in NEX-76.
- Instead of: a script file under `factory/merge-gate/` that nexus-flow ships and the team edits.
- Topics: merge-gate, factory, cli
