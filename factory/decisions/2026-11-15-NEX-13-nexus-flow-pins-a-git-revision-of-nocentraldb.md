# 2026-11-15 · NEX-13 · nexus-flow pins a git revision of NoCentralDB, and gaps go to NoCentralDB as requests

- Decided by: Carsten
- Why: NoCentralDB is unreleased (0.1.0) and priority 1 builds on it; a git pin, moved deliberately one PR at a time, keeps the work moving without an unpinned dependency. What NoCentralDB lacks goes to it as a request; a stop-gap in nexus-flow sits only behind its own seam, named as temporary in its RFC, with a ticket to remove it.
- Instead of: waiting for a crates.io release, or vendoring a copy that drifts.
- Topics: nocentraldb, dependencies, v2
