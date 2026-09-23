# nxsflow.com migration — delivery re-homing to manufakt + `/nxs` endpoints

- **Date:** 2026-07-14 (design approved in session `migrate-nxsflow-com`)
- **Status:** **EXECUTED, THEN OVERTAKEN — historical record as of 2026-08-23.** The move to
  `/nxs` endpoints happened as designed. What did *not* hold is the DESTINATION: this document
  places the delivery origin in the **manufakt (manufaktio) accounts**, and the owner decision of
  2026-08-11 moved it on to the **nxsflowcom** accounts instead — the principle was "dedicated
  rather than shared", and nxsflowcom is equally dedicated *and* owns the domain and the
  distribution (`6j6v.sgfr`). With `cyb7.rhb5`/`cyb7.5bwm`/`cyb7.qe0q` that second move is
  complete and verified against a real self-update, and `6j6v.sgfr` has since deleted `infra/`
  and the five infra workflows from this repo altogether. **Read every "manufakt account" below
  as "nxsflowcom account", and every `nexus-flow (infra/)` instruction as a description of work
  that has been done and then re-homed.**
- **Coordinating repo:** `nexus-flow` (this repo); work also lands in
  `nxsflow-landing-page`, `nxsflow-com-dns`, and (one record) `central-dns-setup`
- **Supersedes where they conflict:** the landing architecture spec
  (`nxsflow-landing-page/docs/superpowers/specs/2026-06-15-nxsflow-landing-architecture-design.md`)
  — specifically its `/flow/*` machine namespace and its choice of the coding-agent
  account as the delivery origin. Its architecture (headless content federation,
  origin pinning analysis, master-cutover shape) otherwise stands and is consumed
  here. DNS mechanics remain SSOT'd by the D4 epic in `nxsflow-com-dns`.

> **No AWS account IDs in this document.** This repo is going public; accounts are
> referred to by SSO profile name only. IDs live in CI secrets and the (private)
> sibling repos.

---

## 1. Context & goals

Today `nxf.nxsflow.com` serves the whole public surface of this project from the
shared coding-agent AWS accounts (which will soon be suspended): the marketing
site (`site/`), the docs (`/docs`, `/de/docs`), `install.sh`, the artifact
downloads, and the self-update chain (updater Lambda + signed manifests). The chain
is live and verified (0.27.x clients update over it).

Three things have changed since that setup was built:

1. **The suite consolidated.** nexus-flow, nexus-memory, and nexus-chat ship as
   **one `nxs` multicall binary** with **one changelog** and **one documentation**.
   A product-named subdomain (`nxf.…`) and a product-named endpoint namespace
   (`/flow/*`) no longer describe what is delivered.
2. **The brand domain gets a real landing page.** `nxsflow.com` (repo
   `nxsflow-landing-page`, hosted from `nxsflowcom-{prod,staging}`) owns the human
   surface: shell, design system, story. Product content is federated into it as
   data, not hosted as sibling sites.
3. **Delivery moves to dedicated accounts.** Binaries and the federated content are
   served from the new `manufaktio-{prod,staging}` accounts instead of the shared
   coding-agent accounts.

The migration is a **gate for `6j6v.fasc`** (making this repo public): the decision
is that public-going waits for the **full migration including teardown** — new
chain live, both machines confirmed on it, `nxf.nxsflow.com` and the coding-agent
delivery stacks gone, and every repo reference pointing at `nxsflow.com`.

## 2. Decisions (all approved 2026-07-14)

| # | Decision |
|---|---|
| A1 | **Manufakt origins are domain-less.** `manufaktio-*` host only an artifact bucket (+ content prefix) and the updater Lambda. No hosted zone, no cert, no CloudFront of their own — the landing distribution fronts them cross-account. |
| A2 | **Full re-home with teardown.** The coding-agent chain is frozen (no new wiring), serves `nxf.nxsflow.com` unchanged as the safety net, and is torn down after both machines are confirmed on the new endpoint. |
| A3 | **Human page: `/open-source`** (`/de/open-source`, `/open-source/docs`). The already-built product-agnostic page represents the whole suite. |
| A4 | **Machine namespace: `/nxs/*`** — `updater`, `latest`, `download/*`, `install.sh`, `release-notes.json` (+ `content.json` and docs objects for the build-time federation). Named after the artifact it delivers; survives any page/story rename. Endpoint paths are immortal once shipped in a release. |
| A5 | **Docs federation is in scope.** `docs.json` (was "optional, later") ships with this migration; `/open-source/docs` must be live before the old domain dies. |
| A6 | **Sequencing "manufakt first".** Fresh origins are stood up first; the landing wires `/nxs/*` against manufakt from day one. The customer-visible chain is migrated exactly once. |
| A7 | **Infra namespace renames to `nxs-*`** (stacks `NxsGitHubDeploy`/`NxsDelivery`, roles `nxs-infra-deploy-*` / `nxs-<env>-release`, SSM `/nxs/infra/*`, bucket `nxs-<env>-artifacts-*`) — free in fresh accounts. **Tarball file names stay `nxf_<v>_…` for now** (clients consume them via manifest `url`; renaming is a separate, non-blocking decision). |
| A8 | **Same minisign keypair, same embedded pubkey.** Signing is host-independent; changing the key would make old clients reject the transition release. |
| A9 | **Dual-publish window.** `release.yml` publishes to both chains during the migration so 0.27.x clients can reach the transition release through the old endpoint. Ends at teardown. |
| A10 | **`fasc` gate = step 9 (teardown) of the master sequence** (§6), in addition to its existing E9/MCP dependency. |

## 3. Target topology

| Account (profile) | Role |
|---|---|
| `manufaktio-prod` | Delivery origin, prod: artifact bucket (tarballs, `install.sh`, `release-notes.json`, `.sha256`/`.minisig` sidecars, private `manifests/` prefix) + content prefix (`content.json`, `docs.json`, docs pages) + updater Lambda (function URL, `AWS_IAM` + OAC). |
| `manufaktio-staging` | Mirror of prod + the E2E write permissions the install/update smoke suite needs. |
| `nxsflowcom-prod` | Landing site + `nxsflow.com` distribution + new hosted zone (exists). Gains the `/nxs/*` behaviors → manufakt-prod. |
| `nxsflowcom-staging` | `staging.nxsflow.com` — the identical chain against manufakt-staging; the rehearsal environment for everything. |
| `nxs-coding-agent-{prod,staging}` | **Frozen.** Serve `nxf.nxsflow.com` / `nxf-staging.nxsflow.com` unchanged until teardown (§6 step 9). |

**Public endpoints** (locale-free machine paths; human paths locale-aware):

| Human | Machine |
|---|---|
| `nxsflow.com/open-source`, `/de/open-source` | `nxsflow.com/nxs/install.sh` (TTL 300 s) |
| `nxsflow.com/open-source/docs`, `/de/…` | `nxsflow.com/nxs/updater` (manifest, same-origin `url`) |
| | `nxsflow.com/nxs/latest` (302 → `/nxs/download/…`) |
| | `nxsflow.com/nxs/download/*` (tarballs + sidecars) |
| | `nxsflow.com/nxs/release-notes.json` (the ONE suite changelog) |
| | `nxsflow.com/nxs/content.json` + docs objects (build-time federation input) |

Staging is byte-identical under `staging.nxsflow.com`. Optional later nicety:
`nxsflow.com/install.sh` → 301 `/nxs/install.sh` for the hero command.

**Origin-pinning invariant (unchanged, consumed):** `install.sh` (`same_origin()`)
and the CLI (`install_from()`) reject tarball URLs on a different origin than the
manifest request. The updater Lambda builds `url` from `PUBLIC_BASE_URL =
https://nxsflow.com/nxs` (staging: `https://staging.nxsflow.com/nxs`), and
`/nxs/download/*` is a behavior on the same distribution — pinning holds while the
bytes come cross-account.

## 4. Content & docs federation (the new mechanism)

The landing owns shell, design, and components; this repo delivers **data**. The
`site/` renderer dies; a **provider** replaces it:

- **`content.json`** — assembled by a small provider directory (successor of
  `site/`): suite story (one binary, three products), hero with
  `curl -fsSL https://nxsflow.com/nxs/install.sh | sh`, terminal demo (real `nxf`
  transcript, re-captured not hand-edited), feature grid, changelog section with
  `feed: /nxs/release-notes.json`, download CTA on `/nxs/latest`.
- **`docs.json`** — built from `crates/{cli,memory,chat}/docs/guide/{en,de}/`, one
  tree per building block (6j6v.9e3r): navigation tree + pages **referenced
  per-URL** (pages uploaded as separate markdown objects — no monolith), grouped by
  block through the contract's additive `groups` level (cyb7.93q6; `contract` stays
  `"1"` and a renderer without group support renders flat). Slugs are
  `<binary>-<topic>`, since the three blocks share one page and its slugs are
  duplicate-free by contract. This is the suite's one documentation; it grows without
  changing the mechanism. Rendered behind the landing's markdown sanitizer
  (allowlist, no raw HTML) — the trust boundary named in the landing spec §4.6.
- **CI validation against the landing-published JSON schema** — fail-closed at the
  source; a contract break stops in this repo's CI, not at the consumer.
- **Pipeline:** new workflow `publish-content.yml` (replaces `site.yml`): on `main`
  push touching content sources → assemble → validate → upload to the manufakt
  content prefix (OIDC) → `repository_dispatch` to `nxsflow/nxsflow-landing-page`.
  The landing rebuild fetches `OPEN_SOURCE_CONTENT_URL =
  https://nxsflow.com/nxs/content.json` (staging: per-env var against
  `staging.nxsflow.com`). The landing's Last-Known-Good seed decouples its deploys
  from provider outages; the staleness backstop (periodic rebuild + alarm) catches
  missed dispatches.
- **Contract delta:** `/open-source` now represents the suite, not just flow. The
  contract is already product-agnostic (discriminated section union); whatever the
  new story needs arrives as **additive optional section types** — no major bump,
  unknown-type-skip applies. The open landing items (contract tests, schema
  publication, trust-boundary rendering, LKG hardening) close in the same stroke;
  `docs.json` flips from optional to required (A5).
- **Changelog:** `release-notes.json` stays the single feed (release bodies,
  self-update notes, web changelog) — served at `/nxs/release-notes.json`; prod
  shows the stable ring, staging previews beta (as today).
- **Legacy links:** during the deprecation window the **old** distribution 301s
  `nxf.nxsflow.com/docs*` → `nxsflow.com/open-source/docs` (CloudFront function on
  the old distribution). The window ends at teardown.
  **Not delivered (2026-08-08, `6j6v.hy2f`):** the WAIT gate `6j6v.gwvw` — `nxf`/`nxf-staging`
  leaving the new zone's `delegations[]` — closed *before* the teardown ran, so both names were
  already NXDOMAIN when the window was due. A 301 needs a hostname that resolves; there was
  nothing left to redirect from. Re-delegating the names just to serve a redirect would have
  meant re-issuing the ACM cert and standing the old distribution back up, so the window was
  dropped rather than resurrected. **Lesson for the next re-home:** the redirect window must be
  opened *before* the delegation is withdrawn — the DNS removal, not the stack deletion, is what
  closes it.

## 5. Infra changes per repo

**`nexus-flow` (`infra/`) — DONE, AND SINCE DELETED (`6j6v.sgfr`).** Everything in this
paragraph happened; the tree it happened in is gone, and the stack it describes now lives in
`nxsflow-landing-page` in the nxsflowcom accounts. The `NXS_{STAGING,PROD}_ACCOUNT_ID` secrets
survive and still drive `release.yml`/`promote.yml`/`publish-content.yml`; the
`NXS_<ENV>_LANDING_DISTRIBUTION_ARN` secrets were the cross-account pin and went with the tree,
since same-account delivery does not need one. The CDK app slims down to the domain-less shape and
renames (A7): `NxsGitHubDeploy-<env>` (trust anchor, one-time manual bootstrap per
manufakt account) and `NxsDelivery-<env>` (bucket incl. content prefix, updater
Lambda, release/content-publish roles). **Removed:** cert stack, hosted-zone/alias
wiring, site bucket, CloudFront. The Lambda gets the new `PUBLIC_BASE_URL`; its
function-URL invoke permission pins the **landing** distribution ARN
(cross-account `sourceArn`), and the bucket policy grants that distribution OAC
read. Those two ARNs are the **coordination contract** between the repos: stable
values, kept as per-env GitHub vars/secrets, documented in both READMEs. New CI
secrets `NXS_{STAGING,PROD}_ACCOUNT_ID` live beside the old `NXF_*` ones for the
dual-publish window. The existing install/update smoke suite retargets to
`staging.nxsflow.com/nxs` (which makes landing-staging availability a declared CI
dependency — accepted: staging rehearses exactly the prod shape).

**`nxsflow-landing-page` (`infra/`).** Behaviors on both distributions:
`/nxs/updater*`, `/nxs/latest*` → Lambda function URL (OAC, CACHING_DISABLED);
`/nxs/download/*`, `/nxs/install.sh` (TTL 300 s), `/nxs/release-notes.json`,
`/nxs/content.json` + docs objects → manufakt bucket (OAC). Behavior precedence
before `default` is asserted in CDK tests. ACM ordering as planned (validation
CNAME in old + new zone; cert issued pre-flip, auto-renew survives the flip).

**`nxsflow-com-dns` + `central-dns-setup`.** D4 stays SSOT and unchanged (cert
`sfvh.2kg0`, functional mail/apex gate `sfvh.acwc`, registrar NS flip `sfvh.frj4`,
two-zone sync `sfvh.jakn`). **New before the flip:** the legacy zone delegates
`staging.nxsflow.com` to the new staging zone (one record, owner:
`central-dns-setup`) so the full staging chain resolves publicly pre-flip. **New
after teardown:** `nxf`/`nxf-staging` leave the `delegations[]` of the new zone
configs.

## 6. Master cutover sequence & gates

Every arrow is a gate. Steps 1–5 are additive and reversible; the old chain is
untouched until step 9.

1. **Manufakt bootstrap** (one-time, manual): SSO access, `cdk bootstrap`
   (eu-central-1), GitHub OIDC provider, `NxsGitHubDeploy` trust anchors.
   *Gate:* CI `cdk deploy` green (staging).
2. **Origin stacks live** (staging → prod) + `release.yml` dual-publishes.
   *Gate:* a release rehearsal writes artifacts into both staging buckets.
3. **Landing behaviors + content pipeline** (staging): `/nxs/*` against
   manufakt-staging; `publish-content.yml` delivers `content.json`/`docs.json`;
   dispatch triggers a staging landing build.
   *Gate:* staging landing renders `/open-source` (+ docs) from federated content.
4. **Staging cut-through:** legacy zone delegates `staging` →
   `staging.nxsflow.com/nxs/*` resolves publicly.
   *Gate:* install/update smoke green against staging — same-origin manifests,
   minisign/SHA-256 verified.
5. **Prod ready (not yet authoritative):** ACM cert issued, prod landing
   distribution carries apex/`www` alias config, `/nxs` behaviors against
   manufakt-prod, functional gate `acwc` (mail + apex completeness) green.
6. **D4 registrar NS flip** (`frj4`; rollback per runbook `cz30`).
   *Gate:* `dig` shows the new NS; apex serves the landing.
7. **Prod smoke:** all `/nxs/*` endpoints same-origin-valid; `/open-source`
   (+ docs) live with the new story.
8. **Transition release:** `DEFAULT_BASE_URL = https://nxsflow.com/nxs`
   (compile-time constant in `crates/cli/src/selfupdate.rs`; `NXF_BASE_URL`
   override unchanged), `install.sh` + README/guide/MCP references updated;
   dual-published. Both machines update **through the old chain** onto it.
   *Gate:* both machines confirmed on the new endpoint.
9. **Teardown:** short 301 window → delete old distributions + coding-agent stacks
   + `nxf`/`nxf-staging` delegations + old zones; dual-publish → single; retire
   `NXF_*` secrets. **This unblocks `fasc`** (which additionally waits on E9/MCP).
   **Done 2026-08-08 (`6j6v.hy2f`).** `NxfDistribution-<env>`, `NxfCert-<env>`,
   `NxfGitHubDeploy-<env>` and the `DelegatedSubdomain[Cert]-nxf[-staging]-nxsflow-com` stacks
   are deleted in both coding-agent accounts, with the hosted zones, CloudFront distributions
   and ACM certificates; `release.yml`/`promote.yml` publish to the `nxs` chain only; the
   `NXF_*` account/zone secrets are deleted. The 301 window was **not** delivered (see §4,
   "Legacy links"). Still open: account suspension (owner) and the retained
   `nxf-<env>-artifacts-*` buckets that go with it.

**Rollback posture:** through step 6 the old chain is authoritative for clients and
untouched. After step 8 the two machines have no path back — which is exactly why
the old chain stays up until they are confirmed (plus the manifest kill-switch
`enabled:false` as the emergency brake).

## 7. Story update (two owners)

- **Main page `/`, `/de`** (owned by the landing repo): the product family retold —
  two flagships (**manufakt.io**, private beta live: "You lead, the agents ship";
  **nexflow.it**, in progress/waitlist) carried by **one open foundation**: the
  `nxs` platform — three products, one binary: flow (the record of work), memory
  (durable knowledge), chat (agent coordination). Sources: this repo's README, the
  fresh landing copy in `manufakt-io`/`nexflowit-app`, the `nxsflow-brand` skill.
- **`/open-source`** (content owned by this repo via `content.json`): the suite
  story in federated form — §4 is the mechanism, the story update is content work
  inside it.

## 8. Operations

Existing planned items, retargeted at `/nxs`: CloudWatch Synthetics on
`/nxs/updater|latest|install.sh` (deploy-time smoke is not enough — the endpoint
must stay observed through deploy-free months), CloudFront 4xx/5xx alarms,
cert-expiry + DNS monitor (is the ACM validation CNAME alive?), staleness alarm
for federated content, S3 versioning + rollback runbook on both sides (landing
bucket; manufakt artifacts/manifests). All alarms live in the landing accounts
(they own domain + distribution) except bucket versioning, which lives with
manufakt.

## 9. Board wiring (three boards + `fasc`)

Principle: each workspace tracks its own work; deliveries from elsewhere are open
`WAIT:` chores that dependents `dep add` onto (closed with the delivered version
in the reason).

- **`6j6v` (this repo):** new epic **"Delivery re-home to manufakt + `/nxs`
  (nxsflow.com migration)"** with items: (1) infra rebuild
  `NxsGitHubDeploy`/`NxsDelivery` + manual bootstrap, (2) `release.yml`
  dual-publish + new secrets, (3) provider (`content.json`/`docs.json` assembler +
  schema validation + `publish-content.yml`), (4) transition release
  (`DEFAULT_BASE_URL`, `install.sh`, README/guide/MCP references), (5) confirm both
  machines on the new endpoint, (6) teardown coding-agent chain + end dual-publish
  + docs. WAIT chores: "WAIT: `/nxs` behaviors live on nxsflow.com (landing)",
  "WAIT: D4 registrar NS flip (dns)". **`fasc` gets `dep add` onto the teardown
  item** (keeps its E9 dependency).
- **`cyb7` (landing):** edit open items `/flow` → `/nxs` and "nexus-flow account" →
  "manufakt account" (`y74k`, `zhxk`, `9m7k`, `dyf9`); `ejnv` (docs.json) → P1
  required; new items: main-page story update, `OPEN_SOURCE_CONTENT_URL` per-env
  wiring. Items describing nexus-flow-side work (`q4v3`, `ag6x`, `zqwj`, `9bq9`)
  become WAIT chores on the `6j6v` deliveries — the real work lives here.
- **`sfvh` (dns):** new: "legacy zone delegates `staging` (coordinate with
  central-dns-setup)" pre-flip; "remove `nxf`/`nxf-staging` delegations"
  post-teardown. D4 items unchanged.

## 10. Out of scope / follow-ups

- Tarball file rename `nxf_…` → `nxs_…` (non-blocking, manifest-driven; decide at
  a later release).
- `nxsflow.com/install.sh` short alias (301).
- WAF/rate-limiting for `/nxs/*` (existing follow-up, unchanged).
- Re-homing the remaining delegations (`bd`, `nxc`, `overtone`) and legacy-zone
  decommissioning beyond D4's own scope.
- `/chat` federation (needs a nexus-chat provider).
- manufakt.io brand reconciliation.

## 11. References

- Landing architecture (mechanism SSOT): `nxsflow-landing-page/docs/superpowers/specs/2026-06-15-nxsflow-landing-architecture-design.md`
- DNS toolkit + D4 cutover (DNS SSOT): `nxsflow-landing-page/docs/superpowers/specs/2026-06-15-dns-toolkit-design.md`, epic `sfvh.6d32` in `nxsflow-com-dns`
- Release management (delivery + update contract): `docs/specs/release-management.md`
- Current delivery infra: **not in this repo** — see `nxsflow-landing-page` (`src/infra/`).
  `infra/README.md` was the answer until `6j6v.sgfr` deleted the tree.
