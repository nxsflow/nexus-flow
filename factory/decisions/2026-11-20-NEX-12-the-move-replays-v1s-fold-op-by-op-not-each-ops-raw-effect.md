# 2026-11-20 · NEX-12 · The move replays v1's fold op by op, not each op's raw effect

- Decided by: coder on NEX-12
- Why: a v1 op's effect depends on what its writer had seen (last-writer-wins registers, observed-remove edges), so applying raw effects in sequence diverges from v1 wherever v1 had concurrency; folding with v1's own reducers and writing each op's difference to the views makes the space equal v1's views after the last op, keeps one change per op as NEX-19 asks, and lets a stopped run carry on from any op (RFC 0008, "How the ops are replayed").
- Instead of: writing each op's raw effect as a NoCentralDB change, or reading v1's stored views, which can be behind or damaged.
- Topics: migration, nocentraldb, history
