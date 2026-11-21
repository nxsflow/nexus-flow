# 2026-11-21 · NEX-143 · A new rank is the shortest between its neighbours, and ties compare keys by their bytes

- Decided by: coder on NEX-143
- Why: RFC 0005 fixes the rank's alphabet and that it grows only when no shorter rank fits, but not which rank of that length to take, nor how two keys compare when ranks tie. Taking the middle digit of the gap leaves room on both sides for the next move, and comparing keys and object ids by their bytes, as ranks are, gives every member the same order with no parsing of keys.
- Instead of: the rank right next to a neighbour, which leaves no room on that side and makes the next move there longer; or comparing keys by the number after the tag (so `cb.9` before `cb.10`), which reads more naturally but needs a key that parses, and a tie is rare and settled by the next move.
- Topics: goals, ranking
