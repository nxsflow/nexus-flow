# 2026-11-21 · NEX-215 · A dependency exception's board item may be a Linear ticket id, but an advisory id is not one

- Decided by: coder on NEX-215
- Why: `cargo xtask deps exceptions` now accepts `<UPPER>-<digits>` (such as `NEX-214`) as the board item, because v2's tickets live in Linear until RFC 0002 moves them into v2. A dash on either side of the match disqualifies it, so a reason that only cites `RUSTSEC-2024-0449` or `CVE-2024-1234` still names no board item and stays red.
- Instead of: the bare `<UPPER>-<digits>` match, which would take `RUSTSEC-2024` out of an advisory id for a ticket and let an exception through without one.
- Topics: dependencies, xtask, tickets
