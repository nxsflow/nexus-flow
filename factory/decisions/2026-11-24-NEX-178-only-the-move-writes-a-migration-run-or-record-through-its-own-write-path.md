# 2026-11-24 · NEX-178 · Only the move writes a migration run or record, through its own write path

- Decided by: coder on NEX-178
- Why: RFC 0008 says only `nxs migrate` writes a `migrationRun`, and the records are written by the move's replayed changes, but leaves open how the store holds that. The store cannot yet tell the migration member from another writer (it opens a device or a session, and the migration member's database is a later ticket's), so the rule sits on the write path: `Opened::write` refuses a create, set, delete or restore of `migrationRun` or `migrationRecord` with `WriteError::MoveOnly`, and `Opened::write_as_move` is the only path that writes them, with every other check in place, so a record stays write-once for the move too. Like every refusal, it writes nothing of the change.
- Instead of: leaving it to the callers, so that any command could write or rewrite a run; or checking the writer's `spaceMember` role, which needs the migration member that NEX-181's move creates.
- Topics: store, migration, v2
