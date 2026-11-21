# 2026-11-21 · NEX-212 · A v2 skeleton declares the dependencies of the tickets it blocks

- Decided by: coder on NEX-212
- Why: a crate's dependencies are listed in its `Cargo.lock` entry, so a ticket that adds one to a crate changes the code-owned lockfile even when the root `Cargo.toml` already names it. The skeletons therefore declare now, through `[workspace.dependencies]`, the normal dependencies NEX-97 (clap, serde, serde_json, the seam), NEX-169 (pulldown-cmark, without its default `html` and `getopts` features, for the CommonMark list after the owner-rules mark) and NEX-177 (rusqlite, serde, toml) build with, and the owner reviews the lockfile once. A dependency a later ticket needs beyond these, its tests' dev-dependencies included, is its own small PR.
- Instead of: naming them only in the root `Cargo.toml`, which leaves every one of those tickets touching `Cargo.lock`, or a hand-written Markdown reader in place of pulldown-cmark, which RFC 0004's "CommonMark list" would make a parser of its own to keep correct.
- Topics: crates, dependencies, v2
