# 2026-11-20 · NEX-78 · A moved v1 item is done, and on main, by the commits that name its v1 id

- Decided by: Carsten
- Why: v1 records no work for an item, and v1-era commits name v1 item ids, not the new keys; a closed item is `done` when a commit on the base branch names its v1 id, the move records that commit as a work link, and the ticket keeps the id in a new field `v1Id` that RFC 0002's on-main rule also accepts (RFC 0008, "How the ops are replayed"). A closed item without such a commit is `canceled`.
- Instead of: RFC 0002's table alone, where every closed item reads `canceled`; or an artifact link on every closed item, so it is on main once done.
- Topics: migration, tickets, git
