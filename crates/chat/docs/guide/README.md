# `nxc guide` sources

`en/` is embedded into the binary (`include_dir!` in `src/guide.rs`); `de/` is website-only and
feeds the federated docs at `/open-source/docs`. One `<topic>.md` per topic, in BOTH locales,
each opening with an `# H1` — the parity guards fail closed on anything else:

- `nxs_guide::assert_catalog_parity` (Rust) holds `TOPICS` against `en/`.
- `assertTopicParity` in `content/lib/docs.mjs` holds the published order against `en/` AND `de/`.

**Seven topics.** Six were written by `6j6v.t6vd` on the mechanism `6j6v.9e3r` shipped —
`getting-started`, `core-concepts`, `commands`, `personas`, `channels`, `limits-and-safety` — and
`writing-declarations` joined them between `channels` and `limits-and-safety` (`6j6v.9w08`), because
writing quality is the last stage of "what you declare". The order is the reading order and the
agent contract for `nxc guide --json`. Adding a `<topic>.md` here means adding its line to `TOPICS`
in `src/guide.rs` AND to `BUILDING_BLOCKS` in `content/lib/docs.mjs`; both parity guards fail closed
if you do only one.

**The ` ```console ` blocks in these guides are executed.** `tests/guide_examples.rs` holds every
one of them, in BOTH locales, against the golden corpus (`tests/golden/*.trycmd`) — so a guide can
never show a command or an output that is not real. The worked session lives in
`tests/golden/messaging.trycmd`; regenerate it with
`cargo build -p nxs && TRYCMD=overwrite cargo test -p nexus-chat --test golden`, then carry the
changed blocks into BOTH locales. Illustrative ` ```bash ` / ` ```yaml ` snippets are deliberately
not checked.

Website slugs are `nxc-<topic>` (the three blocks share one document, so bare topic names would
collide). Cross-links between guides use that slug — `[channels](nxc-channels)` — and
`checkCrossLinks` fails the content build on a target that does not resolve.
