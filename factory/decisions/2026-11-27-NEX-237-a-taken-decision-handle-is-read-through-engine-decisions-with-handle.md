# 2026-11-27 · NEX-237 · A taken decision handle is read through Engine::decisions_with_handle

- Decided by: Carsten
- Why: RFC 0003 gives the writer of a decision a warning when its handle is taken in its local copy, but named no field or read that carries it, and `nxs` reaches the store only through the seam. So the seam gains `Engine::decisions_with_handle(&str) -> Vec<ObjectId>`; `nxs decision add` calls it after the write and prints the warning on stderr, and a library caller reads the same. `decision_add` keeps its signature as written. RFC 0003's seam table and "The handle" say so.
- Instead of: (b) a `handle_taken` field in `decision_add`'s result and in the JSON `nxs decision add` prints; (c) dropping the warning.
- Topics: decisions, seam, v2
