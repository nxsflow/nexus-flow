# E5 — Embedding Host API: versioned library contract (design spec)

> Status: Design · As of: 2026-06-18
> Prerequisite: E2 (Agent CLI MVP) + the facade extraction.
> This spec is the **deliberate, versioned stability contract** for the programmatic
> engine surface that an embedding **application** (not an agent) consumes. It anchors the
> epic's acceptance — "a documented, versioned API contract exists" — in writing.

## 1. Goal & scope

E5 has built the **programmatic, in-process engine access**: a long-lived handle
([`engine::Engine`]) that owns workspace + store for the lifetime of the app, file-based
reactivity ([`engine::Engine::subscribe`]), and write paths with the same convergence semantics as
the CLI. This surface grew CLI-driven and ad hoc; this ticket **raises it to a
deliberate library contract** — without touching it any further in substance. It is final after the
write paths and the robustness follow-ups (`#2at`, `#irr`, `#i8o`,
`#9t7.9`, `#9t7.10`); here it is only written down.

**Contract subject:** the public (`pub`) API of the crate **`nexus-flow-facade`** plus the
few core types that show through in its signatures (§2.2). That is *the*
embedding host seam; the CLI and (in future) MCP consume the same facade.

**Not in this contract** (§7): crate-internal modules, the core's SQL / materialized views, the
on-disk schema (its own compatibility axis, E4 / platform foundation), and
foreign-language bindings (napi/WASM/UniFFI — its own future epic, cf. `9t7` design point 2).

## 2. The contract surface (documented public items)

The authoritative source is the `rustdoc` of the crate surface; this list is the **deliberately
curated enumeration of what belongs to the contract**. A `pub` item named here (or via a
stability tier in §4) falls under the guarantees in §3/§5. Modules are re-exported in
`crates/facade/src/lib.rs`.

### 2.1 `nexus-flow-facade` — public modules & items

- **`engine`** — the long-lived handle (heart of the embedding seam):
  - `Engine` (`Clone + Send + Sync`): `open`, `subscribe`;
    reads `ready`/`blocked`/`next`/`show`/`list`/`prime`, `plugin_config`, `workspace`;
    writes `create`/`update`/`claim`/`close`, `dep_add`/`dep_remove`,
    `mention_add`/`mention_remove`, `contributes_add`/`contributes_remove`, `note_add`.
- **`error`** — every consumer's error envelope: `ErrorKind`, `NxfError` (+ constructors),
  `Result<T>`.
- **`watch`** — change notification: `Change`.
- **`read`** — the compute→render read layer (for consumers that render without the handle):
  `ready`, `blocked`, `next`, `show`, `list`, `prime` + the `*_to_value` projections
  (`items_to_value`, `blocked_to_value`, `next_to_value`); types `BlockedItem`, `ShowRecord`,
  `Blocker`, `BlockedEntry`, `PrimeReport`.
- **`write`** — the shared mutation layer: `NewItem`, `UPDATABLE_FIELDS`, `create`, `update`,
  `claim`, `close`, `dep_add`/`dep_remove`, `mention_add`/`mention_remove`,
  `contributes_add`/`contributes_remove`, `note_add`.
- **`record`** — the canonical, presentation-independent JSON record: `CanonicalItem`,
  `item_value`, `items_value`, `raw_field`.
- **`plugin`** — the declarative plugin config + the compile-time self-registration registry
  (r16h.1): `PluginConfig`, `Vocabulary`, `Priority`, `Ranking`, `RankSpec`, `RankKey`, `Dir`,
  `Nulls`, `Presentation`, `ListView`, `PluginDescription`, `MergeStrategy`, `load`, `available`
  (fallible), `plugin_names`, `registry::{PluginRegistration, registered, find, resolve}`.
  `PLUGIN_NAMES` is **`#[deprecated]`** — the registry-driven `plugin_names()` supersedes it (it
  also lists a consumer's registered plugin); kept one minor as the §5 coexistence window.
- **`workspace`** — `.nexusflow/` resolution + store opening: `Workspace` (+ `db_path`,
  `open_store`), `WorkspaceConfig`, `Replica`, `resolve`, `discover`, `init`, `adopt_prefix`.
- **`validate`** — shared input validation: `iso_date`.

### 2.2 Core types that show through (part of the contract)

These `nexus-flow-core` types appear in the signatures above and are therefore part of the contract
(an incompatible change to them is a contract change per §3): `model::ItemRow`,
`model::ItemType`, `model::EdgeKind`, `store::Store`. The **rest** of the core (op-log tables,
reducers, materialized views, SQL) is **not** part of the contract — it is implementation
behind the facade.

## 3. Versioning & semver policy

**Version source.** A single version for all product crates, `[workspace.package] version`
(today `0.3.1`); the crates inherit it via `version.workspace = true`. It is identical to the
`nxf` release version, and the release tag `v<version>` is the truth (CI enforces
tag == version fail-closed, see `release-management.md` §4.1). **The embedding contract shares
exactly this version number** — there is no separate API version.

**`publish = false` — what the contract still means.** All crates are deliberately not published to
crates.io (`publish = false`); a consumer links the facade as a path/git dependency
(the first one, the Tauri example app, lives in-repo under `examples/tauri-board`). Classic
crates.io semver therefore does not apply mechanically. The contract is consequently a **deliberate,
written-down policy commitment**: changes to the contract surface (§2) are versioned and communicated
**as if** semver applied — so that the in-repo consumers and later external consumers have a real
stability guarantee.

**Meaning of the positions (`0.y.z`, pre-1.0).** As long as MAJOR is `0`, the pre-1.0 reading of
semver applies:

- **MINOR (`y`)** — may contain breaking changes to the contract surface (pre-1.0 right).
  Breaking = removed/renamed public item, changed signature/field set, changed
  error `ErrorKind` mapping of a documented case, or a behavioral change that would break a
  conforming consumer. Every such change MUST appear in the changelog under `changed`/
  `removed` and (where possible) go through the deprecation path in §5.
- **PATCH (`z`)** — additive or fixing only: new public items, new optional fields behind
  defaults, bugfixes that restore the **documented** behavior. Never breaking.

**1.0 and after.** With `1.0.0`, the policy flips to strict semver: breaking contract changes
only in MAJOR, additive ones in MINOR, fixes in PATCH. The transition to `1.0` is the explicit
commitment "this surface is stable"; it happens only once an external (non-in-repo) consumer relies on it.

## 4. Stability tiers

| Tier | Meaning | Items |
|------|-----------|-------|
| **Stable** | Full contract (§3/§5). | The entire §2 surface, unless named below. |
| **Unstable / `#[doc(hidden)]`** | No contract; may break at any time (including in PATCH). Pure test/tooling seams. | `engine::Engine::open_with_poll_interval` (`#[doc(hidden)]` test seam for the watcher cadence). |
| **Tooling switch** | No app contract; env-driven, only for deterministic golden docs/tests. | `workspace::deterministic_ids_enabled` (reads `NXF_DETERMINISTIC_IDS`). |

`#[doc(hidden)]` items do not appear in the `rustdoc` and are **by definition** not part of the
contract surface; a consumer that calls them does so at its own risk.

## 5. Deprecation path

Before a stable item is removed or changed in a breaking way:

1. **Mark** with `#[deprecated(since = "<version>", note = "<replacement/rationale>")]` — the
   compiler warns every consumer at the call site.
2. **At least one MINOR cycle** of coexistence: the deprecated item stays functional, the
   replacement stands beside it.
3. **Changelog fragment** (`type: changed`) describes the migration in EN + DE.
4. **Remove** no earlier than the next MINOR after the marking; the removal is itself a
   breaking change (`type: removed`, §3).

Exception: an item that was never stable (tier "Unstable"/`#[doc(hidden)]`) may disappear without
this path.

## 6. Seam invariant (no new semantic layer)

The contract includes a **behavioral** invariant, not just a type signature: the
embedding surface adds **no** new semantics — same core ops, same derivation
(`ready`/`blocked`/`next`), same canonical record as the CLI contract (E2 §7) and the
MCP seam (E9). Three seams, one core; they must never diverge. This is not mere docs, but
**tested**: the App↔CLI differential test pins identical derived
results over the same store. A contract change that would break this parity is forbidden —
regardless of the version position.

### 6.1 The omission half of the invariant (nxf `6j6v.vtvs`)

The differential proves the seams do not **diverge**. By construction it cannot see whether one of
them is **missing** a verb: it drives the same script through both sides, so a verb that exists only
in the CLI is never in the script in the first place. Strong against divergence, defenceless against
omission — and that is not a theoretical hole. `nxm prime` and `nxc prime` assembled their
SessionStart block inside `cli.rs` for a year with all three differentials green, until
app-foundations had to rebuild the same assembly in TypeScript (Foundation v0.35.0) because the
library could not hand it over (epic `6j6v.fjrc`).

The second half is therefore a **verb-list comparison**, one per module
(`crates/{cli,memory,chat}/tests/verb_seam.rs`, engine in `crates/test-support`), fail-closed in CI:

- The CLI list is walked out of the `clap` tree, **hidden commands included** (`prime` is hidden and
  is exactly the case that motivated this). The seam list is parsed out of that module's `engine.rs`
  plus its compute layer (`read`/`write` for flow, `facade.rs` for memory and chat). Both derived —
  a gate fed by a hand-maintained list drifts like the thing it watches.
- The **store beneath the facade is not seam.** Reaching past the facade into the store is what §6
  forbids, so a capability that exists only there is an omission, not a counterpart.
- A module declares only the **difference**, and every entry carries its reason: `CliOnly` (the verb
  acts on the host — `init`, `agent-manifest`, `self-update`, import/onboarding paths), `Alias` (on
  the seam under another name; the gate verifies the named symbol exists), or `KnownGap` (a real
  omission **with the board item tracking it**). An exception without a reason is a finding, and a
  gap without a ticket is not a waiver. A waiver also fails once it stops being true — when the verb
  is gone, or when the seam has grown the counterpart.

## 7. Explicitly NOT part of the contract

- **Crate-internal items** (`pub(crate)`/private): e.g. `watch::Watcher`, `watch::file_id`,
  `watch::FileId`, `watch::POLL_INTERVAL`, the `engine`-internal `State`/`Inner`/`ensure_current`.
- **The core beneath the facade**: op-log tables, reducer registry, materialized views, the SQL.
  Only what shows through via §2.2 is stable.
- **The on-disk schema** (`.nexusflow/`): its own compatibility axis (forward-compat fold, E4 /
  platform foundation), not the library API.
- **Foreign-language bindings** (napi-rs/WASM/UniFFI): a deliberately later, separate epic; the first
  consumer is a Rust Tauri backend that links the facade directly as a crate.
- **`nxf` CLI flags & output**: their contract is E2 §7 (`--json`), not this spec — even though
  both drive the same facade.

## 8. Anchoring & maintenance

This spec is the written anchoring of the contract in `docs/specs/`. On every change to the
§2 surface: update §2 here, choose the version position per §3, follow §5, write the changelog
— and verify that the seam invariant (§6, differential test `9t7.6`) stays green. With that,
the surface is a **deliberate library contract**, not a state of affairs derived after the fact.
