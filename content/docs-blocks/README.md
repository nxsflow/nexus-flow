# Web documents — a block's second source

A building block has **two** sources of documentation, and the published feed carries them as one
list:

| Source | Path | Served by |
|---|---|---|
| Guide topics | `crates/*/docs/guide/<locale>/<topic>.md` | `<binary> guide <topic>` **and** the website |
| Web documents | `content/docs-blocks/<block>/<locale>/<name>.md` | the website only |

The layout is the same in both — `<locale>/<name>.md`, one file per locale, each opening with an
`# H1` — so there is nothing new to learn here. `assertTopicParity` in
[`../lib/docs.mjs`](../lib/docs.mjs) holds this tree against each block's `documents` list in
`BUILDING_BLOCKS`: a file nobody lists, or a listed name missing in one locale, fails the content
build rather than silently never publishing.

## Why web-only is a PLACE, not a switch

Two gates make every `.md` under a **guide** tree a topic the CLI serves — `assertTopicParity` on
this side, `the_*_embedded_dir_and_topics_list_are_in_sync` on the Rust side. A file dropped into a
guide tree that the binary does not know is a failing build, not a hidden page. A document that must
not reach `nxs guide` therefore needs a tree of its own.

`develop/getting-started` is the case that forces it: `nxs` already carries a `getting-started`, and
two topics of one name across one binary's two catalogs is a failing build too
(`topic_names_do_not_collide_across_the_two_nxs_catalogs` in `crates/nxs/src/guide.rs`). Renaming it
would have solved the collision and cost the convention the website's interpreter stands on — the
one block whose entry document is called something else. **Noted loss:** `nxs guide` has no page
that tells a contributor how to clone and build, and contributors sit in a terminal. If that bites,
the content can later become a guide topic under a non-colliding name, and the web document points
at it.

## What lives here

Each non-`nxs` block owes the website an `index` — its own introduction, which the landing renders
at `/open-source/docs/<block>` and stacks onto the docs start page. That is a **convention the
assembler enforces**, not a habit: `buildDocs` throws when a block that publishes pages has no
index, when no `nxs` block carries a `getting-started` (the start page itself), and when a document
here shadows a guide topic of the same block. They are semantic rules, so the JSON schema cannot
express them — the throw is what actually stops a broken feed before it is published.

The authoring rule is sharp, because those introductions are read one after another on that one
page:

> **A block index must not repeat what `nxs/getting-started` has already said** — no installation,
> no `nxs init`, no workspace layout. It introduces ITS block: what the block is for, what the first
> step inside it is, and where to go from there.

Cross-links use the published slug (`[commands](nxf-commands)`), the same as in the guides;
`checkCrossLinks` fails the build on a target the landing could not resolve. Fenced ` ```console `
blocks are **not** executed here — unlike the guide trees, this tree has no crate test behind it, so
keep samples to what the guides already prove.
