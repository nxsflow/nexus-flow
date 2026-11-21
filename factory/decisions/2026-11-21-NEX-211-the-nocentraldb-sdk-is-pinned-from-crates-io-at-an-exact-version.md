# 2026-11-21 · NEX-211 · The NoCentralDB SDK is pinned from crates.io at an exact version

- Decided by: Carsten
- Why: RFC 0001 chose a git revision pin only because the SDK was unpublished. NoCentralDB 0.1.0-beta.1 is on crates.io since 2026-11-21. An exact `=` version keeps every move deliberate, needs no git source in `deny.toml`, and lets `cargo deny` and `cargo audit` see the crate like any other. The rest of the rule stays: one PR per move, carrying nothing else, waiting for the owner. Written into RFC 0001, "The dependency on NoCentralDB".
- Instead of: keeping the git revision pin `rev = "<full hash>"`.
- Topics: dependencies, nocentraldb
