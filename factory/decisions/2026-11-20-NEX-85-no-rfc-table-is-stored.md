# 2026-11-20 · NEX-85 · No table of RFCs is stored; cargo xtask rfc-index prints the index

- Decided by: pm
- Why: A table of RFCs in `docs/rfc/README.md` is a file that every RFC pull request shares. If a hand-kept table puts open RFC pull requests in conflict, a generated one needs a regeneration pull request after every merge. Now the RFC files are the index: `cargo xtask rfc-index` prints number, title and status from each file's heading and `Status:` line, and `--md` prints a Markdown table. An RFC pull request touches no shared file, nothing goes stale, and no CI job has to guard it.
- Instead of: a table generated between markers in README.md, checked by `cargo xtask rfc-index --check` on `v2`. The `v2` ruleset lets no CI step push the regenerated table, so every RFC merge would have been followed by a regeneration pull request.
- Topics: rfc, factory
