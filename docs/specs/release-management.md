# Release Management, CI & Distribution for nexus-flow (Design Spec)

> Status: Design · As of: 2026-06-09 · Reference: `beads-dashboard` (release pipeline, epics
> `iou`/`6pd`/`z5n`, `docs/RELEASING.md`, `infra/`, `site/`)
>
> **⚠️ THE DELIVERY INFRASTRUCTURE DESCRIBED HERE NO LONGER LIVES IN THIS REPO (2026-08-23,
> `6j6v.sgfr`).** `infra/` — the CDK app, the updater Lambda, the CI trust gates and the E2E
> suite — is deleted, together with `infra-ci.yml`, `infra-staging.yml`, `infra-prod.yml`,
> `infra-staging-release.yml` and `staging-gate.yml`. The bucket, the distribution and the
> updater are owned by **`nxsflow-landing-page`** (`cyb7.rhb5`/`cyb7.qe0q`); `site/` had already
> gone with the nxsflow.com migration (`6j6v.0a6p`).
>
> **What this repo still owns is the RELEASE, not the delivery:** `release.yml`, `promote.yml`,
> `publish-content.yml` and the two publish/promote scripts. They address the far side purely by
> convention — a role named `nxs-<env>-release` and a bucket named `nxs-<env>-artifacts-<account>`,
> both built at run time from `secrets.NXS_<ENV>_ACCOUNT_ID` — which is why the account move
> reached them without a single edit, and why they survived the teardown untouched.
>
> Every mention of `infra/**`, `site/**` and the five removed workflows below is therefore a
> HISTORICAL RECORD of how this was built, not an instruction. Where this document and
> `docs/specs/nxsflow-com-migration.md` conflict, the migration spec wins.
> This spec adapts the **proven release, CI, and publishing model of
> `beads-dashboard`** (GitHub = source, AWS = delivery, channel-ring model,
> landing page on S3/CloudFront) to the nexus-flow engine. Where the reference model
> does not map 1:1 (CLI instead of desktop app), the gaps were closed via
> best-practice research. **Every uncertainty and its resolution is documented in the
> tie-breaker log (§14).**

## 1. Goal & Scope

nexus-flow gets a complete release infrastructure:

1. **Versioning & tagging** — one workspace version, tag as truth, CI guard.
2. **Release pipeline** — tag build → GitHub pre-release (beta ring) → S3/CloudFront delivery
   → promotion to stable **without a rebuild** (build-once-promote).
3. **Self-update & install** — `curl https://nxsflow.com/nxs/install.sh | sh` and
   `nxs self-update`, both against the same channel-manifest endpoint, signature-verified.
4. **Changelog system** — PR fragments (`changes/`, EN+DE) → `release-notes.json` as a
   single-source feed for the GitHub release body, self-update notes, and website changelog.
5. **AWS infrastructure (CDK)** — two accounts (staging/prod), private buckets behind
   CloudFront, updater Lambda, OIDC roles without long-lived keys.
6. **Landing page** — statically prerendered (EN+DE), dedicated site bucket, dedicated
   deploy role, changelog section from the feed.
7. **Consumer & maintenance contract** — the SemVer stability promise for the
   `nexus-flow-facade` crate (§4.3), the git-tag-pin / auto-adopt model for embedding consumers
   (§4.4), and the security-backport policy across maintenance lines (§4.5). This is the contract
   manufakt.io consumes when it embeds the facade.

**Not in this spec:** hosting the `nxf-relay` as a managed service (belongs to the
SaaS track, E4 follow-up slices), crates.io publishing (deferred, §14 TB-8), Windows support
(Phase 2, §14 TB-5).

## 2. The reference model — what gets carried over

`beads-dashboard` has a complete, production-proven model. These principles
are **carried over unchanged**:

| Principle | Meaning |
|---|---|
| **GitHub = source & showcase, AWS = delivery** | Tag triggers CI; the GitHub release is the human-readable history; S3 + CloudFront + Lambda deliver downloads & updates (staged rollout, kill switch). |
| **Channel rings, build-once-promote** | The version is **plain SemVer** `X.Y.Z` (suffixes are rejected by the tooling). Channels (`alpha`/`beta`/`stable`) are **promotion rings**, never part of the version. Every tag build lands in the beta ring (GitHub **pre-release**); flipping to "latest" promotes **the same bytes** to stable (S3 server-side copy). Validated bytes == shipped bytes. Stable may skip versions. |
| **Tag = truth, guard in front** | Annotated tag `v<version>`; a `version-check` workflow enforces file consistency on every PR and tag==version on every release tag — fail-closed, before the build runs. |
| **Changeset fragments instead of commit messages** | Every PR with user impact adds a fragment under `changes/` (EN **and** DE, `type: added\|changed\|fixed\|removed`); CI gate `changelog-check`, which waives the fragment by itself when nothing the PR touches ships and otherwise takes the `skip-changelog` label as the opt-out. The release cut consumes fragments into `release-notes.json` (beta ring); promotion aggregates "everything since the last stable". |
| **Staging = pipeline rehearsal, not a release ring** | Staging is not version-coupled to prod; it exists to rehearse the pipeline (publish script, Lambda, manifest shape) safely. The release pipeline's `workflow_dispatch` publishes to the staging bucket, **without** a GitHub release. |
| **Two AWS accounts, OIDC, least privilege** | Staging/prod strictly separated; all CI auth via GitHub OIDC (no long-lived keys); release role (only `download/*`, `manifests/*`, feed) separated from site-deploy role (only the site bucket) separated from infra-deploy role (only CDK bootstrap roles). Prod release subjects: **tag refs only**. *(Since `6j6v.sgfr` this repo assumes only the release and content-publish roles; the infra-deploy role and the stack that defines all of them belong to `nxsflow-landing-page`.)* |
| ~~**Staging gate with label lease**~~ **— REMOVED (`6j6v.sgfr`)** | It leased a shared staging *deploy* environment to one PR at a time, keyed on `infra/**` or `site/**`. Neither tree is in this repo any more, so the gate guarded nothing and was deleted with them. Recorded rather than dropped because the shape is worth keeping: a label-lease decided by pure, unit-tested logic, read via `workflow_run` (a label set by `GITHUB_TOKEN` raises no `labeled` event), with a deploy-time re-check of `author_association` as defence in depth. |
| **Kill switch & rollback = manifest operation** | Artifacts are versioned and immutable (`download/<channel>/<version>/…`); the moving part is `manifests/<channel>.json`. `"enabled": false` stops updates immediately; rollback = point the manifest back at an older known-good version. No rebuild, no tag. |
| **Security hygiene** | Never interpolate tag/PR data into shell text via `${{ }}` (injection); pass it as env variables instead. `head-object` instead of `s3 ls`, so that IAM errors (403) are not masked as "not found" (404). For promotion, the release role needs **Read+List**, not just Write (learned from the v0.3.0 promote bug). |

## 3. Delta analysis — what is different in nexus-flow

| Dimension | beads-dashboard | nexus-flow | Consequence |
|---|---|---|---|
| Artifact | macOS desktop app (Tauri, universal binary) | **CLI `nxf`** + relay `nxf-relay` (Rust, multi-platform) | Build matrix instead of one universal build; install script + `nxf self-update` instead of the Tauri updater; no DMG |
| Update mechanism | Tauri updater plugin (minisign, in-app) | **`nxf self-update`** (dedicated subcommand, minisign-verified) | Updater Lambda contract stays almost identical; response shape minimally extended (sha256) |
| Platforms | macOS (arm64+Intel lipo'd) | macOS arm64/x86_64, Linux x86_64/aarch64 (musl, static); Windows Phase 2 | Manifest gets 4 platform keys instead of a 2-into-1 artifact |
| Version files | package.json, tauri.conf.json, Cargo.toml, Cargo.lock | `[workspace.package]` in `Cargo.toml` + `Cargo.lock` | Fewer drift surfaces; tooling as `xtask` (Rust) instead of Node scripts |
| Audience | end users (dogfooding team) | **developers & agents** (adoption engine, open source) | Landing page = developer marketing; install command in the hero; the GitHub release is the showcase |
| Repo visibility | private | eventually **public** (open core) | Trust gates of the workflow_run deploys are not optional but mandatory |
| Second binary | sidecar (bundled) | `nxf-relay` (standalone, self-hostable) | The relay ships in the same release: binary tarball + container image (GHCR) |

## 4. Versioning & tagging

### 4.1 One workspace version

All crates inherit the version from the workspace root:

```toml
# Cargo.toml (root)
[workspace.package]
version = "0.1.0"

# crates/*/Cargo.toml
[package]
version.workspace = true
```

This leaves **two** drift surfaces (root `Cargo.toml`, `Cargo.lock`) instead of four as in the
reference project. The CLI reports the version via `nxf --version` (clap, from `CARGO_PKG_VERSION`)
— it is automatically consistent.

### 4.2 Tooling: `cargo xtask` instead of Node scripts

The reference project uses Node scripts (`scripts/*.mjs`). nexus-flow is a pure
Rust repo; the version/changelog tools are implemented as an **`xtask` crate**
(member `xtask/`, convention alias in `.cargo/config.toml`), so that no second toolchain enters
the core workflow (§14 TB-7):

```bash
cargo xtask version set 0.2.0    # writes the workspace version + Cargo.lock, validates plain SemVer,
                                 # consumes changes/ fragments into release-notes.json (beta ring)
cargo xtask version check        # files consistent? (CI: + tag == version on v* tags)
cargo xtask changelog check origin/main   # PR gate: fragment present + well-formed (EN+DE)
                                          # + explicit `facade:` verdict when a consumed surface is touched
cargo xtask changelog show <v> <channel> <lang>      # render the notes for a version
cargo xtask changelog aggregate <prev> <v> <lang>    # stable aggregate since the last stable
cargo xtask changelog prev-stable <v>                # predecessor stable from the feed
cargo xtask changelog promote <v>                    # lift a version into the stable ring of the feed
cargo xtask facade semver-check  # public-API SemVer gate over all consumed surfaces vs the baseline tag (aye.14)
cargo xtask facade baseline      # print the resolved baseline tag + bump axis (documented base)
```

Semantics, fragment format, and feed shape are **carried over 1:1 from the reference project**
(`scripts/changelog.mjs`, `scripts/version-files.mjs`, `changes/README.md` serve as an
implementation template, including test cases):

- Fragment: `changes/<slug>.md` with frontmatter `type: added|changed|fixed|removed`,
  body sections `[en]` / `[de]`, both required. Malformed ⇒ loud failure. (`changes/README.md`
  documents the full format and is excluded from the fragment scan.)
- **Facade-contract marker.** A fragment MAY carry the frontmatter flags
  `facade: none|changed|breaking` (the impact on the consumed library surfaces, §4.3/§4.4) and
  `security: true` — and MUST carry the `facade:` one in the case below. Both are orthogonal to
  `type`. `cargo xtask changelog`
  aggregates flagged items into a dedicated, machine-readable **Facade Contract** /
  **Facade-Kontrakt** section (EN+DE) with the `security` marker visible. By §4.3 a
  `facade: breaking` change may ship only in a minor: `cargo xtask version set` fail-closes a
  **patch** bump that carries one — the changelog-side mirror of the `cargo-semver-checks` gate
  (§4.3).
- **The marker is MANDATORY when the diff touches a consumed surface** (added 2026-08-09,
  `6j6v.gmjd`): if a PR touches `crates/facade/`, `crates/chat/`, `crates/memory/` or the
  substrate they all run on, `crates/foundation/`, then every fragment that PR adds or modifies
  must state an explicit `facade:` value, and `changelog check` is fail-closed on it. Touching
  includes DELETING a file from those trees or moving one out — removing public API is a contract
  change, and on a minor bump `cargo-semver-checks` is report-only, so nothing else would say so.
  Rationale: a BEHAVIOURAL
  break behind unchanged signatures is structurally invisible to `cargo-semver-checks`, and the
  axis gate only fires on a marker that is present — so an omitted marker used to be
  indistinguishable from "checked, no impact", and twice on 2026-08-09 (PRs #311/#314) a break at
  26 public write entry points passed both gates green. The rule is process assurance, not a
  detector: it guarantees the question is asked, not that the answer is right. `skip-changelog`
  bypasses it along with the rest of the gate — a label is a human act on the record.
  `facade: none` is an assertion about the PR, not the release: it satisfies the gate and is
  dropped at fold time, so the feed's `facade` field stays `changed|breaking` for consumers that
  parse it. Both flags are omitted from the feed when unset (old entries unaffected).
- `release-notes.json` (repo root): `channels.{alpha,beta,stable}[]` with
  `{version, date, items[], notes:{en,de}}`; an item additionally carries optional
  `facade`/`security` fields when flagged; beta carries incremental entries, stable aggregates
  since the last stable; a channel with no entries inherits from the more stable ring.
- **Stable = net-diff, not union (`unreleased`).** The stable rollup is the net difference between
  two stable states, not a mechanical union of every beta fragment in the window. A fragment MAY
  carry `unreleased: true` (orthogonal to `type`/`facade`/`security`, default off) for a change that
  only ever mattered inside the beta ring — a fix/revert of something introduced *and* resolved
  between two stable promotes, which a stable user never saw. `cargo xtask changelog aggregate`
  omits `unreleased` items from the stable rollup; the beta ring keeps them (append-only truth is
  never touched). The promoter may set the flag retroactively on a beta feed item when intra-window
  churn only surfaces at promote time. Old feeds round-trip: the field is absent/`false` and omitted
  from output. See `changes/README.md`.
- Headings: EN `Added/Changed/Fixed/Removed`, DE `Neu/Geändert/Behoben/Entfernt`; facade callout
  EN `Facade Contract`, DE `Facade-Kontrakt`.

### 4.3 Rules

- **What requires a fragment (scope):** user-visible `nxf` CLI changes **and**
  substantial engine/library capabilities of the product (e.g., the E5 embedding host API, the
  `nexus-flow-facade` crate). The measure is *product impact*, not "touches the CLI". Purely
  internal changes without product impact — landing site (`site/**`), infra/CI, docs, internal
  refactors — carry no fragment. When every path such a PR touches is one that never ships (the
  `NO_PRODUCT_IMPACT` allowlist in `xtask/src/changelog.rs`), the gate passes by itself, so an
  outside contributor needs no maintainer (6j6v.c37e); otherwise a maintainer applies the
  `skip-changelog` label rather than a faked fragment being written. (Clarified
  2026-06-18: the rule of thumb was previously CLI-only; the finished E5 surface showed that
  deliberately shipped engine capabilities also belong in the feed — otherwise an entire
  epic vanishes without a trace.)
- The version is **plain SemVer** `X.Y.Z`. Suffixes (`-beta.1`) are rejected by the tooling —
  the channel is a ring, not part of the version.
- Release = annotated tag `v<version>` on a `main` commit. The tag is the truth;
  `version-check` (CI) enforces tag==version fail-closed (a malformed tag ⇒ error, not skip).
- Pre-1.0: MINOR may break (the usual 0.x convention); from 1.0 on, strict SemVer. The
  CLI `--json` contract (the agent-ergonomics measuring stick) counts as an API in the SemVer
  sense from 1.0 on.
- **The public Rust API of the consumed library surfaces is a SemVer contract** (added
  2026-06-25, §14 TB-15). This holds **in addition to** the CLI `--json` contract above,
  not instead of it — these crates are consumed as source via a git-tag pin
  (`publish = false` / TB-8), so their `pub` surface is itself a shipped API:
  - **The surfaces are `nexus-flow-facade` (`crates/facade/`), `nexus-chat` (`crates/chat/`) and
    `nexus-memory` (`crates/memory/`)** — all three pinned in lockstep by app-foundations and
    manufakt-io (widened 2026-08-09, `6j6v.a7ce`; the contract had always been meant for the
    consumed seam, but only `crates/facade` was guarded, so on 2026-08-03 a downstream compiler
    found `nexus_chat::…::WorkflowStartRequest` had gained fields in v0.41.0 — a break the release
    notes never mentioned).
  - **Patch `x.y.Z` never breaks these APIs** — no removed/renamed/signature-changed
    `pub` items, no behavioral contract break; a patch carries only bugfixes and security fixes.
    This protection applies **now, in 0.x** (it is the patch axis, not the 1.0 line).
  - **The break axis is Minor `x.Y.0`** (the usual 0.x convention; from 1.0 on, strict SemVer —
    MAJOR becomes the break axis and MINOR becomes additive).
  - **Consequence:** a patch is, by construction, fast-track-adoptable **without** a re-review of
    the API surface. This is the guarantee that lets an embedding consumer (manufakt) pull
    security patches automatically on the patch axis (auto-adopt §4.4, backports §4.5). The
    promise is enforced mechanically by the `cargo-semver-checks` gate over all three packages and
    surfaced in the feed by the facade changelog marker.
  - **What the machine cannot see, the marker must carry.** `cargo-semver-checks` diffs the API
    *shape*; a behavioural break behind unchanged signatures is invisible to it. That half of the
    contract is held by the mandatory `facade:` verdict on every PR touching a consumed surface
    (§4.2, `6j6v.gmjd`) — asked of a human, because no tool of that kind can answer it.
  - **Input request structs stay externally constructible** (decided 2026-08-09, `6j6v.a7ce`).
    `#[non_exhaustive]` would make an added field non-breaking, but on a struct a consumer
    *constructs* it forbids struct-literal construction outright — an immediate break for every
    consumer today, in exchange for a builder API we do not otherwise want. So an added field on
    e.g. `WorkflowStartRequest` remains a break, and the gate is what catches it: replaying
    v0.41.0 as a patch, `cargo-semver-checks` fails with `constructible_struct_adds_field` naming
    `WorkflowStartRequest.milestone`/`.tickets`. Output/receipt types are the opposite case and
    are `#[non_exhaustive]` where a consumer only reads them.

### 4.4 Consumer contract — crate pinners

§5–§7 describe the **binary** channel (tarball, `install.sh`, `self-update`). Embedding
consumers take a third path: they pin `nexus-flow-facade` **as source via a git tag**.
manufakt.io (our most important consumer) embeds the facade and is moving from a path-dep to
exact tag pins. The contract for that path (added 2026-06-25; §14 TB-16):

- **The git-tag pin is the official, supported consumption mode for embedded crates.**
  crates.io is deliberately out (`publish = false` / §14 TB-8), so the supported way to depend on
  `nexus-flow-facade` is a git dependency pinned to an **exact tag** (`tag = "v0.7.1"`), never a
  moving ref (a branch, or a default that drifts). Exact tags = reproducible builds; the SemVer
  contract of §4.3 only means something against a fixed point.
- **"stable" is machine-distinguishable — but not from the tag name.** Tags are plain SemVer
  `vX.Y.Z` with no suffix (§4.3); the tag name carries **no** stability signal. Stability lives
  in the GitHub release metadata: **stable = `prerelease == false` && `latest`**. A tag whose
  release is still `prerelease == true` sits in the **beta ring**. Consumers read the ring from
  the **GitHub Releases API**, not from the tag string.
- **Promotion beta→stable moves no tag.** beta→stable is a metadata + S3 server-side copy on the
  *same bytes* (§5.5); it neither creates nor moves a git tag. A tag pinner therefore always gets
  **identical bytes** regardless of ring — what flips is only the ring marker
  (`prerelease`/`latest`) exposed via the Releases API. Corollary: pinning a tag is safe even
  before promotion; the bytes never change under you, only the marker does.
- **Auto-adopt rule.** A consumer that wants to track our releases polls the Releases API and
  adopts on **two lanes** (full policy in §4.5):
  - **Patch lane (no human):** the highest stable `x.Y.z` **within the pinned minor `x.Y`**
    (`prerelease == false`, on the maintenance line of the pin). By §4.3 these are
    facade-API-stable, so they are safe to adopt unattended; this is the channel for security
    patches (the `security` flag). **This lane goes dry when the pinned line
    is retired** — once `x.Y` falls outside the N-2 window and its ~6-week grace lapses (§4.5),
    no further patches land on it; a consumer must then move to the minor lane.
  - **Minor lane (human-gated):** moving the pin `x.Y` → `x.Y+1` (the "highest `latest &&
    !prerelease`" step) is a deliberate adoption, because a minor is the break axis (§4.3). It is
    also the **forced** move once the pinned line is retired (§4.5).
- **SemVer guarantees (confirmation of the manufakt ask).** Patch = bugfix/security only, never
  an API break; Minor = the break axis in 0.x (strict SemVer from 1.0). A `facade: breaking`
  changelog marker can therefore appear **only** in a minor — the re-review
  trigger coincides with the minor lane by construction.

### 4.5 Security backports & maintenance lines

The auto-adopt **patch lane** (§4.4) only works if security fixes actually land as patches *on
the pinned minor line*. Today's model is fix-forward on a single line (tags on `main`, "stable
skips the broken version", §10/§11) with **no** maintenance branches; backporting is a new
capability. The policy is fixed here (added 2026-06-25; §14 TB-17); the
engineering that delivers it — `release/x.Y` branches, per-line patch tags, derivation-parity per
line — is:

- **Reach: N-2.** Security patches are backported into the **current + 2 previous minor lines**
  (up to 3 active lines at once).
- **Grace window: ~6 weeks, calendar-based** (decided 2026-06-25). A previous minor line stays
  security-supported for ~6 weeks after its successor minor is promoted to stable, then it is
  retired. Deliberately calendar- rather than cycle-based (simple to communicate); **no**
  open-ended LTS in 0.x.
- **Mechanic.** A security backport ships as a **patch `x.Y.z+1` on its line** — by §4.3
  guaranteed facade-API-stable, and flagged `security` in the feed. Each
  maintained line is a `release/x.Y` branch; the operational mechanics (branch-cut timing,
  per-line tag → beta→stable, per-line derivation-parity gate, feed forward-port, grace-window
  retirement) are the **backport runbook**, `docs/specs/maintenance-and-backports.md`.
- **Two-lane auto-adopt** (the consumer side, mirrored in §4.4):
  - **Patch lane (no human):** highest stable `x.Y.z` **within the pinned minor `x.Y`**. Works
    *only* because of this backport policy — without backports, a vulnerability would force a
    (potentially breaking) minor jump and the fast-track lane would collapse.
  - **Minor lane (human):** pinned `x.Y` → `x.Y+1`, planned.
  - The original single rule "highest `latest && !prerelease`" also pulls breaking minors — it is
    the *minor* lane only, never the security lane.

## 5. Build & release pipeline (`release.yml`)

### 5.1 Triggers & targets (as in the reference)

- **Tag push `v*` → PROD** (`https://nxsflow.com/nxs`): build once, upload to the prod bucket,
  write `manifests/beta.json`, then cut the GitHub **pre-release** (beta ring).
- **`workflow_dispatch` → STAGING** (`https://staging.nxsflow.com/nxs`): identical run against
  the staging bucket, **no** GitHub release. Pipeline rehearsal for changes to the
  release tooling itself.

**One chain, publish before announce** (teardown `6j6v.hy2f`): the dual-publish window of the
nxsflow.com migration is closed — the old `nxf-<env>` chain is deleted, and the run publishes
only to the `nxs-<env>` origin. Because there is no second copy to fall back on, the S3 publish
and its `head-object` sidecar check now run **before** the GitHub pre-release, so a release can
never announce bytes the updater cannot serve.

**Both paths pass the same `ci-green` gate** (6j6v.31rs): the run demands a green `ci.yml` run for
the commit it is about, from a `push` or `workflow_dispatch` event, and never tests the tree itself.
For a tag on `main` that run exists already. For a **rehearsal on a feature branch it does not** —
a branch gets only `pull_request` runs, which skip the release profile and therefore do not count.
So a rehearsal is two steps now:

```bash
gh workflow run ci.yml      --ref <branch>   # the full both-profile gate on that branch
# …wait for it to go green…
gh workflow run release.yml --ref <branch>   # the rehearsal itself
```

(Runbook form, with the reproduced run numbers: §10.1.)

Deliberate, not an oversight: a rehearsal that publishes to the staging bucket from a tree whose
gate has not passed is the same mistake as doing it in prod, one bucket over. The alternative —
exempting staging — would leave the gate untested on the only path anyone exercises on purpose.

### 5.2 Build matrix

| Target | Runner | Linkage | Artifact |
|---|---|---|---|
| `aarch64-apple-darwin` | `macos-14` | default | `nxf_<v>_darwin-aarch64.tar.gz` |
| `x86_64-apple-darwin` | `macos-14` (cross) | default | `nxf_<v>_darwin-x86_64.tar.gz` |
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | **static (musl)** | `nxf_<v>_linux-x86_64.tar.gz` |
| `aarch64-unknown-linux-musl` | `ubuntu-latest` (cross) | **static (musl)** | `nxf_<v>_linux-aarch64.tar.gz` |

- One tarball per target with two members: `nxs` (the multicall binary — `nxf`/`nxm`/`nxc` are
  symlinks the installer creates) and `nxf-relay` (+ LICENSE, README).
  One artifact per platform keeps the manifest, install script, and promotion simple; the relay is
  small (§14 TB-9). rusqlite is `bundled`, Linux musl ⇒ one static binary, no
  glibc dependency — the "single static binary" adoption advantage from the E0 spike is
  preserved.
- **Which relay backends the artifact carries: `sqlite` + `postgres`, not `dynamodb`**
  (nexus-flow-6j6v.7nee, §14 TB-18). The relay's durable backends are cargo features, so this is
  a delivery decision, not a runtime one — a build without the feature answers
  `NXF_RELAY_BACKEND=…` with the unknown-backend panic no matter what the operator sets. The
  release job therefore builds `--features nxs-server/postgres`, and says so with the measurements
  in a comment beside the build step. Measured cost of the relay build, cold:

  | backends | crates | build | binary |
  |---|---|---|---|
  | `sqlite` only | 76 | 57 s | 3.8 MB |
  | `+ postgres` | 153 | 71 s | 5.0 MB |
  | `+ dynamodb` | 247 | 3 min 35 s | 22.6 MB |

  Postgres is close to free and is what makes *"sync to a Postgres, e.g. Supabase"* true of the
  artifact people actually download — a managed database is the realistic answer for someone who
  wants their board backed up off their laptop or shared with a colleague. DynamoDB pulls
  `aws-lc-sys` (native C/assembly), which needs `cmake` and a C toolchain on all four release
  runners — including the two musl legs that cross-compile through zig — to add ~19 MB to every
  tarball for a capability nobody consumes from a tarball: the one deployment using it
  (`manufakt-io`) builds the relay from a pinned source revision with its own toolchain and never
  unpacks ours. **DynamoDB is a supported source build, not a shipped capability**, and that is
  documented rather than merely omitted — in the user guide (`running-a-relay`), in the release
  notes, and in the Cargo feature comment.
- **The relay ships without client authentication**, on any backend. Choosing a managed database
  changes where the data rests, not who may read or write a stream: anyone who can reach the relay
  endpoint can do both (see §5.6 and the guide's warning). Authentication is `xsf3` (the decision)
  and `6aza` (the E4 slice); nothing in the delivery may imply it is already solved.
- **The `nxc` agent sidecar is inside `nxs`, not beside it** (nexus-flow-6j6v.smsz). It is a single
  self-contained ESM bundle built by the release job *before* the Rust build (`agent-sidecar`:
  `npm ci && npm run build`, esbuild) — ~1.4 MB, no `node_modules` — and `crates/chat/build.rs`
  compiles it into the binary, which unpacks it into `<cache>/sidecar/<content-hash>/` on first use.
  It is JavaScript, so it needs no signing of its own and is identical across platforms; embedded,
  it rides the binary's own sha256 + minisign gate.

  It was a **third tarball member** in 0.52.0/0.53.0, placed beside the binary by `install.sh` and
  `nxs self-update`. That route has a bootstrap hole no version can close from the inside: the code
  that places the file lives in the version being installed, and an update is always performed by
  the version being replaced. Every machine coming from ≤0.51.0 therefore landed without a sidecar
  — the normal case, not the exception. With the bundle embedded there is no second file to place,
  to forget, or to let drift out of step with the binary (which also settles nexus-flow-6j6v.5vct).
  Both installers still *place* the member when `NXF_VERSION` pins 0.52.0/0.53.0, whose binaries can
  only find one beside themselves, and both **remove** a superseded copy when the release carries
  none. `NXF_EMBED_SIDECAR=require` on the release build makes an absent bundle a failed build
  rather than a binary that installs cleanly and cannot start an agent.
- **No** macOS lipo/universal: unlike the desktop app (one signed artifact for
  both slices), with a CLI the install script/self-update picks the target — two lean
  binaries beat one fat one (§14 TB-5).
- Matrix build (4 jobs) + one `publish` job that collects the artifacts. This breaks with the
  single-job model of the reference but is forced by the platform variety; the
  build-once-promote principle holds unchanged per artifact.

### 5.3 Signing

- **Per artifact**: `*.tar.gz` + `*.tar.gz.sha256` + `*.tar.gz.minisig`.
- **minisign** key pair analogous to the Tauri updater key of the reference: the public key is
  **compiled into** the `nxf` binary (verification at `self-update`), the private key + password
  exclusively as GitHub secrets (`NXF_MINISIGN_PRIVATE_KEY`, `NXF_MINISIGN_KEY_PASSWORD`)
  + an offline backup in the password manager. **Loss = no update path** (installed binaries
  accept only the compiled-in key) — the backup is critical, `.gitignore` blocks
  `*.key`/`*.pub` as belt-and-suspenders. Rotation = a new release with a new pubkey; old
  installations must catch up via the install script (document in the runbook).
  - **Realized verification mechanism (85y.5).** "Compiled in" = the release build
    (85y.13) sets `NXF_MINISIGN_PUBKEY` (base64 line of the pubkey); `crates/cli/src/signature.rs`
    reads it via `option_env!` (`build.rs` forces a rebuild on change). Verification:
    `minisign-verify` (pure Rust, keeps the musl static binary), only **prehashed** signatures
    (`allow_legacy=false`). **Fail-closed**: a build without a key refuses verification
    instead of accepting. Deliberately **no** committed pubkey (env var instead of a `*.pub`
    file); the private key is never built in — until the operator generates the key pair (§12.4),
    `release.yml` signs nothing and installed builds verify fail-closed. Runtime surface:
    `nxf verify-signature --file <f> --signature <f.minisig>` (hidden), used by
    `install.sh`/`self-update` (85y.18).
- **macOS codesigning + notarization (realized; §14 TB-6 revised).**
  With an active Apple Developer membership, `release.yml` signs the two darwin binaries
  (`nxf`, `nxf-relay`) on the `macos-14` build runner with the **Developer ID Application**
  identity (`codesign --force --timestamp --options runtime` — Hardened Runtime + Secure
  Timestamp are notarization prerequisites) and submits them via
  `xcrun notarytool submit --wait` (App Store Connect **API key**, no app-specific
  passwords) to Apple.
  - **No stapling, no format change (design decision 85y.30).** A bare CLI Mach-O
    is not `stapler`-staplable (only `.pkg`/`.dmg`/`.app` containers are). Rather than switching to
    a `.pkg` distribution format (Developer ID **Installer** cert, install.sh/Homebrew/
    promote rework, away from the build-once-promote tarball), we sign+notarize the binaries
    and rely on Gatekeeper's **online** check: notarization binds to the
    cdhash on Apple's servers, **the same** signed binaries move unchanged into the
    existing `tar.gz`. The ZIP that `notarytool` needs is purely a submission vehicle and
    is never shipped. Channel effect: `curl|sh` sets no `com.apple.quarantine` ⇒
    Gatekeeper never engages there; only quarantined channels (Homebrew Cask 85y.22,
    browser download) trigger the online check and need a network connection at first launch.
  - **Toggleable + fail-closed** (pattern like `NXF_MINISIGN_PUBKEY` / `NXF_PROD_RELEASE_ENABLED`):
    signing runs only with `vars.NXF_MACOS_SIGNING_ENABLED == 'true'`. If it is on, every
    Apple secret **must** be present, otherwise the job aborts (no silently-unsigned release); if
    it is off (contributors / before bootstrap), darwin jobs build unsigned with a notice. **Prod
    is additionally fail-closed on signing**: the `publish` job refuses a tag release (exit
    1) if `NXF_MACOS_SIGNING_ENABLED != 'true'` — a forgotten/mistyped variable thus cannot
    ship unsigned macOS binaries (the `enabled` gate for signing and the
    `NXF_PROD_RELEASE_ENABLED` gate for publishing are otherwise orthogonal). Secrets:
    `APPLE_DEVELOPER_ID_APP_CERT_P12_BASE64`, `APPLE_DEVELOPER_ID_APP_CERT_PASSWORD`,
    `APPLE_NOTARY_KEY_P8_BASE64`, `APPLE_NOTARY_KEY_ID`, `APPLE_NOTARY_ISSUER_ID`. **`APPLE_TEAM_ID`
    is NOT required by API-key notarization** (only key/key-id/issuer) — optional/reserved,
    not consumed in the pipeline path. The cert + `.p8` are imported into a throwaway keychain; a
    `trap` on `EXIT INT TERM` shreds the cert, `.p8`, keychain, and the notarization ZIP on
    every exit **including signal** (important on reused self-hosted runners, where
    `$RUNNER_TEMP` is not wiped between jobs), plus a pre-flight purge of any leftovers from
    a hard-aborted prior run; the keychain password is masked via `::add-mask::`.
  - **Verification (acceptance):** two hard gates — `codesign --verify --deep --strict` per
    binary **and** the `notarytool` verdict `Accepted` (from then on the cdhash is registered
    with Apple; exactly what Gatekeeper checks online when a user first opens a quarantined
    copy). `spctl --assess --type exec` is only **informative** and **not** a gate: the
    exec assessment is bundle-oriented and reports "rejected" (exit 3) for a correctly
    signed+notarized *bare* Mach-O — stapling would make it deterministic, but it is
    impossible for a bare CLI binary (design §5.3, confirmed in the staging rehearsal 2026-06-15).
    minisign/openssl (85y.28) + out-of-band pubkey (#vdi) remain complementary — they secure
    integrity/authenticity of the tarball, notarization the OS-native first-launch anchor.
  - **Runner cost / self-hosted (optional).** The two darwin jobs run by default on
    GitHub-hosted `macos-14` — expensive (~10× Linux minutes), and `notarytool --wait` lets the
    runner idle for minutes while billing during Apple notarization. The repo variable
    `NXF_MACOS_RUNS_ON` (JSON label array, e.g. `["self-hosted","macOS","X64"]`) routes the
    darwin jobs to a **dedicated Mac** (idle time then free); empty ⇒ `macos-14` fallback.
    Host architecture does not matter — both jobs build with an explicit `--target` (Intel runner: `x86_64`
    native + `aarch64-apple-darwin` cross; Apple Silicon the other way around), `codesign`/`notarytool` are
    arch-independent. Runner prerequisites: Xcode Command Line Tools (`codesign`, `notarytool`,
    `spctl`, `ditto`); the workflow installs Rust + targets. **Security:** `release.yml`
    triggers only on tag push / `workflow_dispatch` (write-access-gated) — foreign PR code never reaches
    the runner. Since `$RUNNER_TEMP` is **not** wiped between jobs on self-hosted runners,
    a **dedicated, single-tenant** setup is **required** (a dedicated runner in a
    runner group that serves only this repo); the signing step does shred its key
    material on every exit including signal + pre-flight, but a foreign job on the same
    machine remains the residual risk that single-tenancy eliminates.

### 5.4 Publish (adaptation of `publish-release.sh`)

`./.github/scripts/publish-release.sh` expects `VERSION, CHANNEL, ARTIFACT_BUCKET, DOMAIN`
(AWS creds already assumed via the OIDC release role) and writes:

```
s3://<bucket>/download/<channel>/<version>/nxf_<v>_<target>.tar.gz        (+ .sha256, + .minisig)
s3://<bucket>/manifests/<channel>.json
s3://<bucket>/release-notes.json          (Cache-Control: max-age=300)
s3://<bucket>/install.sh                  (Cache-Control: max-age=300; §7.1)
```

Channel manifest (extension of the reference shape by `sha256`; `path` stays S3 key ==
public URL path 1:1):

```json
{
  "version": "0.2.0",
  "pub_date": "2026-06-09T10:00:00Z",
  "notes": "### Added\n- …",
  "enabled": true,
  "rollout": 100,
  "platforms": {
    "darwin-aarch64":  { "path": "download/beta/0.2.0/nxf_0.2.0_darwin-aarch64.tar.gz",  "sha256": "…", "signature": "<minisig>" },
    "darwin-x86_64":   { "path": "download/beta/0.2.0/nxf_0.2.0_darwin-x86_64.tar.gz",   "sha256": "…", "signature": "<minisig>" },
    "linux-x86_64":    { "path": "download/beta/0.2.0/nxf_0.2.0_linux-x86_64.tar.gz",    "sha256": "…", "signature": "<minisig>" },
    "linux-aarch64":   { "path": "download/beta/0.2.0/nxf_0.2.0_linux-aarch64.tar.gz",   "sha256": "…", "signature": "<minisig>" }
  }
}
```

The `.minisig` sidecars **must** sit next to the tarballs in S3 — the later promotion
reads them from there (lesson from the reference: without the sidecar, beta→stable cannot
sign the stable manifest).

GitHub release body = `cargo xtask changelog show <v> beta en` (fallback: "Beta-ring
build."), plus a note about the install command and the promotion mechanism. All tarballs + sidecars
are additionally attached to the GitHub release (`gh release upload`) — GitHub is the
showcase, AWS the delivery.

### 5.5 Promotion (`promote.yml` + `promote-release.sh`, 1:1 adaptation)

Trigger: the GitHub event `release: released` (the pre-release is flipped to "latest" — the
only way the event can fire, because tag builds are always pre-releases). The defensive
guard on `prerelease == false && draft == false` remains.

Flow (identical to the reference, only n platforms instead of 2-into-1):

1. `head-object` guards: **all** platform tarballs **and** `.minisig` sidecars must exist in
   the beta ring (distinguish 404 vs. 403!).
2. Server-side copy `download/beta/<v>/…` → `download/stable/<v>/…` (bytes & signatures
   unchanged).
3. Aggregate stable notes: `prev-stable` from the **committed feed** (not from the
   S3 manifest — the reference learned that it can come back empty),
   `aggregate <prev> <v>`. The output is a **curated draft, not final copy**: `unreleased`-flagged
   items are already filtered out mechanically (§4.2); the promoter then strikes any remaining
   intra-window introduce-then-fix churn — **prefer** flagging it `unreleased` over hand-editing,
   since flags survive a re-promote while hand-edits to the rendered `notes.{en,de}` do not —
   condenses multiple beta refinements of one feature into a single stable line, and phrases for a
   stable audience before publishing.
4. Rewrite `manifests/stable.json`, promote the feed + publish to S3.
5. Commit the regenerated feed **best-effort** back to `main` (no `[skip ci]` — the
   push triggers exactly the site deploy that updates the changelog; loop-free, because the
   release runs only on tags). A branch-protection reject must not fail the promotion.

### 5.6 Container image for `nxf-relay` — **specified, deliberately not built yet**

**Status (2026-08-07, nexus-flow-6j6v.7nee): no container image is published.** There is no
Dockerfile in the repo and no GHCR step in any workflow. This paragraph described one from the
day it was written, which made the spec a standing promise nobody was keeping — the gap is now
named here instead of discovered again.

The intended shape, unchanged, for when it is built: in the same tag build, `docker build`
(distroless/static, the musl binary) → push to **GHCR** `ghcr.io/nxsflow/nxf-relay:<version>` +
`:beta`; promotion retags `:latest` + `:stable` (crane/skopeo, no rebuild — the same
build-once-promote principle for images, §14 TB-9). GHCR instead of ECR: public, free, next to
the source (GitHub = showcase); AWS remains delivery for the **CLI artifacts**.

**Why it is not built yet, and what unblocks it.** The image is the right answer to the question
a user actually has — *where do I run this thing?* — and `docker run` is the shortest path from
"I use the CLI" to "my board is backed up and my colleague can see it". But the relay has **no
client authentication**: whoever reaches the endpoint reads and writes the stream. Publishing a
one-command way to stand a relay up would hand people a one-command way to put an unauthenticated
shared board on the internet, and the friendliness of the packaging is exactly what would make
that happen. The image therefore waits on authentication (`xsf3` → `6aza`) and is tracked as
`6j6v.v99p`, which depends on it — a ticket with an edge, not an intention in a paragraph. Until
then the supported way to run a relay is the binary in the release tarball (see the user guide,
`running-a-relay`).

## 6. Updater endpoint & delivery (AWS)

### 6.1 Lambda contract (was `infra/lambda/updater/`; since `6j6v.sgfr` in `nxsflow-landing-page`)

One Lambda Function URL behind CloudFront, three paths:

- **`/updater?channel=&target=&arch=&current_version=`**
  - `204` — up to date / no build / `enabled:false` / rollout exclusion.
  - `200` + `{ version, pub_date, notes, url, sha256, signature }` — update available;
    `url` = `https://<domain>/<path>` (CloudFront).
  - The gating logic (kill switch, `rollout` 0..100, target/arch, strictly-newer per SemVer) stays
    **pure + unit-tested** (the `decide.mjs` pattern); S3 I/O in the handler, the decision beside it.
  - Rollout determinism: the client additionally sends `machine_id` (hashed
    replica/install ID); the Lambda hashes to 0..100 — a client does not flap between
    "update available/not available" (reference behavior, carried over to the CLI).
- **`/latest?channel=&target=&arch=`** — `302` to the newest channel tarball (for the
  install script & download buttons; defaults: `stable`, detected target/arch), `404`+HTML
  if nothing is published.
- **`/install.sh`** — is **not** served by the Lambda but as an object from the
  artifacts bucket (a dedicated CloudFront behavior, short TTL), published by the release run
  (§5.4). This keeps the script version-free and the Lambda slim.

### 6.2 CDK infrastructure (was `infra/`; deleted by `6j6v.sgfr`)

> **⚠️ Superseded (2026-07-14) and TORN DOWN (2026-08-08) — see
> `docs/specs/nxsflow-com-migration.md` §5 (A1/A4/A7) and §6 step 9.**
> The delivery infra was re-homed to a **domain-less** shape in the manufaktio accounts:
> the stacks/roles described below (`NxfDistribution`/`NxfCert`, the site bucket, in-account
> CloudFront + Route 53 alias, `nxf-*` names) are replaced by `NxsDelivery`/`NxsGitHubDeploy`
> (`nxs-*` namespace; the landing distribution fronts the origins cross-account under `/nxs`).
> As of the teardown (`6j6v.hy2f`) the old stacks no longer exist: `NxfDistribution-<env>`,
> `NxfCert-<env>`, `NxfGitHubDeploy-<env>` and the `DelegatedSubdomain[Cert]-nxf[-staging]-…`
> stacks are deleted in both coding-agent accounts, together with the
> `nxf[-staging].nxsflow.com` hosted zones, CloudFront distributions and ACM certificates.
> Only the `nxf-<env>-artifacts-*` buckets survive (`RETAIN` removal policy; they hold the
> historical tarballs and go away with the account suspension, an owner step).
> **AND THEN THE WHOLE TREE LEFT (2026-08-23, `6j6v.sgfr`.)** `infra/` is deleted from this
> repo — the delivery stack, the deploy/OIDC stacks, the CI gate logic, the E2E suite and the
> updater Lambda. `infra/README.md`, named as authoritative below, is deleted with it. The
> delivery stack now lives in `nxsflow-landing-page` and the `nxs-<env>-artifacts-*` buckets in
> the nxsflowcom accounts. This section is kept for historical context only.

Same stack architecture, renamed; **shared nxsflow AWS accounts** (staging/prod,
with other products — §14 TB-10, revised):

| Stack | Region | Deploy |
|---|---|---|
| `NxfGitHubDeploy-<env>` | eu-central-1 | **manually, once per account** (trust anchor: import OIDC provider, infra-deploy role) |
| `NxfCert-<env>` | us-east-1 | workflow (CloudFront ACM requirement) |
| `NxfDistribution-<env>` | eu-central-1 | workflow |

`NxfDistribution-<env>` creates (all 1:1 like `distribution-stack.ts`, names adjusted):

- **Artifacts bucket** `nxf-<env>-artifacts-<account>` — private (OAC), versioned,
  `RETAIN`; carries `download/…`, private `manifests/…` (no CloudFront behavior, only the
  Lambda reads them), `release-notes.json`, `install.sh`.
- **Site bucket** `nxf-<env>-site-<account>` — private (OAC), `DESTROY`; the content belongs to the
  site workflow (no placeholder seed — the reference learned that it clobbers the live page).
- **CloudFront** with behaviors: default → site (+ directory-index function for clean
  `/de` URLs), `/download/*` → artifacts (long TTL, immutable keys),
  `/release-notes.json` + `/install.sh` → artifacts (object TTL 300s),
  `/updater*` + `/latest*` → Lambda (CACHING_DISABLED, `ALL_VIEWER_EXCEPT_HOST_HEADER`).
- **Route 53** A/AAAA alias to the delegated zone (apex of the subdomain zone).
- **Roles** (OIDC, least privilege):
  - `nxf-<env>-release` — write to `download/*`, `manifests/*`, `release-notes.json`,
    `install.sh`; **plus Read+List on `download/*`** (promotion! — the documented
    IAM bug of the reference); CloudFront invalidation. Prod subjects: **only `refs/tags/*`**;
    staging additionally branches (dispatch rehearsal).
  - `nxf-<env>-site-deploy` — only the site bucket (sync needs get/put/delete/list) +
    invalidation + `ListDistributions`.
  - `nxf-infra-deploy-<env>` (in the trust-anchor stack) — may only assume the CDK bootstrap
    roles + read the bootstrap version; SSM namespace `/nxf/infra/*` (deploy marker, lease);
    staging additionally the E2E manifest write rights.

### 6.3 Domains

> **⚠️ Historical.** The `nxf[-staging].nxsflow.com` subdomains and their delegated hosted
> zones were retired in the teardown (`6j6v.hy2f`); the names no longer resolve. The delivery
> accounts own no domain at all now (A1) — the public surface is the nxsflow-landing-page
> distribution. Rationale for the original choice in §14 TB-1.

| Env | Endpoint (today) | Purpose |
|---|---|---|
| prod | **`https://nxsflow.com/nxs`** | downloads, updater, install.sh (human pages: `/open-source`) |
| staging | `https://staging.nxsflow.com/nxs` | pipeline rehearsal + install/update smoke |

| Env | Domain (retired) | Purpose |
|---|---|---|
| prod | `nxf.nxsflow.com` | landing page, downloads, updater, install.sh |
| staging | `nxf-staging.nxsflow.com` | pipeline rehearsal |

## 7. Install & self-update (replaces the Tauri updater)

### 7.1 `install.sh`

```bash
curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

- Detects `uname -s/-m` → `target-arch`; downloads via `/latest?channel=${NXF_CHANNEL:-stable}&…`
  (302 → CloudFront tarball) + `.sha256` sidecar; **sha256 verification is mandatory**,
  minisign verification additionally if `minisign` is installed.
- Installs to `${NXF_INSTALL_DIR:-$HOME/.local/bin}` (no sudo default): the `nxs` binary, the
  `nxf`/`nxm`/`nxc` persona symlinks to it, and optionally (`NXF_INSTALL_RELAY=1`) `nxf-relay`;
  checks PATH and otherwise prints the shell-profile line. No agent sidecar is placed — it lives
  inside `nxs` (§5.2) — and a copy left there by 0.52.0/0.53.0 is removed.
- Idempotent; `NXF_VERSION=0.2.0` pins a version (direct `download/<channel>/<v>/` path).

### 7.2 `nxs self-update`

A CLI subcommand. Canonical form is the umbrella `nxs self-update`; `nxf
self-update` survives as a hidden, deprecated alias (same code path, suite-wide swap) so existing
muscle memory and scripts keep working:

- `nxs self-update [--channel stable|beta|alpha] [--check] [--json]`.
- Queries `/updater` with `channel` (persisted in the user config, default `stable`),
  `target`, `arch`, `current_version`, `machine_id`. `204` ⇒ "up to date".
- Downloads the tarball, verifies **sha256 + minisign against the compiled-in pubkey**
  (no installation without a valid signature — exactly the Tauri updater guarantee), replaces its
  own binary atomically (write-to-temp + rename; the Windows special case only in Phase 2).
- No downgrade (strictly-newer per SemVer) — a ring switch to an older state keeps the
  installed version until the ring catches up (reference behavior).
- **No background auto-update.** An agent CLI must be deterministic; a binary
  that swaps itself out between two calls is not. Instead: a discreet
  update hint on stderr (never into the `--json` stdout!), throttled to 1×/day via a cache file,
  disableable (`NXF_NO_UPDATE_CHECK=1`, CI detection) (§14 TB-4).
- Implementation: a thin home-grown build against our manifest endpoint (the `self_update` crate
  is tailored to GitHub-release backends and would not carry the endpoint contract;
  §14 TB-3).

### 7.3 Package managers (Phase 2/3)

Homebrew tap (`nxsflow/homebrew-tap`) as the first follow-up channel — formula update as a step in the
promote workflow (the stable promotion updates the tap). cargo-install/AUR/nix: community,
not pipeline-mandatory. (§14 TB-2: deliberately **after** our own install path, not instead of it.)

## 8. Landing page (`site/`)

Blueprint = `beads-dashboard/site/` (React 19 · Tailwind v4 · Lingui · statically
prerendered, EN + `/de`, no router — the reasons documented there hold unchanged).

- **Content:** hero with the install command (copy button) instead of a download button; the pitch from the
  product vision (agent-native, offline-first, convergent — "gives every AI assistant a
  reliable memory"); feature section along the core pillars (data model,
  derivation layer, CLI, sync); terminal demo (`nxf prime` / `ready` / `next`) instead of
  app screenshots; changelog section from `release-notes.json` (build-time import via a
  `sync-release-notes` equivalent); "powered by nexus-flow" explanation with a reference to
  manufakt.io / nexflow.it; GitHub link prominent.
- **Brand** (§14 TB-11): nexus-flow is the **foundation** of the product family → parent-brand
  design: white background, lime accent `#B9FF66` used sparingly, graphite `#1C1C1C`, Merriweather for
  headings, Merriweather Sans for body, JetBrains Mono for terminal/code; tone: confident,
  clear, understated — no hype. Badge/logo rules from the brand guideline (F-highlight
  optional, clear space 1×).
- **Deploy** as in the reference (`site.yml`): main push (path `site/**`, `release-notes.json`) →
  prod; staging via the gate label; `aws s3 sync` with immutable cache for `assets/`, 60s TTL for
  HTML; CloudFront invalidation; the staging build shows an env badge with the commit SHA.

## 9. Workflow inventory

| Workflow | Trigger | Function |
|---|---|---|
| `ci.yml` *(exists)* | PR, main, `release/<major>.<minor>`, dispatch | fmt, clippy `-D warnings`, tests debug **and** release + postgres-parity (the per-line derivation-parity gate, §4.5). The three `(release)` test steps are **staggered** (6j6v.6e0e): push/dispatch only, not on the PR event — they were ~54% of a run's machine time, and everything that ships is tagged off `main`/`release/**`, where they always run. Superseded **PR** runs are cancelled by a `concurrency` group; a push run on `main`/a maintenance line never is. The push filter is `release/[0-9]+.[0-9]+`, NOT `release/**` (6j6v.31rs): the wide glob also caught the `release/v<X.Y.Z>` preparation branches, so every version-bump PR ran this whole workflow twice on one tree (~20m37 of runner time on the PR event, ~44m57 on the branch push, measured on 61b33407). The macOS platform gate runs on the org's own Mac via `NXF_MACOS_RUNS_ON` — except for a FORK pull request, which stays GitHub-hosted. |
| `version-check.yml` | PR, main, `release/<major>.<minor>`, tags `v*` | `cargo xtask version check` — consistency + tag match, fail-closed (per line too) |
| `changelog-check.yml` | PR (+`labeled`/`unlabeled`) | fragment requirement, waived by itself when nothing the PR touches ships, opt-out `skip-changelog`; plus the explicit `facade:` verdict when the diff touches a consumed surface (§4.2, `6j6v.gmjd`) |
| `facade-semver.yml` | PR, main, `release/<major>.<minor>`, tags `v*` | `cargo xtask facade semver-check` — public-API SemVer gate over all three consumed surfaces, `nexus-flow-facade`/`nexus-chat`/`nexus-memory` (patch never breaks); fail-closed on a patch bump, report-only on a feature PR / minor; runs per line (§4.3) |
| `release.yml` | tag `v*` (prod) / dispatch (staging) | **`ci-green` gate** → matrix build, sign, GitHub pre-release, S3 publish, beta manifest, GHCR image. The gate DEMANDS a green `ci.yml` run for the tagged commit (`push`/`workflow_dispatch` only — a `pull_request` run skips the release profile) and never tests the tree itself; no green run ⇒ red release. It replaced a `test` job that re-ran `cargo test --all` and `cargo test --all --release` on a commit CI had just tested — 21m28 of a 31m52 release on v0.51.0 (6j6v.31rs). Script: `.github/scripts/require-green-ci.sh`, hermetically tested by `tests/require-green-ci.test.sh`. |
| `promote.yml` | `release: released` | beta→stable: S3 copy, stable manifest, notes aggregate, feed commit, image retag; **line-aware** — promotes from `main` or, for a backport, the release's `release/x.Y` line |

**SIX ROWS USED TO STAND HERE AND THE FILES ARE GONE.** `staging-gate.yml`, `infra-staging.yml`,
`infra-staging-release.yml`, `infra-prod.yml` and `infra-ci.yml` were deleted with the infra
teardown (`6j6v.sgfr`); `site.yml` went earlier with the site retirement (`6j6v.0a6p`, replaced by
`content-ci.yml` + `publish-content.yml`). They are listed by name rather than silently dropped
because a reader who met them in an older document needs to learn they are gone, not merely fail
to find them. What they did:

| Removed workflow | Did | Where it lives now |
|---|---|---|
| `staging-gate.yml` | auto-labelled `deploy-staging-infra`/`-site`, single-holder lease, trust gate | nothing — there is no shared staging environment in this repo to lease |
| `infra-staging.yml` | cdk deploy staging + E2E + SSM marker | `nxsflow-landing-page` |
| `infra-staging-release.yml` | released the SSM lease when a holding PR closed; **MOTHBALLED 2026-08-21** because its role stayed in the old account while the secret moved, so every closed PR minted a red run | nothing — the lease it freed no longer exists |
| `infra-prod.yml` | cdk deploy prod + black-box smoke | `nxsflow-landing-page` |
| `infra-ci.yml` | `tsc --noEmit` + `node --test` over `infra/**` on every PR | `nxsflow-landing-page` (for the code that moved there) |
| `site.yml` | site build + S3 sync + invalidation | `nxsflow-landing-page` |

The E2E suite (`infra/e2e/`) went with them. Black-box coverage of the live endpoints is now the
landing repo's scheduled endpoint prober (`cyb7.fv48`), which alarms per check and asserts
same-origin `url`/`Location` rather than a bare status code. What this repo still runs against a
live chain is `install-update-smoke.yml` — weekly, against staging, over the *installer*.

This table was never exhaustive and is less so now; `.github/workflows/` is the list.

## 10. Release runbook (short version)

1. `cargo xtask version set <v>` → bump + fragments→feed; commit
   (`chore(release): v<v>`), PR to `main`, `version-check` green, merge.
2. **Wait for CI on the merge commit to go green**, then
   `git tag -a v<v> -m "v<v>" && git push origin v<v>` → `release.yml`: matrix build,
   GitHub pre-release (beta ring), S3 `download/beta/<v>/`, `manifests/beta.json`.

   The wait is new and it is the whole point of 6j6v.31rs: `release.yml` no longer runs the test
   suite a second time, it READS the verdict of the `ci.yml` run for that exact commit. Tagging the
   instant the merge lands does not skip anything — the `ci-green` job simply waits for that run
   instead, and the release takes just as long as it used to. Tagging once CI is green is what
   turns the gate into a few seconds. Either way it is fail-closed: no green run for the commit,
   red release, never a fallback test run.
3. **Dogfood on beta**: `nxs self-update --channel beta` (we use nexus-flow ourselves —
   the engine eats its own tail, vision §Phases).
4. **Promote**: open the GitHub release, deselect "Set as pre-release", set "Set as latest"
   → `promote.yml` copies the same bytes to stable.
5. **Broken?** `enabled:false` into the affected channel manifest (kill switch, immediate), fix
   forward as `<v+1>`; stable simply skips the broken version.

For a **security backport to a previous minor line** (not fix-forward on the current line), follow
the backport runbook, `docs/specs/maintenance-and-backports.md` (§4.5 policy: N-2, grace ~6 weeks)
— `release/x.Y` branch → patch tag `x.Y.z+1` → beta→stable on the line, with the same gates per
line.

### 10.1 Rehearsing a change to the release chain itself (two dispatches)

Changing `release.yml`, the publish script or the signing steps? Rehearse it against **staging**
from the feature branch before merging — and note that this takes **two** dispatches, in this
order (6j6v.rvyp):

```bash
gh workflow run ci.yml      --ref <branch>   # 1. the full both-profile gate ON THAT BRANCH
# …wait for that run to go green (~20-25 min, both profiles) — step 2 reads its verdict…
gh workflow run release.yml --ref <branch>   # 2. the rehearsal itself → staging bucket, no release
```

Step 1 is not optional and not a formality: `release.yml`'s `ci-green` gate counts only `push` and
`workflow_dispatch` runs of `ci.yml` (a `pull_request` run skips the release-profile tests, §5.1),
while `ci.yml`'s own push filter is narrowed to `main` and `release/<major>.<minor>`. A feature
branch therefore produces *only* runs that do not count, and dispatching `release.yml` first fails
after a five-minute wait with `no green ci.yml run for commit <sha>`. Reproduced 2026-08-08: run
`31264672012` red, then `ci.yml` dispatched (`31264929960`, green), then `release.yml` again
(`31265539686`, fully green).

## 11. Rollback & kill switch

Unchanged from the reference: artifacts immutable + versioned, the manifest is the
moving part. Kill switch = `"enabled": false`; rollback = write the manifest back to a known-good
version (one `aws s3 cp`). `self-update` never downgrades — rollback protects
new installs/checks; already-affected users are healed by a higher fix release.

## 12. Bootstrap checklist (one-time)

> Steps 1–3 are superseded twice over and are kept only for the historical record. First by the
> domain-less bootstrap that used to be documented in `infra/README.md` (eu-central-1 only, no
> us-east-1 cert region, no hosted zone, trust anchor `NxsGitHubDeploy-<env>`); then by
> `6j6v.sgfr`, which deleted `infra/` and that README along with it. **The AWS bootstrap for the
> delivery chain is not this repo's job any more** — it belongs to `nxsflow-landing-page`.

1. Use the shared nxsflow accounts (TB-10 revised): staging
   `manufaktio-staging`, prod `manufaktio-prod`; check/complete the CDK bootstrap in
   eu-central-1 **and** us-east-1 (staging: done).
2. Create hosted zones `nxf.nxsflow.com` / `nxf-staging.nxsflow.com` and delegate them from
   `nxsflow.com`.
3. GitHub OIDC provider per account (if not present), then manually
   deploy `NxfGitHubDeploy-<env>` (trust anchor; a workflow cannot create the role it
   assumes itself).
4. Generate the minisign key pair; compile in the pubkey, store the private key + password as
   GitHub secrets + an offline backup (set the password via `printf` without a newline — a reference trap).
5. Create the label `skip-changelog`. (`deploy-staging-infra` / `deploy-staging-site` were
   the staging-gate leases and were deleted with it — `6j6v.sgfr`.)
6. **macOS Developer ID signing + notarization.** Requires an active
   Apple Developer membership.
   a. **Developer ID Application certificate (.p12):** generate a CSR in Keychain Access
      (Certificate Assistant → *Request a Certificate from a Certificate Authority*), create a
      **Developer ID Application** cert on developer.apple.com under *Certificates*,
      download the `.cer`, import it into the keychain, then export the cert **+ private key**
      as a `.p12` (strong export password). Base64:
      `base64 -i DevID_Application.p12 | tr -d '\n'`.
      → secrets `APPLE_DEVELOPER_ID_APP_CERT_P12_BASE64`, `APPLE_DEVELOPER_ID_APP_CERT_PASSWORD`.
   b. **Notarization API key (.p8):** App Store Connect → *Users and Access* → *Integrations*
      → *App Store Connect API* → generate a key (role *Developer* suffices for notarization);
      the `AuthKey_XXXXXXXXXX.p8` is downloadable **only once**. Note the Key ID (10 digits) and
      Issuer ID (UUID, at the top of the Keys page). Base64: `base64 -i AuthKey_*.p8 | tr -d '\n'`.
      → secrets `APPLE_NOTARY_KEY_P8_BASE64`, `APPLE_NOTARY_KEY_ID`, `APPLE_NOTARY_ISSUER_ID`.
   c. **Team ID** (10 digits, Membership page): **not required** by API-key notarization
      (the pipeline uses only key/key-id/issuer) — `APPLE_TEAM_ID` only optional as a reference in
      the backup; not consumed in the workflow.
   d. Set the repo **variable** `NXF_MACOS_SIGNING_ENABLED=true` (arms the fail-closed
      signing — prod releases abort without this variable rather than shipping unsigned).
   e. **Offline backup** (password manager): `.p12` + export password, `.p8` + Key ID +
      Issuer ID + Team ID. The `.p8` is not re-downloadable — loss = generate a new key.
   f. **(optional) Self-hosted macOS runner against GitHub macOS costs.** Register an Actions
      runner on a dedicated Mac (Intel suffices — both darwin targets build there, `x86_64` native +
      `aarch64` cross), with Xcode Command Line Tools installed; set the repo variable
      `NXF_MACOS_RUNS_ON` to the label array (e.g. `["self-hosted","macOS","X64"]`).
      Empty ⇒ `macos-14` fallback. Saves the expensive macOS minutes incl. the `notarytool --wait`
      idle time. Run it **dedicated + single-tenant** (a dedicated runner in a runner group
      only for this repo), since `$RUNNER_TEMP` is not wiped between jobs (details §5.3).
7. ~~First infra deploy via PR (gate → staging → E2E → merge → prod); then the first
   site deploy.~~ **No longer a step here** (`6j6v.sgfr`): this repo deploys no infrastructure.
   The delivery stack is bootstrapped in `nxsflow-landing-page`; what this repo needs from it is
   a role `nxs-<env>-release` and a bucket `nxs-<env>-artifacts-<account>` to exist.
8. Rehearsal: `release.yml` via dispatch against staging (with `NXF_MACOS_SIGNING_ENABLED=true`
   the run verifies codesign + notarization + `spctl --assess`); then the first real
   tag release.
9. **npm publishing of `@nexus-flow/mcp` via trusted, STAGED publishing.** The `publish-npm`
   job authenticates by **OIDC trusted publishing — no long-lived npm token** (validated against the
   current npm docs; this supersedes the earlier `NXF_NPM_TOKEN` plan). It is fail-closed: a prod release
   only reaches npm once the trusted publisher is configured on the package, so nothing ships prematurely.
   **Since 6j6v.c7dd it STAGES rather than publishes** (`npm stage publish`): the version sits on npm
   unavailable to the public until a maintainer approves it with 2FA. That is npm's default for trusted
   publishers created since 2026-09-03, and the owner's choice: no CI run can put a package in front of
   `npx` users on its own. **Every release therefore ends with one human step** — see g.
   One-time owner steps (require the npm account that will own the org):
   a. **Own the scope.** Create the **`@nexus-flow`** org on npmjs.com (this reserves the `@nexus-flow/*`
      scope) and confirm the **`@nexus-flow/mcp`** name is free.
   b. **Create the package (first publish).** Trusted-publisher config is set on a package's settings page,
      which generally needs the package to exist. Do a **one-time** first publish to create it — locally
      from `npm/mcp/` with `npm publish --access public` (bump `version` off `0.0.0-dev` first, or let the
      release stamp it), or via a temporary token — **then** configure trusted publishing for every release
      after. (If npm's package-settings page lets you configure a trusted publisher before first publish,
      use that and skip the manual publish.)
   c. **Configure the trusted publisher.** npmjs.com → `@nexus-flow/mcp` → *Settings* → *Trusted Publisher*
      → **GitHub Actions**, with: organization/owner **`nxsflow`**, repository **`nexus-flow`**, workflow
      filename **`release.yml`** (filename only). Leave *Environment* blank (the job uses none). Under
      *Allowed actions* keep **stage publish only** (npm's default); do NOT tick direct publish — the job
      never uses it, and leaving it off is what makes the approval step binding. A trusted publisher that
      allows only staging answers a direct `npm publish` with `403 … OIDC permission denied for this
      action`. These must match the `publish-npm` job exactly (`id-token: write`, `release.yml`). The entry
      is bound to the repository, not only its name: after the 2026-09 cutover to the history-free repo
      the old entry answered `404` on the PUT until it was removed and re-added.
   d. **Lock it down.** Once a trusted publish works, set the package to *Require two-factor authentication
      and disallow tokens* (npm recommendation) — OIDC still works, and stray tokens can no longer publish.
      *Optional defense-in-depth (yk2c / review #160.4).* The trust today is "any `release.yml` run with
      `id-token: write`"; the `IS_PROD` + `NXF_NPM_PUBLISH_ENABLED` gate already closes the hole in
      practice. To pin the OIDC subject tighter, bind the trust to a dedicated **GitHub Environment**: add
      `environment: <name>` to the `publish-npm` job **and** set the **same** Environment in the npm
      Trusted-Publisher config — they must move in **lockstep** (adding `environment:` alone changes the
      OIDC subject and would break a trust that expects none, so do not add it repo-side until npm is
      configured for it). Deliberately **not** wired yet; do it here if you want the tighter subject.
   e. **Arm the prod path.** Set the repo **variable** `NXF_NPM_PUBLISH_ENABLED=true` (the existing gate;
      staging rehearses the packaging regardless — **registry-free** via `npm pack --dry-run` plus
      assertions on name, stamped version and packed file list, so a rehearsal of an already-published
      version still means something; `npm publish --dry-run` cannot serve here because it asks the
      registry to validate the version and fails on every re-run — 6j6v.xk2t). **No** `NXF_NPM_TOKEN`
      secret is needed — trusted publishing
      replaces it; if one was ever added, remove it.
   f. **Verify end-to-end (live).** Cut/promote a release and confirm `@nexus-flow/mcp@<version>` publishes.
      (Provenance: while `nexus-flow` is **private** it is **not** attached — a known npm limitation, not a
      failure. Once the repo goes **public** the `publish-npm` job passes an **explicit `--provenance`**,
      auto-gated on repo visibility (yk2c): the publish then **fails closed** if an attestation cannot be
      attached, so a silent default-change can never ship an unattested package. Nothing to flip — it
      activates by itself at going-public.) Then a **real cold start** on
      a machine **without** `nxs`: `npx -y @nexus-flow/mcp -- --workspace <path>` fetches the signed `nxs`
      from the live CDN, passes **sha256 + minisign**, caches, and serves. Confirm a **tampered/wrong-key**
      artifact still **aborts** against the live pubkey (the fail-closed guarantee — already unit-proven by
      the shim's `node --test` suite, re-asserted against the real CDN here). npm CLI ≥ 11.15.0 (staged
      publishing; OIDC alone needed 11.5.1) + Node ≥ 22.14 are required; the job pins them (Node 24 +
      `npm install -g "npm@>=11.15.0 <12"`).
   g. **Approve every release (the human step).** The `publish-npm` job goes green when the version is
      STAGED, and writes the approval instructions into its job summary. Approve on npmjs.com
      (`@nexus-flow/mcp` → staged versions) or with `npm stage list @nexus-flow/mcp` then
      `npm stage approve <stage-id>`; both ask for 2FA. `npm stage reject <stage-id>` discards a bad one.
      Until then `npx -y @nexus-flow/mcp` keeps resolving the previous version. Do not re-run the job for
      a version that is staged but unapproved: npm refuses a second copy of the same version, and the
      job cannot see the queue (its OIDC token may only stage or publish), so it fails rather than guess.

## 13. Implementation phases

| Phase | Content | Result |
|---|---|---|
| **P1 — version & changelog** | `[workspace.package]`, `xtask` (version/changelog), `changes/`, `version-check.yml`, `changelog-check.yml` | Tags are guarded, the notes pipeline stands |
| **P2 — infra** | `infra/` (CDK: trust anchor, cert, distribution), `staging-gate`/`infra-staging`/`infra-prod`/lease, E2E | `nxf[-staging].nxsflow.com` delivers; updater Lambda live |
| **P3 — release & promotion** | `release.yml` (matrix, minisign, publish), `promote.yml`, `install.sh` | `curl … install.sh \| sh` works; beta→stable without rebuild |
| **P4 — self-update** | `nxf self-update`, update hint, channel config | Full update loop, dogfooding on beta |
| **P5 — landing page** | `site/` (brand, EN/DE, changelog), `site.yml` | Public showcase |
| **P6 — follow-up channels** | Homebrew tap in promote, GHCR `:stable` retag, possibly Windows | Reach |

P1–P3 are the critical path; P4 can run in parallel with P5. Each phase ends with a
staging rehearsal before it touches prod.

## 14. Tie-breaker log

The brief requires that uncertainties be resolved **not** by going back with questions but by
inference from the reference project, the product vision, or best-practice research — and that
this be documented. Here is every decision made:

| # | Uncertainty | Decision | Rationale / source |
|---|---|---|---|
| **TB-1** | **Landing page domain** — nowhere specified; the vision only says "own landing page". | `nxf.nxsflow.com` (staging: `nxf-staging.nxsflow.com`) | Reference pattern: products without their own TLD live as a short subdomain under `nxsflow.com` (`bd.nxsflow.com`, `bp.nxsflow.com`, `overtone.nxsflow.com` per the brand guideline). `nxf` is the binary name — domain == command strengthens recognition. An own TLD (e.g. nexus-flow.dev) would be a brand decision that is not mine to make; the subdomain is the conservative default, upgradeable at any time. |
| **TB-2** | **Distribution tooling** — home-grown pipeline vs. `cargo-dist` (actively maintained, v0.31 Feb 2026, generates CI/installer/manifests itself). | Adapted home-grown pipeline following the beads-dashboard model; cargo-dist rejected. | The brief is explicitly "adapt the reference project's approach". Its core values — channel rings with build-once-promote, kill switch/rollout via our own manifests, delivery via our own domain/CDN, staging rehearsal — are not expressible in cargo-dist (it owns the GitHub release flow, not the AWS ring). Mixing two systems would yield two truths. cargo-dist remains a reference for installer details. |
| **TB-3** | **Self-update implementation** — the `self_update` crate vs. a home-grown build. | A thin home-grown build against our `/updater` endpoint. | The crate is tailored to GitHub-releases/S3-listing backends; our contract (channel manifest, kill switch, rollout, minisign pubkey in the binary) is exactly the Tauri updater semantics of the reference, which the crate does not provide. The home-grown build is small (HTTP GET + sha256 + minisign-verify + atomic rename) and keeps the guarantee "no installation without a valid signature". |
| **TB-4** | **Auto-update behavior of the CLI** — desktop apps update in-app; what does an agent CLI do? | No background auto-update; an explicit `nxf self-update`; a hint only on stderr, throttled, disableable. | Product vision: agent ergonomics is the measuring stick, `--json` deterministic. A binary that replaces itself between two agent calls breaks determinism; an update banner in stdout would break every JSON parser. Precedent rustup/gh: an explicit update command + a discreet hint. |
| **TB-5** | **Platform matrix** — the reference is macOS-only (universal); what does nexus-flow v1 need? | macOS arm64 + x86_64 (separately, no lipo), Linux x86_64 + aarch64 (musl, static). **Windows deferred** (Phase 2/P6). | Vision: an adoption engine for developers & agents ⇒ Linux is mandatory (CI runners, servers, containers), macOS mandatory (dev machines, dogfooding). Windows has additional costs (self-replace semantics, signing) with an unclear initial target group — deliberately pushed back. No universal binary: the reference's lipo artifact solved a desktop problem (one signed artifact for both slices); with a CLI the install script picks the target, two lean binaries are simpler and smaller. musl static because of the E0 finding "single static binary = best adoption engine". |
| **TB-6** | **macOS signing/notarization** — the reference notarizes (optional, `APPLE_*` secrets); does the CLI need it from day 1? | **Revised (2026-06-15): realized** with Developer ID signing + notarization (online, no stapling, tar.gz unchanged). Originally deferred (hook toggleable). | The v1 rationale (deferred) holds unchanged for the `curl|sh` path (no quarantine ⇒ Gatekeeper does not engage), but **quarantined channels** (Homebrew Cask 85y.22, browser download) trigger Gatekeeper and need the OS-native first-install anchor against origin/CDN compromise. **Stapling rejected:** a bare CLI Mach-O is not staplable; a `.pkg` would need an installer cert + rework of install.sh/Homebrew/promote away from the build-once-promote tarball — disproportionate, since online notarization provides the same anchor (only a first-launch network connection on quarantined channels). See §5.3. |
| **TB-7** | **Tooling language** for the version/changelog scripts — Node (like the reference) vs. Rust. | Rust (`xtask` crate), identical semantics & formats. | nexus-flow is a pure Rust repo (CLAUDE.md: TDD, one toolchain); Node only for `site/` (unavoidable there, Vite/Lingui). The reference scripts together with their tests serve as a porting template — the semantics are carried over, not reinvented. |
| **TB-8** | **crates.io publishing** — open-source engine, but all crates are set to `publish = false`. | Deferred until API stabilization (at the earliest with the programmatic plugin layer). | The existing code says `publish = false` (a deliberate setting); the vision sells the engine as a *binary/CLI* ("downloadable engine"), not as a library. A prematurely published core crate freezes the data model while E2–E4 are still shaping it. Binary distribution covers 100% of the v1 need. |
| **TB-9** | **Relay distribution** — does `nxf-relay` belong in this release model? | Yes, minimally: included in the CLI tarball + a container image on GHCR (`:beta`→`:stable` via retag, no rebuild). Hosted relay: out of scope. | Vision: the sync server is "open source and self-hostable" ⇒ self-hosters need an artifact from the first release. An image retag is the container-native form of build-once-promote. *Operating* a managed relay is SaaS work (E4 follow-up slices) and not release management. GHCR instead of ECR, because the image belongs to the showcase (GitHub), not to paid delivery. |
| **TB-10** | **AWS account topology** — share the beads accounts or have our own? | **Revised (owner decision, 2026-06-12): shared nxsflow accounts** (staging: `manufaktio-staging`, prod: `manufaktio-prod`) for nexus-flow, nexus-chat, and beads-dashboard. The original decision (two dedicated accounts) is rejected. | The products are predominantly downloadable tools; the signature chain lies outside AWS (minisign key = GitHub secret, clients fail-closed) — an account compromise cannot ship malicious updates. nexus-flow remains cleanly namespaced (`nxf-*` roles/buckets/SSM). Accepted residual risks: mutual CI trust via the account-wide CDK bootstrap roles; `install.sh` is shipped unsigned. **Revisit trigger:** a hosted relay with tenant data in prod ⇒ an account split with a dedicated CDK qualifier (instead of the default `hnb659fds`). |
| **TB-11** | **Brand/design of the landing page** — nexus-flow is missing from the brand hierarchy of the guideline. | Parent-brand language ("The foundation"): lime `#B9FF66` sparingly on white, graphite, Merriweather/Merriweather Sans, JetBrains Mono for terminal; tone confident/understated. | nexus-flow is literally the foundation of both flagships ("powered by nexus-flow", the Intel-Inside model of the vision) — the parent language is the only one that tells this role. The open-source alternative (Overtone lavender) is, as a product-specific palette, explicitly not mixable ("never mix colors between products"). The final logo decision (its own monogram?) remains a brand question — until then, the parent mesh icon + wordmark. |
| **TB-12** | **Languages of the release notes** — EN-only or EN+DE? | EN+DE, exactly like the reference. | The reference's fragment format, feed, and bilingual landing page are built for it; the target group of the flagships is (also) German-speaking. Cost: one extra sentence per PR. |
| **TB-13** | **Update-hint source for "What's new"** — the app has an in-app view; the CLI? | `notes` in the updater response (shown by `self-update`) + the website changelog. An `nxf changelog` command is an optional follow-up build, not v1. | The reference's four feed surfaces (release body, updater notes, in-app view, website) naturally shrink to three for a CLI; a dedicated changelog view in the terminal has no v1 audience (the agent does not read it, the human reads the website). |
| **TB-14** | **Channel persistence** — where does the CLI remember the ring? | User config (`~/.config/nxf/config.toml`, `channel = "stable"`), overridable via flag/env. **Not** in the workspace (`.nexusflow/`). | The ring is a property of the *installation* (one binary per machine), not of the project — a workspace setting would mean that two projects could "want" different binary states, which a single binary cannot fulfill. Analogous to the reference's in-app picker (an app setting, not a document setting). |
| **TB-15** | **Facade Rust-API stability** — §4.3 defined "API in the SemVer sense" as the CLI `--json` contract only; the `pub` surface of `crates/facade` (consumed via git tag, `publish = false`) had no promised stability, which manufakt needs to automate adoption. | The public API of `crates/facade` is a SemVer contract **in addition to** the CLI contract: patch never breaks it, minor is the break axis (0.x; strict SemVer from 1.0). See §4.3. | manufakt embeds the facade and pins git tags; to pull security patches unattended they need a hard "a patch never breaks the facade" promise. Making the patch axis safe by construction is what makes a fast-track lane possible; enforced mechanically by `cargo-semver-checks`, surfaced by the facade changelog marker. (Decision.) |
| **TB-16** | **Crate consumption mode + "stable" signal** — the spec described only the binary channel; how does an embedding consumer pin the facade, and how do they tell stable from beta when tags are plain SemVer with no suffix? | Git-tag pin is the official mode for embedded crates (exact tags, no moving refs); "stable" is read from GitHub release metadata (`prerelease == false && latest`), **not** from the tag name; promotion moves no tag (identical bytes, only the ring marker flips). See §4.4. | crates.io is out (TB-8), so a git-tag pin is the only reproducible source dependency. Encoding stability in the tag string would fight the "plain SemVer, the channel is a ring not part of the version" rule (§2/§4.3); the Releases API already carries `prerelease`/`latest`, so the ring marker is the natural, machine-readable stable signal. |
| **TB-17** | **Security-patch reach across lines** — fix-forward on a single line cannot serve consumers who pin a minor and want "security daily, no unplanned minor jumps"; how many lines do we backport to, and for how long? | N-2 (current + 2 previous minor lines); grace window ~6 weeks calendar-based after the successor minor's promotion, then retire the line; no open-ended LTS in 0.x. See §4.5. | Without backports every vulnerability forces a potentially breaking minor and the patch-lane auto-adopt (§4.4) collapses. N-2 bounds the maintenance cost to ≤3 active lines; a calendar grace window is simple to communicate and avoids an unbounded LTS commitment in a pre-1.0 product. Delivered by the maintenance-branch mechanics. (Decision.) |
| **TB-18** | **Which relay backends the released artifact carries** — `postgres` and `dynamodb` are opt-in cargo features, so the shipped `nxf-relay` could select neither: `NXF_RELAY_BACKEND=dynamodb` was accepted by an operator and answered with the unknown-backend panic. Ship every backend in one binary, build the §5.6 container image and give it the features, or declare the source build the supported path? | **Split by measured cost: ship `postgres`, keep `dynamodb` a source build** (2026-08-07, owner). The release job builds `--features nxs-server/postgres`; the §5.6 image waits on authentication (TB-19). | Measured, not assumed — cold relay build: sqlite-only 76 crates / 57 s / 3.8 MB; +postgres 153 / 71 s / 5.0 MB; +dynamodb 247 / 3 min 35 s / 22.6 MB. Postgres is nearly free and makes *“sync to a Postgres, e.g. Supabase”* true of the downloaded artifact — the realistic answer for a user who wants the board off their laptop or shared with a colleague. DynamoDB pulls `aws-lc-sys` (native C/asm, needs `cmake` + a C toolchain on all four runners, two of them cross-compiling to musl through zig) to add ~19 MB to every tarball for a capability no tarball consumer takes: `manufakt-io` builds the relay from a pinned revision with its own toolchain (`apps/web/relay-pin.json`) and never unpacks ours. Omission alone was the original defect, so the source-build path is written down (§5.2, §5.6, guide `running-a-relay`, release notes), not merely left out. |
| **TB-19** | **The §5.6 container image** — specified since day one, never built. Delete the promise or keep it? | **Keep it, gated on client authentication** (`xsf3` → `6aza`), tracked as `6j6v.v99p` with a real dependency edge; §5.6 states plainly that no image is published today. | Deleting it would throw away the right answer to the user's actual question (*where do I run this?*) — `docker run` is the shortest path from “I use the CLI” to “my board is backed up and my colleague can see it”. Building it now would be worse: the relay has no client authentication, so a one-command way to run it is a one-command way to put an unauthenticated shared board on the internet, and the convenience is precisely what would make that happen. A ticket with an edge cannot rot the way a paragraph of intent did. |

## 15. Open items (genuine follow-up decisions, not blockers)

- **A dedicated monogram/logo** for nexus-flow (a brand question for the human; until then TB-11).
- **Windows slice** (P6): self-replace semantics, code signing, `zip` instead of `tar.gz`.
- **Updater Lambda hardening**: Function URL to CloudFront-only (OAC + `AWS_IAM`) — the same
  open item as in the reference, the same risk assessment (only public metadata).
- **Branch protection/required checks**: on the org's Free plan only advisory (reference
  `2dm`); make required when switching to a public repo (`version-check`, `changelog-check`,
  `ci`).
- **Telemetry of the rollout control**: deliberately none (no phone-home except the explicit
  update check); if ever desired, a separate decision with opt-in.
