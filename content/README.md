# content provider — federated `/open-source` content

Successor of the retired `site/` renderer (nxsflow.com migration, spec
`docs/specs/nxsflow-com-migration.md` §4, item `6j6v.0a6p`).

This repo no longer hosts a landing site. It delivers **data**: the
[`nxsflow-landing-page`](https://github.com/nxsflow/nxsflow-landing-page) shell owns layout,
design, and components, and renders the suite story on the `nxsflow.com` homepage (it was the
`/open-source` page until 2026-09) from the federated content this provider assembles.

## Outputs

`node assemble.mjs` writes an upload-ready tree under `dist/`:

| File | Uploaded to | Served at |
|---|---|---|
| `dist/content.json` | `s3://nxs-<env>-artifacts/content.json` | `/nxs/content.json` |
| `dist/docs.json` | `s3://nxs-<env>-artifacts/docs.json` | `/nxs/docs.json` |
| `dist/docs/<locale>/<slug>.md` | `s3://nxs-<env>-artifacts/docs/<locale>/<slug>.md` | `/nxs/docs/<locale>/<slug>.md` |

- **`content.json`** — the suite story as a versioned, discriminated **section union** (hero,
  terminal-demo, feature-grid, changelog, powered-by, download-cta), per locale. The wire shape is
  the landing's **content-contract v1**; it is validated against the published JSON schema in CI
  (fail-closed at the source — a contract break stops here, not at the consumer).
- **`docs.json`** — a navigation tree + **per-URL** page refs, grouped into the suite's three
  **building blocks** (`groups`, added additively by the landing as cyb7.93q6 and filled here by
  6j6v.9e3r — `contract` stays `"1"`, and a renderer without group support renders flat). The page
  bodies are uploaded as **separate** markdown objects under `docs/*` (no monolith); the landing
  renders each behind its markdown sanitizer.

## Sources (canonical, never hand-copied)

- Story copy: [`data/story.mjs`](data/story.mjs) (EN + DE), grounded in the repo `README.md`.
- Terminal demo: [`data/terminal-demo.json`](data/terminal-demo.json) — **generated + drift-guarded**
  by `crates/cli/tests/terminal_demo.rs` (real `nxf` output; regenerate with
  `UPDATE_DEMO=1 cargo test -p nexus-flow-cli --test terminal_demo`).
- Changelog entries: the repo-root `release-notes.json` (prod embeds the stable ring, staging the
  beta ring), plus `feed: /nxs/release-notes.json`.
- Docs: `crates/{nxs,cli,memory,chat}/docs/{guide,develop}/{en,de}/*.md` — one tree per building
  block plus the umbrella's second one for the `develop` group, the same
  guides `nxf guide` / `nxm guide` / `nxc guide` serve (and `nxs guide` fans out over), so the web
  docs can never drift stale-but-green from the CLI. `lib/docs.mjs`'s `BUILDING_BLOCKS` is the one
  list that drives them; all three trees carry guides now (`6j6v.t6vd` wrote chat's first six and
  `6j6v.9w08` added `writing-declarations` to them, `6j6v.h4k0` memory's five), and a block with no
  topics contributes no group at all — the state a
  fourth block would be in between its mechanism and its content (the contract rejects an empty
  one).
- Web documents: [`docs-blocks/<block>/{en,de}/*.md`](docs-blocks/README.md) — a block's **second**
  source (`6j6v.1pxs`), published beside its guide topics and before them. Website-only: it carries
  each non-`nxs` block's `index` and develop's `getting-started`, which cannot be a guide topic
  because `nxs` already serves one. Web-only is a PLACE, not a switch — a `.md` under a guide tree
  that the binary does not know is a failing build. Each nav entry says which source it came from
  (`source: "guide" | "web"`); the list itself is source-blind.

- Page summaries: `BUILDING_BLOCKS[].summaries` in [`lib/docs.mjs`](lib/docs.mjs) — one line per
  page per locale, which becomes that page's `<meta description>` (`6j6v.5cxr`). Mandatory: a page
  without one fails the build. The English half of every **guide** topic is held against
  `crates/nxs/tests/golden/guide.trycmd` — the exact `nxs guide --json` output — so the feed and the
  CLI cannot say two different things; German and the web documents exist nowhere else and are held
  on presence.

  Published slugs are **`<binary>-<topic>`** (`nxf-getting-started`): the three blocks share one
  document, so bare topic names would collide, and docs.json slugs are duplicate-free by contract.
  Cross-links between guides use that slug, and `checkCrossLinks` fails the build on any target the
  landing's `docsHref` could not resolve — an un-carried cross-reference degrades to plain text with
  no error otherwise. flow's eight topics are ALSO written under their pre-rename keys
  (`docs/<locale>/<topic>.md`) so existing deep links keep resolving.

## Commands

```bash
npm ci                 # locked install (ajv, the only dep)
npm test               # node --test — the provider unit tests
node assemble.mjs [--out dir] [--channel stable|beta] [--base-path /nxs]
node validate.mjs [--content dist/content.json] [--schema <url|path>]
```

## CI

- **`content-ci.yml`** (PR gate, replaces `site-ci.yml`): locked install → unit tests → assemble →
  validate against the vendored contract mirror.
- **`publish-content.yml`** (deploy, replaces `site.yml`): assemble → validate (vendored **and**, when
  configured, the landing-published schema) → OIDC upload to the `nxs-<env>-content-publish` role's
  content prefix → `content-updated` `repository_dispatch` to the landing (prod).

## Owner configuration

Two coordination handles (no account ids / URLs are committed — this repo goes public):

- **`vars.CONTENT_CONTRACT_SCHEMA_URL`** (repo *variable*): the landing-**published** content-contract
  schema URL. Until the apex go-live, set it to the landing **prod-distribution** URL
  (`https://<prod-distribution>/schema/content-contract-v1.json`); after go-live,
  `https://nxsflow.com/schema/content-contract-v1.json`. When unset, `publish-content.yml` validates
  only against the vendored mirror and emits a warning (drift at the source is then not caught).
- **`secrets.LANDING_DISPATCH_TOKEN`** (repo *secret*): a cross-repo token (fine-grained PAT or App
  installation token) allowed to `POST /repos/nxsflow/nxsflow-landing-page/dispatches`. Enables the
  prod `content-updated` dispatch; when unset the dispatch is skipped and the landing's staleness
  backstop picks the content up.

`schema/content-contract-v1.json` is a **vendored mirror** of the landing-published schema, kept for
offline/dev + the test-suite gate. `NXS_<ENV>_ACCOUNT_ID` (shared with the release dual-publish
window) resolves the target account/bucket.
