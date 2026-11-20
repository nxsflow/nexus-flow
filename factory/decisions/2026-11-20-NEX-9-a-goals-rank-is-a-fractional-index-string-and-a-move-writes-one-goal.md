# 2026-11-20 · NEX-9 · A goal's rank is a fractional-index string, so a move writes one goal and concurrent reorders lose nothing

- Decided by: coder on NEX-9
- Why: the owner's order among the goals in focus (NEX-59) must survive two people reordering at once. A rank strictly between its neighbours' ranks, compared by its bytes, changes one goal per move: moves of different goals both stand, a goal moved twice keeps the last write with the other as a conflict, and equal ranks are broken by key, then object id (RFC 0005, "The order of the goals in focus").
- Instead of: positions 1 to n on each goal, which rewrite every goal between the old and the new place; or one list of the goals in focus on a single object, where concurrent moves of different goals would undo each other.
- Topics: goals, ranking, nocentraldb
