# beads / `bd` provenance review (pre-public gate `wwkf`)

> Status: complete · As of: 2026-07-13 · Gate for: `fasc` (public-going) · Companion to:
> `docs/specs/beads-to-nxs-dogfood-migration.md`

This is the **content** half of the pre-public beads review. The **mechanical** half — a
full-history secret scan (gitleaks + trufflehog) and early-commit/blob review — ran separately as
gate `3wc2` and came back clean. This doc triages every `beads`-related reference in `HEAD`,
records the four provenance decisions the gate asked for, and states the license position.

**Verdict: clear to go public w.r.t. beads provenance.** Attribution, positioning, and the
Apache-2.0 `LICENSE` were already established in PR #90; the migration importer is a first-class,
verified feature; no beads source code was ported, so no NOTICE obligation arises. One *distinct*
finding — internal `beads-dashboard` references — is flagged below for the docs sign-off gate
(`7p5a`); it is not a beads-OSS provenance issue.

## Method

- `git grep -cI 'beads'` over `HEAD` → **~383 hits across 48 files** (the "429" in the ticket is the
  count of migrated *beads issues*, per the migration doc — not source hits).
- Read the three beads modules end to end to separate **ported code** from **format interop**.
- Verified the upstream project: **[beads](https://github.com/gastownhall/beads)** by Steve Yegge —
  "distributed graph issue tracker for AI agents, powered by Dolt", CLI `bd`, released under the
  **MIT license** (confirmed on the upstream repo, 2026-07-13).

## The two distinct "beads"

| Name | What it is | Public exposure |
|---|---|---|
| **beads / `bd`** | The OSS prior-art project (Yegge, MIT). The thing we migrated *off*. | Deliberately attributed. |
| **`beads-dashboard` / `beads-dashboard-av4`** | An nxsflow-**internal** predecessor project/epic (`manufakt.io` in stripped-down form). | Internal name — see *Finding B*. |

## The four decisions (beads OSS)

### 1. `beads_import` — keep as a first-class feature (verified)

The importer is a real migration path, not dead weight. It is split cleanly:

- `crates/nxs-init/src/beads.rs` — pure `serde` structs over the `bd export --all` JSONL
  (`_type`-tagged `issue`/`memory` records) plus detection of beads' managed-block markers
  (`<!-- BEGIN BEADS INTEGRATION … -->`, the `bd prime` hook). **Format consumption, not code.**
- `crates/cli/src/beads_import.rs` — maps `BeadsIssue` onto flow's own core `Store` model
  (type/priority/status/edges/timestamps). Independent implementation.
- `crates/nxs/src/migrate_beads.rs` — consent-gated orchestration (`nxs init --from-beads`):
  back up `bd export`, set up flow+memory, import, roll back only beads' *managed* config.
- `crates/memory/src/import.rs` — maps `BeadsMemory` onto the fact model.

**Still works against the real format?** Yes, with strong evidence:
- The real dogfood migration on **2026-07-06** (v0.16.0) imported **429 issues** with edge counts
  matching an independent `bd export --all` baseline exactly (see the migration doc).
- `crates/nxs/tests/e2e.rs` exercises the import against realistic `bd export` JSONL (including a
  `stub_bd` that emulates `bd export`), covering `_type`, `dependencies`
  (`parent-child`/`blocks`), close reasons, and memories.
- The structs are drift-tolerant: every optional field is `#[serde(default)]`, and known field
  renames are accepted (e.g. `due_at`/`due`), so a sparse or slightly newer export still parses.

**Decision: keep.** Residual risk is only upstream *format drift after* 2026-07-06 (a maintenance
concern, not a public-going blocker). *Optional:* advertise it as a migration path — a one-line
mention was added to the README acknowledgements (see Finding A resolution).

### 2. Concept borrowings in docs — deliberate attribution (done)

The README carries an explicit **"Prior art & acknowledgements"** section crediting beads, `bd`, and
Steve Yegge by name and link, and stating *why* the CLI vocabulary is deliberately familiar. The
engineering/vision docs that mention beads do so as legitimate technical comparison (sync model,
derivation, memory-in-vs-out-of-tracker), not as unattributed lifting. **No random, unattributed
mentions remain in user-facing copy.**

### 3. License — no obligation

- **No beads source code was ported.** The beads modules deserialize beads' *output* (`bd export`
  JSONL) and recognize beads' *managed-block markers*. That is interop with a data format, not a
  derivative work of beads' source.
- Upstream beads is **MIT**; nexus-flow is **Apache-2.0** (`LICENSE`, present and referenced from the
  README). MIT→Apache-2.0 is compatible, but it is moot here because nothing is copied or
  redistributed.
- **Therefore no `NOTICE`/attribution *obligation*.** The README acknowledgement is a courtesy and a
  positioning choice, not a license requirement. (MIT has no NOTICE mechanism; Apache-2.0's NOTICE
  requirement applies to redistributed third-party source, which we have none of.)

### 4. Positioning — defined (done)

The "nexus-flow vs beads" narrative is set in the README and `docs/vision/nexus-flow.md`:
beads is *proven prior art* that set the agent-native bar; nexus-flow rebuilds the foundation
(convergent relational CRDT instead of git/Dolt-style sync; derived `ready`/`blocked`/`next`
instead of stored state; one platform / three products instead of memory-inside-the-tracker) and
carries the idea forward. This aligns with decision `9sk0` (nexus-flow as a product).

## Reference inventory

| Bucket | Files (hits) | Assessment |
|---|---|---|
| Migration/import **code** | `nxs/src/migrate_beads.rs` (71), `nxs-init/src/beads.rs` (45), `cli/src/beads_import.rs` (32), `memory/src/{cli,import}.rs` (23/8), plus init/doctor/frontdoor wiring | Feature — keep. Interop, no ported code. |
| **Tests** | `nxs/tests/e2e.rs` (30), `golden/verbs.trycmd`, `cli/tests/convergence_e2e.rs` | Validate the importer against realistic `bd export`. Keep. |
| Engineering/vision **docs** | `docs/specs/beads-to-nxs-dogfood-migration.md` (25), `docs/vision/nexus-flow.md` (13), `docs/specs/release-management.md` (7), others | Legitimate comparison / migration record. Keep. |
| **User-facing** | `README.md` (6) | Deliberate attribution. Done. |
| **Generated** | `docs/generated/{man/nxf-init.1, commands.json, cli.md}` | Regenerated from CLI help — reflect the `--from-beads` flag. Keep. |
| `.gitignore` | ignores `.beads-credential-key` etc. | Harmless; `3wc2` confirmed the key was never committed. |

## Finding A — advertise the migration path (resolved)

The acknowledgements mentioned beads but not that an existing beads board can be migrated. Added a
one-line, factual mention of `nxs init --from-beads` to the README so the prior-art credit doubles
as a migration on-ramp. Low-risk, in scope for "optionally advertise as a migration path".

## Finding B — internal `beads-dashboard` references (hand off to `7p5a`)

Distinct from beads-OSS provenance: several files reference the nxsflow-**internal** predecessor
`beads-dashboard` / epic `beads-dashboard-av4`, which would become public as-is:

- `infra/lib/distribution-stack.ts:4` — "Adapted from the reference `distribution-stack.ts`
  (beads-dashboard), names changed."
- `infra/lib/github-oidc.ts:11,34` — "production beads-dashboard roles", "reference beads-dashboard
  roles".
- `infra/README.md:32`, `site/README.md:8` — beads-dashboard named as a sibling product/blueprint.
- `docs/vision/nexus-flow.md` — `Ref: beads-dashboard-av4` header and a "Relationship to
  `beads-dashboard-av4`" section.

None of this is sensitive (no secrets — `3wc2` is clean; these are nxsflow's *own* internal names and
"adapted-from" credits). But it exposes an internal project/epic name and is a **positioning/tone**
decision, not a beads-OSS provenance one. **Recommendation:** fold into the docs language review +
sign-off gate `7p5a` — decide per reference whether to keep (harmless internal credit), rename, or
drop. Left unchanged here to avoid pre-empting that gate.

## Bottom line

The beads-OSS provenance gate is satisfied: importer verified and kept, attribution and positioning
deliberate, Apache-2.0 license in place, no ported code and thus no NOTICE obligation. The only
open thread is the internal `beads-dashboard` naming (Finding B), handed to `7p5a`. The public
switch itself stays with the owner (`fasc`).
