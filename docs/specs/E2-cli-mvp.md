# E2 — Agent-CLI MVP (local-first, declarative plugin seam) (Design Spec)

> Status: Design · As of: 2026-06-09
> Predecessor: E1 core data model (`docs/specs/E1-core-data-model.md`, merged `ef23980`).
> This is the **primary agent interface**: a thin, deterministic CLI over the
> opinion-free core, with the first — initially declarative — plugin seam.

## 1. Goal & Scope

E2 makes the E1 core **usable by agents**: a local-first/offline CLI (`nxf`) whose
bar is *to be at least as good to drive for a coding agent as `bd`* — `bd` being the proven
agent-CLI bar to clear. An agent can run a complete session exclusively with `nxf`
(prime → ready → claim → update → close), without beads, all offline, `--json` deterministic.

The plugin seam is present from day one, but **declarative** (config-driven): `issue-tracker` is the
default config, a minimal `personal-todo` sits beside it as a **second real artifact** (the
genuine seam proof, §4). No command code knows issue-tracker vocabulary hardcoded. The later
**programmatic** plugin layer is deliberately carved out of E2 (its own issue) and will
consume the same config, not replace it.

**In E2:**
- New crate `crates/cli` (`nexus-flow-cli`), binary `nxf`, over `nexus-flow-core`.
- Workspace discovery + persisted replica identity (`.nexusflow/`).
- Commands: `init`, `prime`, `create`, `list`, `ready`, `blocked`, `next`, `show`, `update`,
  `claim`, `close`, `dep add/remove`, `note add/list`, `search` — `--json` everywhere, deterministic.
- **Read AND write body** (`nxf update --body=…`) — full-replace interface, backend
  for now LWW (see §4a). This preserves parity with `bd update --description`; not read-only.
- Declarative plugin config (vocabulary, ranking override, presentation, priority validation).
- `prime` command + thin optional SessionStart hook wrapper.

**Not in E2 (deferred):**
- yrs/Yjs body **backend** (character-precise CRDT merge, web-Yjs interop) → a later slice. E2
  already delivers the body *interface* (full-replace) in full; that slice only swaps the backend
  LWW→Yrs **without changing the CLI signature** (§4a). Not a retrofit, but the same staircase.
- Programmatic plugin API (trait/module that also extends the config programmatically) →
  **its own issue outside E2**.
- Sync server, realtime, multi-device transport → **E4**.

## 2. Architecture

Three layers, cleanly separated:

1. **core** (`nexus-flow-core`, existing) — opinion-free substrate + pure derivation. E2 does not
   change the core (at most additive, backward-compatible helpers).
2. **Plugin config** — declarative TOML, two embedded (`issue-tracker` default + `personal-todo`,
   §4). Provides vocabulary, ranking override, presentation columns/fields, priority range. Loader +
   typed deserialization.
3. **Command layer** — maps CLI commands onto core ops, renders results **through the active
   config**. Contains no issue-tracker semantics in code.

### Crate layout

```
crates/cli/
  Cargo.toml          # bin "nxf"; deps: nexus-flow-core, clap, serde, toml, rusqlite (re-export)
  src/
    main.rs           # clap entry, dispatch, global flags (--json, --db), exit codes
    workspace.rs      # .nexusflow discovery (search upward), replica.toml, open store
    plugin/
      mod.rs          # PluginConfig types, loader, embedded configs (issue-tracker, personal-todo)
      ranking.rs      # declarative ranking → SQL/sort over ready candidates
    commands/
      init.rs prime.rs create.rs list.rs ready.rs blocked.rs
      show.rs update.rs claim.rs close.rs dep.rs note.rs
    output.rs         # JsonValue/human rendering, determinism guarantees, error envelope
  tests/              # integration via assert_cmd + tempfile (one workspace per test)
```

The workspace `Cargo.toml` gains `crates/cli` as its second member.

## 3. Workspace & replica identity

- `nxf init` creates `.nexusflow/` in the current directory:
  - `db.sqlite` — the persistent store (`Store::open(path, site)`).
  - `replica.toml` — `site_id: i64` (random, stable) + `prefix: String` (short, for the ID scheme
    from `id::ReplicaIds`). Generated once at init, immutable thereafter.
  - `config.toml` — workspace settings, including `plugin = "issue-tracker"` (selection of one of
    the embedded configs, §4; default `issue-tracker`). Separate from `replica.toml` because
    plugin choice is workspace policy, not replica identity.
- All other commands search for `.nexusflow/` **upward** (git-style: cwd → parent → … → root).
  If absent, a clear error (`{"error":"no .nexusflow workspace found; run `nxf init`"}`).
- Override: `--db <path>` / `NXF_DB` env for explicit paths (tests, CI, multiple workspaces).
- A unique site_id per clone/device ⇒ exactly what OR-set `add-tag` and LWW merge need in E4.
  E2 uses it locally; E4 builds on it.
- **Concurrent access:** agents fire commands in parallel ⇒ multiple `nxf` processes write
  against `db.sqlite`. SQLite runs in **WAL mode** with a set `busy_timeout` (e.g. 5 s);
  every mutating command is a short transaction (append op + fold). This serializes parallel
  writers instead of "database is locked".

## 4. Declarative plugin seam (§ core risk)

An embedded TOML config is the source of all opinionated decisions. Schema
(rough form, finalized during implementation):

```toml
name = "issue-tracker"

[priority]
labels = ["P0", "P1", "P2", "P3", "P4"]   # display
min = 0
max = 4                                    # loud rejection outside [min,max]

[vocabulary.status]
open        = "open"
in_progress = "in progress"
closed      = "closed"

[vocabulary.types]
project = "project"
epic    = "epic"
task    = "task"

[ranking.next]
# overrides core::next default; orders ONLY the ready candidates, does NOT define readiness
order = [
  { field = "priority", dir = "asc" },
  { field = "due",      dir = "asc", nulls = "last" },
  { field = "id",       dir = "asc" },
]

[presentation.list]
columns = ["id", "priority", "status", "title"]

[presentation.show]
fields = ["id", "type", "status", "priority", "due", "defer", "assignee", "belongs_to"]
```

**Two embedded configs — real artifacts, not test-only.** E2 ships *two* embedded
plugin configs in the binary: `issue-tracker` (default) and a minimal `personal-todo` (pure
vocabulary remap: "task"→"todo", status labels, different priority labels — **no** divergent
behavior). Selection via a `plugin = "<name>"` field in `.nexusflow/` (default `issue-tracker`);
**no** external file registry/dynamic loading (that stays OUT, §10). This proves the seam is
**"in the binary and green"**, not merely via a test config written to make a test pass —
`personal-todo` is exactly the artifact that exposes whether the seam genuinely leaks. The
`id`-sorted JSON form stays identical across both configs (§7); only presentation/vocabulary/
ranking differ.

**Seam boundary (invariant):**
- `core::ready` / `core::blocked` remain the **structural truth** — no plugin changes
  what ready/blocked *means* (open, no open blocker, not in a cycle, defer due).
- The plugin supplies only: **ordering** of `next` (overrides the core::next default),
  **presentation** (which columns/fields), **vocabulary** (label mapping), **priority validation**.
- `nxf next` = the `core::ready` set, ordered by the config's `ranking.next`. The final
  `id` tiebreaker is **deterministic but semantically arbitrary** (replica prefix + ULID) — it
  guarantees stable ordering for the gate, but is not a *meaningful* ranking; meaningful secondary
  ordering must come via `ranking.next` fields before the tiebreaker.
- No command code references "issue"/"priority"/ranking directly; everything flows through the config.

This makes E3 **not a retrofit**: the programmatic layer (its own issue) receives the same
`PluginConfig` and may additionally override it programmatically.

## 4a. Body contract (diff bridge) — invariant across LWW→Yrs

Body editing is designed so that the **agent contract stays unchanged across the backend swap
(E2-LWW → ra0-Yrs)**. This is an architecture principle, not an E2 feature:

- **Interface = full-replace (invariant).** The agent always passes "here is the whole new
  body" (`nxf update <id> --body=<text>` or `--body-file=-` from stdin). That is its normal case —
  it regenerates prose, it does not edit characters. **This signature never changes across LWW→Yrs.**
- **E2 backend = LWW, no diff (YAGNI).** E2 computes **no** diff: a full-replace is exactly
  *one* `set_field("body", <text>)` op. The read/diff step would be dead code as long as the backend
  is LWW — so we do not build it now. Correct and sufficient for a local session.
- **ra0 backend = Yrs text, this is where the diff arrives.** Only the backend swap introduces the
  diff translation: ra0 reads the current body, diffs the new one via LCS/Myers **atomically**
  (read→diff→apply in *one* snapshot at write time) and translates into Yrs
  `insert`/`delete`. **Same CLI signature, no new verb.** The contract from E2 holds; only the
  implementation behind the `set_field` equivalent is replaced.

**What E2 owes for this (so ra0 is a clean swap, not a rebuild):** the body mutation runs
in E2 over *one* narrow internal path (e.g. `write_body(id, full_text)` in `commands/update`),
not scattered through `set_field` directly. ra0 replaces only the internals of this path
(diff + Yrs) — the CLI layer and the tests over it stay untouched. That is the concrete
seam §4a promises. Exactly this diff-bridge core + the two
honesties below are to be spelled out as design (currently filed as an issue note).

Two honesties (apply only with the Yrs backend, belong in `ra0`'s design, noted here in advance):

- The diff shrinks the conflict blast radius from *total* (LWW overwrites everything) to *minimal*
  (only touched regions) — it does **not** eliminate conflicts. If two replicas rewrite the same
  paragraph from a stale base, Yrs converges deterministically, but semantically the winner is
  last-at-the-spot. Do **not** sell this as "lossless merge".
- The **Yrs update** is the convergence truth (op-log); a readable **diff summary**
  ("body +12/−3") is *presentation/history* and must **never** enter the sync path. Two artifacts,
  cleanly separated.

## 4b. Escaping-free long-text input (Epic 95d)

The `--body-file=-` seam promised in §4a is implemented — and generalized over **all** long-text
fields (`description`, `design`, `completion_criterion`/DoD, `closing_comment`), after
`#25i` replaced the generic `body` with this structured field model. Agents
regenerate prose with backticks, quotation marks, `!` and multiple lines; in the argv string that
is shell-fragile. Three escaping-free sources, resolved at **one** common seam
(`crates/foundation/src/text_input.rs`) — field *validation* stays in the single facade write path
(`crates/facade/src/write.rs`), this layer only does source → string:

> **Moved 2026-09-11 (nxf 6j6v.s46h).** The seam was `crates/cli/src/field_input.rs`, a private
> module of `nxf`, until `nxc send`/`nxc reply` needed the same notation for a message **body** —
> which is not a field of anything, and `nexus-chat` may not depend on flow (spec §4.4). It is
> `nxs_foundation::text_input` now, beside the error type its contract is written in, and both
> CLIs call it. Nothing about the three sources or their contract changed in the move.

1. **STDIN `-` sentinel** (95d.1): a flag value of `-` reads this field from STDIN —
   `cat design.md | nxf create … --design -`, `nxf update <id> --set description=-`,
   `nxf close <id> --reason -`, `nxf note add <id> -`. STDIN is stored **verbatim**
   (no trailing-newline trimming).
2. **File flags** (95d.2): `--description-file` / `--design-file` / `--dod-file` (create),
   `--reason-file` (close), `--set-file field=path` (update). UTF-8 enforced, verbatim; a
   path of `-` is likewise the STDIN sentinel. Unlike STDIN, **multiple** fields may each come
   from their own file in one invocation.
3. **JSON payload over STDIN** (95d.3): `<json> | nxf create --json -` or
   `nxf update <id> --json -` writes multiple fields as **one** piped object. `create` is
   strict (`deny_unknown_fields`, required fields); `update` maps `field→value`. Both run through
   the **same** `write::create`/`write::update` validation path — no second logic. The `-`
   is a positional; `--json` still controls only the output.

**Cross-cutting rules (at the resolver, not per variant):** exactly **one** source per field
(inline | `-` | file | JSON), multiple assignment is a `validation` error; **at most one**
STDIN reader per invocation (a second `-` ⇒ error); every rejection fires **before** the first write
(no partial write). Deliberately **no** `$EDITOR` path as an agent interface.
Determinism + `--json` output as everywhere.

## 5. Command surface (MVP)

Command names deliberately modeled on `bd` (agents that know `bd` — the proven agent-CLI bar —
transfer directly). `--json` on all; without `--json` a human-readable table/detail view from the
same data source.

| Command | Effect (core op) |
|---|---|
| `nxf init` | Create workspace + `replica.toml` + `config.toml` |
| `nxf prime [--json]` | Workflow rules + ready/blocked snapshot + command reference |
| `nxf create --type --title [--priority --due --defer --parent]` | `create_item` (+ `set_field`, + belongs-to edge with `--parent`) |
| `nxf list [--status --type]` | filtered item list, columns from `presentation.list` |
| `nxf ready` | `derive::ready` |
| `nxf blocked` | `derive::blocked` |
| `nxf next` | `derive::ready`, ordered by `ranking.next` |
| `nxf show <id>` | item detail (`presentation.show`) + deps + notes + body |
| `nxf update <id> --<field>=<val> … [--body=… \| --body-file=-]` | `set_field` per field; body via diff bridge (§4a); validation see below |
| `nxf claim <id> [--assignee]` | sugar: status=in_progress (+ assignee) |
| `nxf close <id> [--reason]` | sugar: status=closed (+ closing_comment) |
| `nxf dep add/remove <from> <to>` | `add_edge`/`remove_edge` (kind=dep) |
| `nxf note add <id> <text>` / `note list <id>` | `add_note` / `notes_of` |
| `nxf search <query> [--status --type]` | substring/token search over title/body/notes, deterministic by `id` |

`search` is in the MVP, **not** optional: agents navigate large graphs through it (parity with
`bd search`, §8). `stats` (situational overview) is nice-to-have and deliberately left out (§10).

**`update` validation (beyond priority):** the allowed field set is explicit
(`title`, `status`, `priority`, `due`, `defer`, `assignee`, `body`, `closing_comment`).
`status` is checked against `vocabulary.status`; `priority` against `[priority].min/max`;
`due`/`defer` are parsed as ISO-8601 dates (loud rejection on garbage); `type` is
**immutable** (not an update field). Every violation → exit≠0 + error envelope (§7).

**`dep add` loudly rejects cycles at write time.** If `nxf dep add A B` would
create a cycle (B already reaches A over `dep` edges), the edge is **not** written — an error
with `kind="cycle"` (§7). Derivation *could* compute the cycle away later (`ready`/`blocked`
exclude cyclic nodes, `invariant::cyclic_nodes` reports them), but for agent ergonomics
loud rejection at the source is far better than a silently accepted, invalid edge that
derivation papers over afterward. The check runs against the materialized edge projection at
write time (analogous to priority/date validation).

**`search` runs against the materialized projection** (`items`/`notes` views), **not** against
the CRDT op-log/update. In E2 (LWW plaintext) this is trivial; with ra0 (Yrs body) it stays
correct because the search is against the *materialized* Markdown, not against Yrs internals.

## 6. `prime` & bootstrap

`nxf prime [--json]` is the `bd prime` equivalent and emits:
- Brief workflow rules (claim before work, close with a reason, --json everywhere).
- The current `ready`/`blocked` snapshot (so the agent immediately sees entry work).
- A compact command reference.

The **SessionStart hook** is just a thin wrapper that calls `nxf prime` and passes its output
through — optional and host-specific (Claude Code etc.). The truth lives in the command, which is
therefore deterministically testable; the hook itself carries no logic.

*Note (not to solve in E2):* the workflow rules in `prime` ("claim before work", "close with a
reason") are strictly speaking **issue-tracker policy**, not core truth. With one config in
E2 acceptable to hardcode; cleaner they would later come from the `PluginConfig`. Noted for E3/u2d.

## 7. Determinism & output

- **`--json` = canonical record, presentation-independent.** `--json` emits *all*
  core fields of an item in stable order — **independent** of `presentation.*`. The
  `presentation.list/.show` config controls **only** the human-readable table/
  detail view (which columns/fields are visible). This means a plugin switch (E3)
  **never** changes the agent contract, and the golden tests hang on the core record, not on the
  default config.
- Stable field order (serde struct order), arrays sorted by `id`, identical
  output on repeating the same input. **JSON is the tested contract.**
- **Error envelope structured, not flat.** In `--json` mode:
  `{"error": {"kind": "<machine-readable>", "msg": "<human>"}}` on stdout, exit code ≠ 0; otherwise
  plain text on stderr. `kind` is a closed set: `no_workspace`, `not_found`,
  `validation`, `cycle`, `conflict`, `io`. So the agent **never** has to match the `msg` string to
  distinguish "not found" from "invalid" from "no workspace" — cheap and aimed precisely at the
  agent-ergonomics bar.
- Mutating commands return the affected object (or `{"ok":true,"id":…}`), so the
  agent can continue without a follow-up query.

## 8. Testing (TDD)

Test-first for all behavior; scaffolding (Cargo.toml, clap skeleton) excepted.

- **Per command:** JSON contract test (golden on the canonical record, §7), determinism test
  (run twice ⇒ byte-equal), error path (missing workspace, unknown ID, invalid
  priority/status/date).
- **Seam proof (against a real artifact):** the config-swap test switches to the **embedded
  `personal-todo`** (not a test config written specially to pass) and checks that
  vocabulary/ranking/presentation change **without** touching command code — *and*
  that the `--json` form (canonical record) stays **identical** across both configs (§7). "In the
  binary and green" is the proof of "no hardcoding / no retrofit".
- **Cross-replica determinism:** two workspaces with different site_id, the same ops
  via `export`/`apply`, identical derived output.
- **Body — convergence, NOT byte equality:** the determinism byte-golden test does **not** apply
  naively to the body. Two replicas with "the same" full-replace produce, under Yrs (ra0),
  *different* updates (different client-ids/origins) — correct and expected. The
  body test is therefore **"both op orderings → same materialized Markdown"**, a
  test class of its own. In E2 (LWW) the body set is byte-stable; the separation is nonetheless
  established now, so ra0 does not inherit a wrongly dimensioned golden test.
- **Parity gate "≥ bd":** an explicit `nxf`↔`bd` parity table (`prime`/`ready`/`blocked`/
  `show`/`update`/`claim`/`close`/`dep`/`search` ↔ bd equivalents) as acceptance, plus a
  **dogfood criterion**: a real working session (create issue → claim → write body →
  close) runs entirely over `nxf`, without falling back to `bd`. Only then is E2 considered
  "≥ bd" proven rather than asserted — `bd` being the proven agent-CLI bar to clear.
- Integration via `assert_cmd` + `tempfile`; every test a fresh workspace.

**Known limitation (product agnosticism):** E2 ships the two reference configs of the vision
(`issue-tracker` + `personal-todo`) as real binary artifacts — the seam proof is thus *no longer*
test-only. Open for later: external/dynamic plugin loading (registry) and the
*programmatic* plugin API. `personal-todo` is deliberately a pure
vocabulary remap with no behavior of its own; a fully built-out second product plugin is Phase 2.

Quality gates before commit: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check` green.

## 9. Beads consequences

- Epic: body **editing** stays in the E2 acceptance criterion (full-replace, §4a) —
  only the character-precise Yrs **backend** is split out. Pull acceptance accordingly.
: **yrs body backend** (character-precise merge, web-Yjs interop, spell out the
  diff-bridge core) — backend swap without changing the CLI signature, outside E2.
: **programmatic plugin layer** (trait/module over the declarative config,
  outside the epic).
 (loudly reject priority 0–4) is satisfied in E2 via `[priority].min/max`.

## 10. Explicitly OUT (summary)

- Character-precise Yrs body backend (ra0) · programmatic plugin API (u2d) · sync/server (E4) ·
  ranking beyond the declarative sort · **external/dynamic** plugin registry (loading from
  files at runtime) · `stats` (nice-to-have). **IN:** `search`, body editing (full-replace) and
  **two embedded configs** (`issue-tracker` + `personal-todo`, selection via `plugin` field, §4).

## 11. Implementation order (cut the seam vertically, early)

E2 is large (15 commands, config loader/deser, ranking engine, discovery, body interface, `search`,
`prime`, hook, plus TDD + parity + cross-replica + dogfood). The plan (writing-plans, next
step) must **de-risk the "no hardcoding" claim before it becomes expensive to disprove** —
so cut the seam *vertically* first, do not build 15 commands wide:

1. **Vertical seam cut first.** Workspace discovery + `init` + config loader (both configs)
   + error envelope + JSON record, then **one** read command (`show` or `list`) end-to-end
   *through the config*. This proves the architecture (core → config → command → JSON) on *one*
   path, including the config-swap test, before breadth emerges.
2. **Mutation path.** `create` + `update` (incl. validation + body full-replace over the narrow
   `write_body` path, §4a) + `claim`/`close` sugar.
3. **Derivation + graph.** `ready`/`blocked`/`next` (ranking engine), `dep add/remove` (with
   cycle rejection), `search`, `note`.
4. **Bootstrap + parity.** `prime`, hook wrapper, parity table + dogfood session as
   acceptance gate.

The `bd` decomposition follows this cut (step 1 = its own small,
early-closable issue). Cross-cutting concerns (WAL/`busy_timeout`, determinism, `--json` contract) are
established in step 1 and apply to all commands thereafter.
