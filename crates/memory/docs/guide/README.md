# `nxm guide` sources

`en/` is embedded into the binary (`include_dir!` in `src/guide.rs`); `de/` is website-only and
feeds the federated docs at `/open-source/docs`. One `<topic>.md` per topic, in BOTH locales,
each opening with an `# H1` — the parity guards fail closed on anything else:

- `nxs_guide::assert_catalog_parity` (Rust) holds `TOPICS` against `en/`.
- `assertTopicParity` in `content/lib/docs.mjs` holds the published order against `en/` AND `de/`.

**Five topics, written by `6j6v.h4k0`** on the mechanism `6j6v.9e3r` shipped: `getting-started`,
`core-concepts`, `commands`, `agents-and-mcp`, `import-and-migration`. The order is the reading
order and the agent contract for `nxm guide --json`. Adding a `<topic>.md` here means adding its
line to `TOPICS` in `src/guide.rs` AND to `BUILDING_BLOCKS` in `content/lib/docs.mjs`; both parity
guards fail closed if you do only one.

**`getting-started` is an INTRODUCTION, not a third copy of the suite's setup** (`6j6v.4g1b`): what
this module is for, what it presupposes, and its first own command. Installing the suite and
creating a workspace belong to the `nxs` bracket (`6j6v.0fvt`), and the cross-link to it is
deliberately unwritten until that slug exists — `checkCrossLinks` makes an unknown target a build
failure, in both locales. `src/guide.rs` pins both halves of that cut in a test.

**The ` ```console ` blocks in these guides are executed.** `tests/guide_examples.rs` holds every
one of them, in BOTH locales, against the golden corpus (`tests/golden/*.trycmd`) — so a guide can
never show a command or an output that is not real. The worked session lives in
`tests/golden/guides.trycmd`, whose blocks are ONE command each wherever a guide might want that
command alone (a guide can only quote a block whole); regenerate it with
`cargo build -p nxs && TRYCMD=overwrite cargo test -p nexus-memory --test golden`, then carry the
changed blocks into BOTH locales. Illustrative ` ```bash ` snippets are deliberately not checked.

Website slugs are `nxm-<topic>` (the three blocks share one document, so bare topic names would
collide). Cross-links between guides use that slug — `[core concepts](nxm-core-concepts)` — and
`checkCrossLinks` fails the content build on a target that does not resolve.
