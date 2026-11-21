# 2026-11-21 · NEX-89 · The SDK boundary counts every ncdb-* crate as the SDK

- Decided by: coder on NEX-89
- Why: RFC 0001 wants a change in NoCentralDB's API at a pin move to touch one crate, and a crate that depended on one of NoCentralDB's own crates (`ncdb-*`) directly would break at that move just as one on `nocentraldb` would. The pin check (`sdk_pin.rs`) already treats `ncdb-*` as NoCentralDB's, so `cargo xtask sdk-boundary check` refuses a workspace crate other than `nexus-flow-store` that lists `nocentraldb` or any `ncdb-*` crate, in any dependency kind (normal, dev, build, target-specific) and under any rename.
- Instead of: checking the package `nocentraldb` alone, which a direct `ncdb-*` dependency would slip past.
- Topics: nocentraldb, dependencies, crates, v2
