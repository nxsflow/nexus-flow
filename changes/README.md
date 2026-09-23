# Changelog fragments (`changes/`)

Every PR with **user impact** adds one fragment here. `cargo xtask version set <v>` folds the
fragments into `release-notes.json` (the single-source feed that drives the GitHub release body,
`self-update` notes, and the website changelog) and deletes them. Spec
`docs/specs/release-management.md` §4.2.

A PR with **no** user impact needs no fragment when every path it touches is one whose change is no
change to what a user gets — specs and runbooks under `docs/`, CI workflows, tests, `xtask/`, the
repository's own prose (`README.md`, `AGENTS.md`, …). The `changelog-check` gate recognises that by
itself, so a contribution from a fork goes green without anyone's help. The list is
`NO_PRODUCT_IMPACT` in `xtask/src/changelog.rs`, each entry with its reason, and it is an
allowlist: the guide pages under `crates/*/docs/`, the diagrams in `docs/architecture/`, every
workflow that can publish, the files that define this gate, and everything else that reaches a user
still need a fragment. For a change outside that list that genuinely has no user impact, a
maintainer applies the `skip-changelog` label. Otherwise the gate is fail-closed: no fragment and
no label ⇒ red, and the error names the paths that ship.

## Format

One file per PR: `changes/<slug>.md`.

```markdown
---
type: added            # added | changed | fixed | removed   (required)
---
[en]
English, user-facing. One or more lines.
[de]
Deutsch, nutzerseitig. Eine oder mehrere Zeilen.
```

- **`type`** (required): `added` | `changed` | `fixed` | `removed`. Renders under the headings
  Added/Changed/Fixed/Removed (EN) · Neu/Geändert/Behoben/Entfernt (DE).
- **`[en]` and `[de]`** (both required, non-empty): the user-facing text. A missing or empty
  language section is rejected loudly (TB-12 — the feed and the landing page are bilingual).

Validate locally before pushing:

```bash
cargo xtask changelog check origin/main
```

## Optional: a curated stable rollup (`changes/stable/<version>/`)

The stable ring is the **net difference between two stable states**, and `cargo xtask changelog
aggregate` builds it by unioning every beta fragment in the window. That is the right answer while
stable follows beta closely. It is the wrong answer when stable has fallen far behind: promoting
0.35.0 → 0.63.0 unioned 74 fragments from 33 versions into 201 KiB of beta-voice prose — a
changelog nobody reads and a `self-update` payload nobody wants.

So a version MAY carry a **curated rollup**: one directory, `changes/stable/<version>/`, holding
ordinary fragments (same frontmatter, same `[en]`/`[de]`, same optional `facade:`/`migration`).
When it exists, those fragments ARE the stable entry's items — `aggregate` renders them and
`promote` writes them, so the manifest and the website say the same thing.

- **Write it for someone upgrading, not for someone following.** Condense every beta refinement of
  one feature into one line, in the voice of the release, and leave the per-version detail on the
  releases page where it already lives.
- **It is items, not rendered copy.** That is deliberate: `notes` stays derived from `items`, so
  `cargo xtask changelog rerender --check` keeps holding that derivation and a re-promote
  reproduces the same result — unlike hand-editing `notes.{en,de}`, which
  `docs/specs/release-management.md` §5.2 warns against for exactly that reason.
- **It is read non-recursively-safe.** `changes/*.md` is the pending-fragment scan and never
  descends, so a rollup can never be mistaken for a fragment or consumed by `version set`.
- **Absent is the normal case.** A one- or two-version stable jump needs no curation.

Flagging intra-window churn `unreleased: true` (above) stays the first tool — prefer it where an
item genuinely never mattered to a stable user. Curation is for what DID matter and simply needs
saying once instead of eleven times.

## Optional: migration note (major bumps)

A fragment MAY carry a migration note describing what happens **automatically vs manually** on
upgrade. Both languages are required if either is present:

```markdown
[migration.en]
What's automatic / what's manual, EN.
[migration.de]
Automatik / Handarbeit, DE.
```

On a real major bump (new major ≥ 1, increased vs the current version) at least one fragment in
the release window must carry a migration note; in 0.x none is required.

## The facade verdict (`facade:`) — required when you touch a consumed surface

Three crates are **consumed library surfaces**: an embedding consumer (manufakt, app-foundations)
pins them by git tag in lockstep and compiles against their `pub` API (spec §4.3/§4.4).

| Surface | Directory |
| --- | --- |
| `nexus-flow-facade` | `crates/facade/` |
| `nexus-chat` | `crates/chat/` |
| `nexus-memory` | `crates/memory/` |

Plus one **contract-critical crate** outside them: `crates/foundation/` — the substrate all three
run on. Its `validate_author`/`is_attributable` is what every public write entry point of all three
delegates to, its store carries the append-time attributability assert, and its error kinds are the
ones those entry points return.

**If your PR's diff touches any of those, every fragment your PR adds or modifies must carry an
explicit `facade:` value.** That includes **deleting** a file from one of those trees, or moving one
out of it — removing public API is a contract change like any other. `changelog-check` is
fail-closed on it:

```markdown
---
type: changed
facade: none           # none | changed | breaking   (required when a consumed surface is touched)
security: true         # optional; marks a security fix
---
```

- **`facade: none`** — checked; the consumed contract does not move. A verdict, not a shrug.
- **`facade: changed`** — the API moved but stayed backward-compatible.
- **`facade: breaking`** — a backward-incompatible change, **including a behavioural one**: an
  input that used to be accepted and now errors is a break even though every signature is
  identical. By the §4.3 contract this may ship **only in a minor** (the break axis) — `cargo
  xtask version set` fail-closes a **patch** bump carrying one, the changelog-side mirror of the
  `cargo-semver-checks` gate. So `facade: breaking` appears, by construction, only in a minor.
- **`security: true`** — surfaced in the feed (and the Facade Contract section when combined with
  a facade flag) so security patches can be prioritized and fast-tracked on the patch lane
  (§4.4/§4.5).

### Why the verdict is mandatory there

`cargo-semver-checks` diffs the API *shape*, so it cannot see that `create(…, "")` went from `Ok`
to `Err`. The axis gate only fires on a fragment that *carries* `facade: breaking`. An **omitted**
marker was therefore indistinguishable from "checked, no impact" — and twice on 2026-08-09 (PRs
#311 and #314) a behavioural break at 26 public write entry points passed both gates green and was
caught only by hand. The rule cannot detect a break; it makes sure someone is asked.

`facade: none` is an assertion about the **PR**, not about the release: it satisfies the gate and
is then dropped at fold time, so the feed's `facade` field keeps carrying only `changed` /
`breaking` for consumers that parse it. `facade`/`security` are omitted from the feed JSON when
unset — existing `release-notes.json` entries are unaffected.

A PR with **no** user impact still opts out with `skip-changelog`, which bypasses this rule too —
deliberately: applying a label is a human act on the record, not a silent omission.

## Optional: beta-only churn flag (`unreleased`)

Stable release notes are the **net diff between two stable states**, not a union of every beta
fragment. When a change only ever mattered inside the beta ring — it fixes or reverts something that
was itself introduced **and** resolved between two stable promotes, so a stable user never saw it —
flag it so the stable rollup leaves it out while the beta ring keeps the full, append-only truth:

```markdown
---
type: fixed
unreleased: true       # optional, orthogonal to type/facade/security; default off
---
```

- **`unreleased: true`** — omit this item from the stable aggregate (`cargo xtask changelog
  aggregate`); it still appears in the beta ring. Typical case: a fix for a regression that only
  existed in beta, or the follow-up fix of an introduce-then-fix pair inside one stable window
  (flag the `fixed`, keep the final `added`).
- The promoter may also set the flag **retroactively** on a beta item in `release-notes.json` when
  intra-window churn only becomes apparent at promote time — a reproducible signal, unlike a
  hand-edit of the rendered notes (which a re-promote would overwrite).
- Optional, default off, omitted from the feed JSON when unset — existing entries are unaffected.

## Example

```markdown
---
type: changed
facade: changed
security: true
---
[en]
Hardened workspace-path resolution in the embedding handle against a traversal edge case. The
public API is unchanged in shape; callers need no changes.
[de]
Die Workspace-Pfadauflösung im Embedding-Handle gegen einen Traversal-Sonderfall gehärtet. Die
öffentliche API bleibt in der Form unverändert; Aufrufer müssen nichts anpassen.
```
