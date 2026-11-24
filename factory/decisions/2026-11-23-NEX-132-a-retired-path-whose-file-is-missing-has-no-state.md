# 2026-11-23 · NEX-132 · A retired path whose file is missing has no state

- Decided by: coder on NEX-132
- Why: RFC 0006's table gives a retired path a state only when its file is present (`retired`), and `install` never writes a path nexus-flow stopped shipping. `Shipped::state` therefore answers `None` for a retired path with no file, as it does for a path the list does not name, so nothing is written there. Whether the report lists such a path is for the ticket that builds the report (NEX-137); the comparator gives it no state of RFC 0006's six.
- Instead of: `written`, which would install a file nexus-flow no longer ships, or a seventh state, which would add to RFC 0006's public output.
- Topics: templates, factory, seam, v2
