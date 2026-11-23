# 2026-11-23 · NEX-102 · A sync is done when the relay is connected and nothing is unconfirmed or behind

- Decided by: coder on NEX-102
- Why: RFC 0001's `nxs sync` waits "until the initial sync is complete and nothing is behind or unconfirmed". In the SDK's terms that is all of these at once:
  - the connection is `Connected`;
  - `unconfirmed` in `Db::sync().current()` is 0;
  - every open space is held, its `SpaceStatus::initial_sync_complete` is true, and its `behind` is 0.

  A space the database did not hold when it opened is looked up again on every poll, since sync may bring it. The wait polls these reports every 100 ms on the calling thread: the SDK syncs on a thread of its own, and nxs brings no async runtime.
- Instead of: following the SDK's status streams, which would need a runtime or a waker of nxs's own for the timeout; or not requiring `Connected`, which would let an island database count as synced at once.
- Topics: sync, store
