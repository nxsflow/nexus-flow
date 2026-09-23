# Plugin custom fields — plugin-declared item fields: schema, merge policy, CLI/MCP exposure (design spec)

> Status: **approved** (owner approval 2026-07-12) — decomposed into T1–T4 under `6j6v.rvay` (§11). Epic `6j6v.rvay`
> ("Plugin-Custom-Fields"). Builds on the existing **per-field merge seam** (`w213`: the
> `MergeStrategy` vocabulary + `set_field_merge`, today present-but-dormant on the write path —
> §4) and the E1 op-log substrate (`docs/specs/E1-core-data-model.md`). Pattern mirrors the
> nexus-chat M2 spec (`docs/specs/nexus-chat-M2.md`): one bounded data-model addition, everything
> else derived/folded, wire-compat via the `is_foldable` store-don't-fold gate.
>
> **First consumers, waiting on this epic:** `6j6v.ky26` (the `r16h` consumer plugin: a `file` type
> with a custom `uri` field), and the nexflow.it data model (`person.email`, `project.status` /
> `project.deadline`, `context.color` / `context.mono`), whose §4 field work rides `labels` /
> `description` interim until this ships. Delivered through the foundation **Schema + Record +
> Parity** seam (`@nxsflow/engine-client`) so app-foundations passes the fields through unchanged.

## 1. Goal & scope

Today an item has exactly **16 canonical fields**, hardcoded in **five lockstep places** (core DDL
`ITEM_LWW_FIELDS` `crates/core/src/schema.rs:21`, `ItemRow` `crates/core/src/model.rs:140`, facade
`CanonicalItem` `crates/facade/src/record.rs:15`, the read schema `FIELD_SCHEMA`
`crates/facade/src/read.rs:48`, the write whitelist `UPDATABLE_FIELDS` `crates/facade/src/write.rs:69`).
A plugin can rename and re-rank those fields but cannot **add** one. Consumers that model real
domains — a file's `uri`, a person's `email`, a project's `status`/`deadline`, a context's
`color` — have nowhere to put them and currently overload `labels`/`description`.

This epic lets a **plugin declare its own fields** on top of the 16, each with a name, data type,
merge policy, and CLI/MCP/schema visibility — chosen deliberately **instead of** minting new
canonical fields (a canonical `uri` would burden every plugin and every consumer with a field only
one domain uses).

**In scope (this epic):**

1. **Field declaration** — a plugin declares custom fields in a new `[fields]` TOML table, parsed
   and validated at plugin load (§3).
2. **Value storage** — custom values fold into their **own** materialized view (`custom_fields`),
   an EAV table over the same op-log, leaving the 16 canonical columns / `ItemRow` / `CanonicalItem`
   **byte-identical** (§2). Sparse, exactly like `labels`/edges.
3. **Merge policy per field** — the op carries the `w213` strategy so convergence needs no config;
   this epic **activates the dormant `w213` seam on the write path** for custom fields (§4).
4. **CLI / MCP / schema exposure** — `nxf schema --json` lists the declared custom fields;
   `nxf create`/`update --set <name>=<v>` writes them (type-validated); `show`/`--json` returns them
   as a sparse `custom` map; MCP `flow_schema`/`flow_create`/`flow_update` inherit it byte-identically
   (§5, §6).
5. **Migration & wire-compatibility** — additive view + a flow view-schema bump + refold; an old
   binary **store-don't-folds** a custom-field op; foreign/undeclared fields are **carried, not
   shown** (§7).
6. **The consumer seam** — the declared fields cross the facade **Schema + Record** boundary so
   app-foundations / `@nxsflow/engine-client` surfaces them without a bespoke change (§8).

**Explicit non-goals (§9):** slice *implementation* (post-approval); **set-valued** (`or-set`)
custom fields (scalar-only v1 — no v1 consumer, and they need their own view+fold like `labels`);
richer scalar types beyond text/longtext/date/enum (int/bool/format-validated uri/email/color);
the `r16h`/`file` consumer plugin itself (`ky26`) and blob sync; a per-field permission model.

## 2. Data model — the 16 canonical, unchanged, plus a folded `custom_fields` view

The load-bearing decision (**TB-CF-1**): custom values do **not** become columns on `items`. They
fold into a **separate** materialized view, so the canonical item shape is untouched.

### 2.1 Why not dynamic columns

`ALTER TABLE items ADD COLUMN <custom>` per plugin field would (a) make the `items` table shape
**plugin-dependent**, breaking the differential-oracle byte-identity and the flow/memory goldens
that pin the canonical schema; (b) require per-plugin migration DDL; (c) put a plugin's vocabulary
into the meaning-free core table. Rejected.

### 2.2 The `custom_fields` view

A new flow-core view, materialized alongside `items`/`edge_adds`/`notes` (`crates/core/src/schema.rs`):

```
custom_fields(item_id TEXT, field TEXT, value TEXT,
              value_v INTEGER, value_site INTEGER,      -- the LWW Lamport/site pair
              PRIMARY KEY (item_id, field))
```

One row per `(item, custom field)` that has ever been set — **sparse** (no row ⇒ field unset). This
mirrors how `labels`/edges already live in their own views over the op-log rather than on `items`.
`value` is always stored as `TEXT` (like every canonical LWW cell — `schema.rs:53`); the declared
**type** governs validation and presentation, not storage.

### 2.3 Record exposure — a sparse `custom` map (preserves the 16-key parity)

On read, custom values surface under **one** additive, sparse key on the JSON record:

```json
{ "id": "…", "type": "file", "title": "…", …the 16 canonical keys…,
  "custom": { "uri": ".nxs/files/report.md" } }
```

`custom` is present **iff** the item has ≥1 custom value, exactly like the sparse `labels` key
(`read.rs:794`). `CanonicalItem`'s 16 fixed keys (`record.rs:15`, pinned by `record.rs:129`) stay
**byte-identical** — `custom` is a read-layer join appended after them, never a 17th canonical
field. This is what keeps the SemVer/parity contract (§8) intact.

## 3. Field declaration — the plugin `[fields]` table

A plugin declares custom fields in a new top-level TOML table, parsed + validated in
`load_registration` (`crates/facade/src/plugin/mod.rs:237`) next to `[merge]`/`[types]`. Shape
(modelled on the existing `[types.rules.<t>]` style):

```toml
[fields.uri]
type = "text"            # text | longtext | date | enum        (v1 set — TB-CF-3)
on   = ["file"]          # types this field applies to; omit ⇒ all types (TB-CF-2)
required = true          # must be set at create for an applicable type (default false)
label = "File URI"       # display label; default = titleised name

[fields.status]
type   = "enum"
values = ["backlog", "active", "done"]   # required iff type = "enum"
on     = ["project"]
merge  = "lww"           # lww (default) | crdt-text  — §4, TB-CF-5

[fields.email]
type = "text"
on   = ["person"]
```

**Fields are type-scoped (TB-CF-2).** `on` names the types a field applies to; omitting it makes the
field global. This matches every consumer's model — `person.email` is meaningless on a `project`,
`file.uri` on a `person`. `--set email=` on a non-`person` item is a validation error (§5); `schema`
annotates each custom field with its `on` set (§6).

**Load-time validation** (fail-closed, extends `types.validate()`/`validate_merge()`):

- **Name collision** — a custom field name must not equal any canonical field or alias (`title`,
  `description`, `status`, `priority`, `due`, `defer`/`defer_until`, `assignee`, `type`,
  `parent`/`belongs_to`, `design`, `dod`/`completion_criterion`, `closing_comment`, `deleted`,
  `archived`, `closed_at`, `labels`, `notes`). Loud error at load (TB-CF-6).
- **Charset** — `^[a-z][a-z0-9_]*$` (safe as a `--set` key and a JSON key).
- **`on` types** — every listed type must be in `[types].list`.
- **`enum`** — `values` non-empty and required; forbidden for non-enum types.
- **`merge`** — `lww` or `crdt-text` only in v1 (`or-set` is rejected — TB-CF-4); reuses the
  existing `MergeStrategy` vocabulary (`plugin/mod.rs:60`).

## 4. CRDT / op semantics — the `field set` op and the (now-live) `w213` merge

Custom fields fold through a **new, self-describing op shape** so the meaning-free core folds them
with **no** knowledge of any plugin's vocabulary.

### 4.1 The op

| `target_kind` | `op_type` | `target_id` | `field` | `value` |
|---|---|---|---|---|
| `field` | `set` (LWW) or `set:crdt-text` | `<item id>` | `<custom name>` | value string, or empty to clear |

**Naming (T1 note):** `target_kind = "field"` is the op's *target category* (the analogue of the
existing `item`/`edge`/`note`/`label` kinds) — it is deliberately distinct from the op's existing
`field` **column**, which here holds the custom field's *name* (`uri`, `email`, …). The pair reads
as "a `set` on the `field`-kind target whose `field` is `uri`". If that overload proves confusing in
the T1 implementation, name the kind `custom` instead (`("custom", set*)`) — a pure wire-label choice
with no semantic effect on the structural fold below.

A `field set` op is **foldable purely structurally** — the reducer folds *any* `("field", set*)` op
into `custom_fields` by `(item_id, field)` keep-if-beats LWW, identical in shape to `fold_item_lww`
(`crates/core/src/task_reducer.rs:54`) but keyed on the pair. It needs no plugin config, so a custom
field from *another* plugin/version still folds and syncs (§7); the **plugin** decides only what may
be *written* (§5) and what *surfaces* (§6). `is_foldable` (`task_reducer.rs:144`) gains the
`("field", "set" | "set:crdt-text")` arm; `fold()` (`:163`) routes it to a new `fold_field`.

### 4.2 Merge policy per field type — activating `w213`

The op's `op_type` **carries the strategy** (`set` vs `set:crdt-text` — the exact `w213` encoding
`item_set_op_type`/`from_item_set_op_type`, `crates/core/src/model.rs:90`), so convergence is a pure
function of the op with no sender-side config (TB-CF-5). In v1 **both** fold as whole-value
keep-if-beats LWW registers — matching how the canonical prose bodies already converge as
LWW-longtext (the char-level body backend was cancelled 2026-06-16; CLAUDE.md). `crdt-text` is thus a
**forward-compatible marker**: a declared `longtext` custom field records `set:crdt-text` ops today
(folding LWW) and rides a real char-level merge for free if `4b39` ever splits `fold_item`.

Crucially, the facade write path today calls **bare** `set_field` and never `set_field_merge`
(`write.rs:651` — the `w213` seam is dormant). Custom fields are the **first** writer to go through
the strategy-tagged path (`store.set_field_merge`, `store.rs:185`), so this epic finally activates
`w213` end-to-end — for custom fields; the canonical fields stay on the bare LWW path (no behavior
change, no re-fold of existing data).

**Clearing** a custom field is a `field set` with an empty value (an LWW tombstone-by-value on a
scalar register); the row remains with an empty `value` and reads as unset in the `custom` map.

## 5. Write surface — `create`/`update --set`, type-validated

`--set <name>=<value>` stays the single write verb; the change is entirely inside `prepare_set`
(`crates/facade/src/write.rs:417`), after the canonical-alias check:

1. If `name` is canonical/aliased → unchanged path (bare `set_field` LWW).
2. Else if `name` is a **declared custom field** of the active plugin → **new path**: validate the
   item's `type` is in the field's `on` set; validate `value` against the declared type (`date` →
   ISO-8601, reusing the `due`/`defer` check; `enum` → member of `values`; `text`/`longtext` → any
   UTF-8); emit a merge-tagged `field set` op via `set_field_merge`.
3. Else (undeclared) → the existing **loud `validation` error**, now listing canonical + declared
   custom names (`write.rs:418`).

`create` (`write.rs:495`) accepts the same `--set` custom fields and additionally **enforces
`required`**: creating an item whose type has a `required` custom field without supplying it is a
`validation` error naming the field. MCP `flow_create`/`flow_update` pass `--set` straight through
(`crates/nxs/src/mcp/flow.rs`), so they inherit all of the above with no adapter change.

**`required` is a create-time guard only (v1; TB-CF-7, PR-review Integrity #1).** It guarantees the
field is set when an item of an applicable type is *born*; it is **not** re-checked on `update` or on
a type change. So `update --set uri=` (empty) *clears* a required custom field, and retyping an item
into a type that requires a field it lacks leaves it unset — both leave the field empty rather than
erroring, consistent with the LWW-clear semantics of every other field (an item can always be edited
into an incomplete state; the engine records, it does not gate mutation). Stricter enforcement —
`update` rejecting the clear of a required field, and retype re-validating required fields for the
new type — is a **deliberate v1 non-goal** (§9): it needs an update/retype-side validation pass with
no v1 consumer, and `file.uri` is satisfied by the create guard. T3 (`6j6v.p7b2`) implements the
create guard only; a presentation layer that wants "this file is missing its uri" derives it from the
sparse `custom` map, not from a write-time reject.

## 6. Read & schema surface

**`nxf schema --json` / `flow_schema`** (`crates/facade/src/read.rs:322`, one source for CLI + MCP):
the declared custom fields are appended to the existing `fields[]` array, each marked to distinguish
it from a canonical field and carrying its declaration:

```json
{ "field": "uri", "kind": "text", "custom": true, "on": ["file"],
  "required": true, "settable": true, "label": "File URI" }
```

Canonical entries are unchanged (no `custom` key), so existing `schema` consumers are unaffected;
the `FIELD_SCHEMA` const (`read.rs:48`) stays the canonical source and the custom entries are merged
in from `cfg`.

**Item reads** (`show`, `list`, `next`, `prime`): the sparse `custom` map (§2.3) is attached as a
read-layer join. For the lane views it is a **single bulk read** — `custom_fields_of_bulk(&ids)`,
mirroring `labels_of_bulk` (`read.rs:789`) — so `next`/`list` gain **no N+1** (directly relevant to
the `xn8s` read-cost work). `show` prints custom fields under their labels, after the canonical
block, using the same `[vocabulary.fields]` label mechanism.

## 7. Migration, wire-compatibility & multi-plugin coexistence

**Additive, old-binary-safe.** The one schema change is the new `custom_fields` view — a
foundation-additive migration that **bumps the flow view-schema version** so an existing workspace
materialises the view and **refolds** (`store.rs:119`) any `field set` ops that stored-don't-folded
under an older binary.

**Forward-compat is already in the fold gate.** An old binary that meets a `("field", set)` op does
**not** know the `target_kind` → `is_foldable` returns false → it **store-don't-folds** the op: the
op stays in the `ops` log (the source of truth, `foundation/src/schema.rs:262`), is neither dropped
nor a panic, and folds once the binary understands `field` ops. This is the identical guarantee M2
relies on for `thread set deadline`.

**Canonical items stay byte-identical.** Because custom values live in their own view (§2), `items` /
`ItemRow` / `CanonicalItem` and every canonical `--json` are unchanged — the differential oracle and
the flow/memory trycmd goldens remain green with no update.

**Foreign / undeclared fields are carried, not shown.** A `field set` op whose name the active plugin
does **not** declare still folds into `custom_fields` and syncs; it simply does not surface in
`schema` or the `custom` map for this plugin. Switching plugins (or a later declaration) re-surfaces
it — the op-log is never lossy. This is what lets two consumers with different field sets share one
synced substrate.

## 8. Consumer seam — foundation Schema + Record + Parity

The fields cross the facade boundary (the SemVer contract, `crates/facade/src/lib.rs:25`) through
the same trio every field already uses:

- **Schema** — `read::SchemaReport` (§6) gains the custom `fields[]` entries; `@nxsflow/engine-client`
  reads them to build its form/column model.
- **Record** — the sparse `custom` map on the item record (§2.3) crosses as an additive join; the
  16-key `CanonicalItem` **JSON** is untouched (the differential-oracle / golden byte-identity holds,
  and no `pub` item is removed/renamed/retyped). **Correction (implementation, ky26 review):** at the
  *Rust* level the epic adds `pub` fields to externally-constructible structs (`SchemaField`,
  `NewItem`, `PluginConfig`), which `cargo-semver-checks` classifies as `constructible_struct_adds_field`
  — a real break. So this is a **SemVer-minor** on the **break axis** and ships flagged
  **`facade: breaking`** (as the delivered `changes/` fragment was), NOT the `facade: changed`
  originally written here. Public *fn* signatures stayed stable — the read-side join is delivered via
  additive `*_with_custom` wrappers rather than retyping existing functions.
- **Parity** — `nxf <cmd> --json` and the `flow_*` MCP `structuredContent` stay byte-identical
  because both render from the same `read::schema` / record path (`crates/cli/tests/parity.rs`), so
  the custom fields appear identically on both surfaces with no extra wiring.

app-foundations therefore passes the declared fields through as `Schema + Record` without a bespoke
per-field change — the generic seam the epic is designed to feed.

## 9. Deliberate non-goals (v1)

- **Slice implementation.** This spec + the §11 T-slice decomposition is the deliverable; slices
  land after approval.
- **`update`/retype-side `required` enforcement.** `required` is a create-time guard only (§5); v1
  does not reject clearing a required field on `update`, nor re-validate required fields on a type
  change. Adding that needs an update/retype validation pass with no v1 consumer — deferred.
- **Set-valued (`or-set`) custom fields.** Scalar-only in v1. A multi-value custom field needs its
  own observed-remove view + fold (like `labels`/edges); no v1 consumer needs it, so it is deferred
  rather than half-built (TB-CF-4).
- **Richer scalar types.** `int`, `bool`, and format-validated `uri`/`email`/`color` are out; v1 is
  `text`/`longtext`/`date`/`enum` (a hex color or uri is `text`, a fixed palette is `enum`) —
  TB-CF-3. Adding a type later is additive.
- **The `r16h` / `file` consumer plugin itself** (`ky26`) and **blob sync** — this epic ships the
  *mechanism*; `file`+`uri` is its first *user*, tracked separately.
- **A per-field permission / audit model** — custom fields obey the same actor model as canonical
  fields; nothing finer in v1.

## 10. Open points / tie-breaker log — **RESOLVED (owner approval 2026-07-12)**

All eight decisions were approved as proposed.

| ID | Question | Resolution |
|---|---|---|
| **TB-CF-1** | Where do custom values live — dynamic `items` columns or a separate view? | **Separate folded `custom_fields` EAV view** (§2). Keeps `items`/`ItemRow`/`CanonicalItem` byte-identical (oracle + goldens green, SemVer-minor), sparse, and mirrors `labels`/edges. Dynamic columns would make the core table plugin-dependent. |
| **TB-CF-2** | Are fields global or type-scoped? | **Type-scoped** via `on = […]` (default: all types). Matches every consumer (`person.email`, `project.status`, `file.uri`); `--set` on a non-applicable type errors. |
| **TB-CF-3** | Which data types in v1? | **`text` \| `longtext` \| `date` \| `enum`.** Covers all named consumers (uri/email/color = `text`; a fixed palette/status = `enum`; deadline = `date`). `int`/`bool`/format-validated types are additive follow-ups. |
| **TB-CF-4** | Multi-value (list) custom fields in v1? | **No — scalar only.** `or-set` custom fields need their own view+fold and have no v1 consumer; deferred (§9). |
| **TB-CF-5** | Merge policy & the `w213` seam? | **All v1 custom fields fold keep-if-beats LWW**, with the op carrying the `w213` strategy marker (`set`/`set:crdt-text`) so a real char-level merge is a future drop-in. This epic is the **first** writer to activate the dormant `w213` `set_field_merge` path; canonical fields are untouched. |
| **TB-CF-6** | Write namespace — flat `--set uri=` or a `custom.`/`x-` prefix? | **Flat `--set <name>=` with a load-time collision guard** vs the canonical set; values read back under a sparse `custom{}` map (so `CanonicalItem`'s 16 keys stay parity-stable). Best agent-ergonomics; a future canonical-field collision is caught loudly at plugin load. |
| **TB-CF-7** | `required` custom fields? | **Supported** (default false), enforced at **`create` only** for applicable types — cheap, reuses the create-validation path, and `file.uri` wants it. `update`/retype do NOT re-enforce it in v1 (clearing is allowed, LWW-consistent); stricter enforcement is a §9 non-goal (PR-review Integrity #1). |
| **TB-CF-8** | Foreign / undeclared field ops? | **Carried, not shown** (§7): they fold + sync but surface only when a plugin declaring the name is active. No data loss; enables mixed-consumer sync. |

## 11. Mapping to implementation tickets (created under `6j6v.rvay`, approved 2026-07-12)

Board tickets: **T1 `6j6v.sfyy`**, **T2 `6j6v.pgp4`**, **T3 `6j6v.p7b2`**, **T4 `6j6v.ekf5`**. T1 and
T2 are independent (both ready); T3 depends on T1+T2; T4 on T1+T2+T3. `6j6v.ky26` (the `file`+`uri`
consumer) depends on T4.

| Ticket | Delivers | This spec |
|---|---|---|
| **T1 `6j6v.sfyy` — core fold & view** | The `custom_fields` view + the `("field", set/set:crdt-text)` op: `is_foldable` arm + `fold_field` keep-if-beats LWW keyed on `(item_id, field)`; the flow view-schema bump + refold; convergence/robustness tests (reorder-independence; store-don't-fold under the old gate; canonical `items`/oracle byte-identity; foreign-field carry). Builds on the `w213` merge machinery. | §2, §4, §7 |
| **T2 `6j6v.pgp4` — plugin declaration** | Parse + validate the `[fields]` table in `load_registration` (type/`values`/`on`/`required`/`label`; collision, charset, `on`-type, enum, and merge validation). | §3 |
| **T3 `6j6v.p7b2` — write path** | `prepare_set`/`create` accept declared custom fields (type + `on` + `required` validation) and emit merge-tagged `field set` ops via `set_field_merge`; undeclared-name error lists custom names. | §4, §5 |
| **T4 `6j6v.ekf5` — read / schema / record / seam** | Custom fields in `read::schema` `fields[]`; the sparse `custom` map on the record + `custom_fields_of_bulk` (no N+1); `show`/`list`/`next --json`; MCP `flow_schema`/`flow_create`/`flow_update` parity; the facade Schema+Record seam. | §5, §6, §8 |

End-to-end acceptance (a real consumer: declare `file`+`uri`, `create --type file --set uri=…`,
`schema`/`show`/sync round-trip) is the **`6j6v.ky26`** consumer ticket, now wired `depends-on` T4
(`6j6v.ekf5`) — not part of this spec (pattern: M2's `cdy7` follow-up dogfood).

## 12. The `file` consumer (`6j6v.ky26`) — the `.nxs/files/` convention + the v1 blob boundary

`ky26` is the FIRST real user of this epic: a `file` item type with a required custom `uri` field,
proven end-to-end. It is a **consumer dogfood**, not new shipped surface — the fixture plugin lives
in the test-only `plugin-probe` crate (`src/file-fixture.toml`, registered from `src/lib.rs`) and is
exercised in-process by `crates/plugin-probe/tests/ky26_file_consumer.rs` (a subprocess `nxf` cannot
see a test-only plugin). The three bundled/CLI plugins are unchanged; nothing new ships to `nxf`.

**The `.nxs/files/` convention.** A `file` item *links* a created file:

- `title` — the filename (e.g. `report.md`);
- `description` — a human summary of the file;
- `uri` (the custom field) — a **workspace-relative** path under `<workspace>/.nxs/files/`
  (subfolders are free, e.g. `.nxs/files/reports/q3.md`). Relative, so the item stays valid across
  machines once the blob is present. `uri` is `type = "text"`, `on = ["file"]`, `required = true`.

**V1 blob boundary (explicit, load-bearing).** The op-log syncs the file **items** — their metadata
(`uri`, title, description) — via the relay, and a peer's replica converges on the SAME `uri` custom
value (the "sync the metadata" guarantee the ky26 sync round-trip test pins: replica A creates + sets
`uri`, `export()`s; replica B `apply()`s and its read surfaces the identical `uri`). The **blobs
themselves** (the bytes under `.nxs/files/`) are **NOT synced in v1** — blob transport is a later
engine theme (E4-neighbourhood, §9). A `file` item can therefore point at a blob a given replica
does not yet hold. This boundary is the point of the ky26 dogfood: it proves the *metadata* path
end-to-end without over-reaching into blob transport.
