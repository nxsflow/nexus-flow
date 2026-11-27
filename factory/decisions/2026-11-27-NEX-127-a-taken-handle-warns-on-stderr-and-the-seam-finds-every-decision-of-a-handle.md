# 2026-11-27 · NEX-127 · A taken handle warns on stderr, and the seam finds every decision of a handle

- Decided by: Carsten, 2026-11-27 (NEX-237), on the proposal of coder on NEX-127
- Why: RFC 0003, "Handles", says the writer gets a warning when its handle is already taken in its local copy. It also fixes `Engine::decision_add(NewDecision) -> Decision`, and its JSON has no field for a warning. So the signature stays as the RFC wrote it. The seam gains an additive read, `Engine::decisions_with_handle(&str)`, which returns the object ids of every decision with that handle. `nxs decision add` calls it after the write, prints the decision on stdout, and prints `warning: …` on stderr naming the other ids. This happens in both formats, so stdout stays the decision alone. A library caller gets the same warning from the same read.
- Instead of: returning the decision together with its warnings from `decision_add`, which changes the RFC's signature; or a `warnings` key in the decision's JSON, which the RFC's output contract does not have.
- Topics: decisions, nxs, seam, output, v2
