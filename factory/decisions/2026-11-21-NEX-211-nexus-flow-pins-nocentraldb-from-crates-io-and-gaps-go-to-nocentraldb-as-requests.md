# 2026-11-21 · NEX-211 · nexus-flow pins NoCentralDB from crates.io at an exact version, and gaps go to NoCentralDB as requests

- Decided by: Carsten
- Why: NoCentralDB 0.1.0-beta.1 is on crates.io, and the git pin was chosen only because it was unpublished (NEX-211, (a)). The SDK is pinned at an exact `=` version, moved deliberately one PR at a time, a PR that carries nothing else and waits for the owner. The rest of NEX-13 stands unchanged: what NoCentralDB lacks goes to it as a request; a stop-gap in nexus-flow sits only behind its own seam, named as temporary in its RFC, with a ticket to remove it. Written into RFC 0001, "The dependency on NoCentralDB".
- Instead of: the git revision pin of NEX-13, or vendoring a copy that drifts.
- Supersedes: 2026-11-15-NEX-13-nexus-flow-pins-a-git-revision-of-nocentraldb.md, 2026-11-21-NEX-211-the-nocentraldb-sdk-is-pinned-from-crates-io-at-an-exact-version.md
- Topics: nocentraldb, dependencies, v2
