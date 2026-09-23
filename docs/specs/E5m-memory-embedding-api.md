# E5m — Memory Embedding API: versioned library contract (design spec)

> Status: Design · As of: 2026-06-26
> Prerequisite: the memory verbs + the memory-facade extraction
> + the in-process engine handle.
> This spec is the **deliberate, versioned stability contract** for the programmatic
> memory surface that an embedding **application** (not an agent) consumes. It anchors the
> epic's acceptance — "a documented, versioned API contract exists" — in writing. It is the
> direct mirror of flow's E5 contract (`docs/specs/E5-embedding-host-api.md`).

## 1. Goal & scope

The memory engine (`nxm`) grew CLI-shaped: every verb opened the workspace fresh, printed the
record, and returned `Result<()>`. E5m has built the **programmatic, in-process engine access** so
an embedding application can hold memory the way flow's E5 lets it hold the issue tracker: a
long-lived handle ([`engine::Engine`]) that owns workspace + store for the lifetime of the app,
file-based reactivity ([`engine::Engine::subscribe`]), and write paths with the same convergence
semantics as the CLI. This ticket **raises that surface to a deliberate library contract** — without
touching it further in substance. It is final after the facade (`#3rx.1`), the handle (`#3rx.2`), and
the differential (`#3rx.3`); here it is only written down.

**Contract subject:** the public (`pub`) API of the memory-embedding modules in the
**`nexus-memory`** crate — `engine`, `facade`, `watch` — plus the few items that show through in
their signatures (§2.2). That is *the* embedding host seam for memory; the CLI and (later) the MCP
server (E9/`#76u`) consume the same facade.

**Why a module surface, not a separate crate (vs flow's `nexus-flow-facade`).** flow extracted its
facade into its OWN crate because flow's store/reducer live in `crates/core`, separate from the
clap-bearing `crates/cli`. memory's store, reducer, model, and CLI already live together in the one
`nexus-memory` library crate, so the facade + engine are modules WITHIN it rather than a fourth
crate. The stability promise is identical — a written-down policy over the named `pub` surface
(§3) — only the packaging differs. (Splitting a clap-free `nexus-memory-embed` crate out, the way
flow did, remains a possible later refactor; it would not change this contract's surface or
guarantees.)

**Not in this contract** (§7): crate-internal items, the `nxm` CLI flags/output (their contract is
the `--json` agent surface + the goldens), the foundation substrate / op-log / SQL, the on-disk
schema (its own compatibility axis, E4 / platform foundation), and foreign-language bindings
(napi/WASM/UniFFI — a deliberately separate future epic, as for flow's E5).

## 2. The contract surface (documented public items)

The authoritative source is the `rustdoc` of the surface; this list is the **deliberately curated
enumeration of what belongs to the contract**. A `pub` item named here (or via a stability tier in
§4) falls under the guarantees in §3/§5. Modules are declared in `crates/memory/src/lib.rs`.

### 2.1 `nexus-memory` — public embedding modules & items

- **`engine`** — the long-lived handle (heart of the embedding seam):
  - `Engine` (`Clone + Send + Sync`): `open`, `subscribe`;
    reads `recall`/`memories`/`prime`; writes `remember`/`forget`; `workspace`.
- **`facade`** — the compute→render layer (for a consumer that renders without holding the handle,
  and the layer the engine + CLI + MCP all share):
  - the canonical record `MemoryRecord` (fields `key`, `body`, `author`, `updated`, `active`);
  - reads `recall`, `memories`; writes `remember`, `forget` (each takes `now`/`actor` explicitly);
  - the session bootstrap `prime` → `PrimeReport` (nxf 6j6v.wph0), with its own
    `render_markdown`/`to_value` views, the `PrimeCommand` entries and the prose constants
    (`PRIME_CONTEXT_RECOVERY`, `PRIME_MEMORY_RULE`, `PRIME_NO_MEMORIES`, `PRIME_COMMANDS`), plus
    `render_memory_blocks` for a host composing its own context document.
- **`watch`** — change notification: `Change` (re-exported from the foundation).
- **`error`** — every consumer's error envelope (re-exported from the foundation): `ErrorKind`,
  `NxfError` (+ constructors), `Result<T>`.
- **`workspace`** — `.nxs/` resolution + memory store opening: `Workspace` (foundation) + the
  `MemoryWorkspaceExt` trait (`open_memory_store`), `memory_config`, `MEMORY_MODULE`, and the
  re-exported foundation `resolve`/`discover`/`setup`/`init`/`WorkspaceConfig`.

### 2.2 Items that show through (part of the contract)

These appear in the signatures above and are therefore part of the contract (an incompatible change
to them is a contract change per §3):

- `facade::MemoryRecord` — the canonical record value every read/write returns. **Its declared field
  order (`key`, `body`, `author`, `updated`, `active`) is part of the contract**: `serde_json`
  serializes it in declaration order (no `preserve_order` in the build graph) and the `nxm --json`
  goldens pin the exact bytes. A consumer parses by key, but the byte order must not drift.
- The foundation types `error::{ErrorKind, NxfError, Result}`, `watch::Change`, and
  `workspace::{Workspace, WorkspaceConfig}` — shared across every nxs product; their contract is the
  foundation's, surfaced here.
- `facade::PrimeReport` — the session bootstrap as data, and **the exact Markdown
  `render_markdown` produces is part of the contract**: it IS the SessionStart-hook block `nxs prime`
  fans out to and a host injects verbatim, so its bytes may not drift (pinned by
  `crates/memory/tests/prime_golden.rs`, and bound to the CLI by the parity differential). The prose
  constants it carries are contract for the same reason. `to_value` is additive-only.
- `key::auto_key` — the deterministic content-hash auto-key a `None` key resolves to (so a consumer
  can predict the minted key). The **rest** of `nexus-memory` (the `fact` reducer, the store, the
  `memories` view DDL, the op-log) is **not** part of the contract — it is implementation behind the
  facade.

## 3. Versioning & semver policy

**Version source.** A single version for all product crates, `[workspace.package] version`
(today `0.9.0`); the crates inherit it via `version.workspace = true`. It is identical to the `nxs`
release version, and the release tag `v<version>` is the truth (CI enforces tag == version
fail-closed, see `release-management.md` §4.1). **The memory-embedding contract shares exactly this
version number** — there is no separate API version (as for flow's E5).

**`publish = false` — what the contract still means.** `nexus-memory` is deliberately not published
to crates.io; a consumer links it as a path/git dependency. Classic crates.io semver therefore does
not apply mechanically. The contract is consequently a **deliberate, written-down policy
commitment**: changes to the contract surface (§2) are versioned and communicated **as if** semver
applied — so in-repo and later external consumers have a real stability guarantee.

**Meaning of the positions (`0.y.z`, pre-1.0).** As long as MAJOR is `0`, the pre-1.0 reading of
semver applies:

- **MINOR (`y`)** — may contain breaking changes to the contract surface (pre-1.0 right). Breaking =
  removed/renamed public item, changed signature/field set, **changed `MemoryRecord` field set or
  order**, changed `ErrorKind` mapping of a documented case, or a behavioral change that would break
  a conforming consumer. Every such change MUST appear in the changelog under `changed`/`removed`
  and (where possible) go through the deprecation path in §5.
- **PATCH (`z`)** — additive or fixing only: new public items, new optional fields behind defaults,
  bugfixes that restore the **documented** behavior. Never breaking.

**1.0 and after.** With `1.0.0`, the policy flips to strict semver. The transition is the explicit
commitment "this surface is stable"; it happens only once an external (non-in-repo) consumer relies
on it.

## 4. Stability tiers

| Tier | Meaning | Items |
|------|-----------|-------|
| **Stable** | Full contract (§3/§5). | The entire §2 surface, unless named below. |
| **Unstable / `#[doc(hidden)]`** | No contract; may break at any time (including in PATCH). Pure test/tooling seams. | `engine::Engine::open_with_poll_interval` (`#[doc(hidden)]` test seam for the watcher cadence). |
| **Tooling switch** | No app contract; env-driven, only for deterministic goldens/tests. | `NXM_NOW` (pins the `updated` write clock), `NXF_DETERMINISTIC_IDS` (pins the replica/site). |

`#[doc(hidden)]` items do not appear in the `rustdoc` and are **by definition** not part of the
contract surface; a consumer that calls them does so at its own risk.

## 5. Deprecation path

Before a stable item is removed or changed in a breaking way:

1. **Mark** with `#[deprecated(since = "<version>", note = "<replacement/rationale>")]` — the
   compiler warns every consumer at the call site.
2. **At least one MINOR cycle** of coexistence: the deprecated item stays functional, the
   replacement stands beside it.
3. **Changelog fragment** (`type: changed`) describes the migration in EN + DE.
4. **Remove** no earlier than the next MINOR after the marking; the removal is itself a breaking
   change (`type: removed`, §3).

Exception: an item that was never stable (tier "Unstable"/`#[doc(hidden)]`) may disappear without
this path.

## 6. Seam invariant (no new semantic layer)

The contract includes a **behavioral** invariant, not just type signatures: the embedding surface
adds **no** new semantics — the same `fact` reducer, the same keep-if-beats LWW + reversible
tombstone, the same canonical `MemoryRecord` as the `nxm` CLI (`--json`) and the MCP seam (E9). Three
seams — the CLI for agents, the embed API for apps, the MCP server for MCP-native hosts — one memory
core; they must never diverge. This is not mere docs but **tested**: the App↔CLI differential
(`crates/memory/tests/parity.rs`) pins byte-equal records AND byte-equal op-logs
over the same store/script. A contract change that would break this parity is forbidden — regardless
of the version position.

The invariant is about **omission** as much as divergence (nxf epic 6j6v.fjrc). A differential can
only compare verbs that exist on both seams, so a verb living on ONE seam passes it silently — which
is exactly what happened to `prime`: it sat as a private function in `cli.rs`, unreachable from the
library, until app-foundations rebuilt the same assembly in TypeScript. Since 6j6v.wph0 `prime` is a
facade verb with a differential case of its own, covering both the `--json` projection and the
rendered SessionStart block. **A new CLI verb that assembles anything belongs in the facade, and the
differential is what proves it.**

Since nxf `6j6v.vtvs` that last sentence is *enforced* rather than remembered:
`crates/memory/tests/verb_seam.rs` walks the `nxm` clap tree (hidden verbs included) against the
public symbols of `engine.rs` + `facade.rs` and fails on any verb with no counterpart and no recorded
reason. nxm's whole declared difference is three `CliOnly` entries — `init`, `agent-manifest` and
`import`, all host-facing — with no gaps. See E5 §6.1 for the waiver kinds and what each one has to
prove.

## 7. Explicitly NOT part of the contract

- **Crate-internal items** (`pub(crate)`/private): the foundation handle internals
  (`State`/`Inner`/`ensure_current`), `watch::Watcher`/`file_id`/`POLL_INTERVAL`.
- **The memory core beneath the facade**: the `fact` reducer, the store helpers, the `memories`
  materialized view DDL, the op-log. Only what shows through via §2.2 is stable.
- **The on-disk schema** (`.nxs/`): its own compatibility axis (forward-compat fold, E4 / platform
  foundation), not the library API.
- **Foreign-language bindings** (napi-rs/WASM/UniFFI): a deliberately later, separate epic; the
  first consumer is a Rust Tauri backend that links the crate directly.
- **`nxm` CLI flags & output**: their contract is the `--json` agent surface + the golden corpus,
  not this spec — even though both drive the same facade.

## 8. Anchoring & maintenance

This spec is the written anchoring of the contract in `docs/specs/`. On every change to the §2
surface: update §2 here, choose the version position per §3, follow §5, write the changelog — and
verify that the seam invariant (§6, differential `#3rx.3`) stays green. With that, the surface is a
**deliberate library contract**, not a state of affairs derived after the fact.
