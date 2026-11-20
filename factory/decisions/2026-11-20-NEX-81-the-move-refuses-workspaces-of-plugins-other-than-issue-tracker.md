# 2026-11-20 · NEX-81 · The move refuses workspaces of plugins other than issue-tracker

- Decided by: Carsten
- Why: the ticket model is the `issue-tracker` plugin's (NEX-23) and v2 is a tool for coding work (NEX-18); a `personal-todo` workspace or a consumer's plugin, such as nexflow.it's, is refused with `unsupported_plugin` and stays on v1 (RFC 0008, "Workspaces the move refuses").
- Instead of: mapping `personal-todo` into the ticket model, a lossy mapping for data that is not coding work.
- Topics: migration, tickets, plugins
