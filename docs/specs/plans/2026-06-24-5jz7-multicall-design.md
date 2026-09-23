# 5jz.7 — Multicall: one `nxs` binary, `nxf`/`nxm` as argv[0] shims (Design)

> Design doc. Approved approach (Carsten, 2026-06-24): split into **7a
> (engine)** and **7b (delivery)**; `nxf-relay` stays separate; re-scope 5jz.4/.5/.6. The
> implementation plan for 7a follows in its own dated plan doc.

## Goal

One product **CLI** binary, `nxs`, that contains the whole flow + memory + umbrella logic. `nxf`
and `nxm` stop being separately compiled binaries and become **symlinks** to `nxs`; the binary
routes by `argv[0]`. Product benefits: one artifact to build / sign / notarize / release, a
consistent `--version`, and — by construction — **no version drift between the tools** (this
dissolves the "Release-Kopplung" the epic had consciously accepted).

## Why this is cheap to do

`crates/nxs/src/main.rs` ALREADY force-links `nexus_flow_cli` and `nexus_memory` (so their
`inventory::submit!(ModuleInit)` registrations reach the image — the 5jz.1 composition root). The
`nxs` binary therefore already contains all of flow's and memory's logic. Multicall is **routing +
collapsing bin targets**, not relinking new code. Compile-time/space savings are marginal (the
workspace already compiles each third-party dep once and shares it); the win is product-level.

## Decomposition

- **7a — engine (this issue):** multicall routing in the `nxs` binary; collapse
  `nxf`/`nxm` from bin targets to symlinks; migrate the test harness. One PR.
- **7b — delivery (follow-up):** release pipeline + `install.sh` + `self-update`
  ship the single binary + symlinks, signing/notarizing only the real `nxs`. Separate PR.

## Routing design (7a)

- Each CLI crate exposes `run_from(args: impl IntoIterator<Item = OsString>) -> ExitCode`; the
  existing `run()` becomes `run_from(std::env::args_os())`.
- The `nxs` binary's entry inspects the **`argv[0]` basename** (NOT `current_exe()` — on Linux that
  resolves the symlink back to `nxs`; the invocation name lives in `argv[0]`):
  - `nxf` → `nexus_flow_cli::run_from(args)` (clap program-name = `nxf`).
  - `nxm` → `nexus_memory::run_from(args)`.
  - `nxs` (or anything else) → peek `argv[1]`:
    - `flow`   → flow surface, synthesizing program-name `nxf` and dropping the `flow` token.
    - `memory` → memory surface, program-name `nxm`, dropping `memory`.
    - else     → the `nxs` umbrella parser (init/prime/migrate/doctor/sync), unchanged.
- `flow`/`memory` are intercepted **before** clap parses, so they are NOT in the `nxs` clap tree and
  never appear in `nxs --help` (the "hidden" requirement, achieved structurally). `nxs flow <args>`
  is byte-identical to `nxf <args>` because it runs the same parser with the same program-name.

## Bin targets, symlinks, and the test harness (7a)

- **Cargo:** drop `[[bin]] nxf` (cli crate) and `[[bin]] nxm` (memory crate); keep their libraries.
  `nxs` becomes the only CLI bin target (alongside the separate `nxf-relay`).
- **Symlinks:** `nxf` → `nxs`, `nxm` → `nxs`, created next to the built `nxs` (a tiny xtask/test
  helper for dev + cargo test; the release pipeline does the same in 7b).
- **Test harness:** today ~30 files call `assert_cmd::Command::cargo_bin("nxf"|"nxm")`. With no
  `nxf`/`nxm` bin targets, `cargo_bin` can't find them. Introduce one shared test helper —
  `bin(name)` — that resolves the built `nxs` (from the integration test's own location) and ensures
  a sibling `nxf`/`nxm` symlink exists (idempotent, created once), returning a `Command` for it.
  Existing tests keep exercising the **real symlink** (true argv[0] multicall coverage); the change
  is mechanical (swap the per-crate `nxf()`/`nxm()` helper bodies). `nxs` tests keep `cargo_bin`.
- **Shell-out paths unchanged:** `nxs prime` / the assembler resolve sibling binaries via
  `resolve_binary` (`crates/nxs-init/src/spawn.rs`); with symlinks, `nxf`/`nxm` resolve to `nxs`
  and `nxf prime` dispatches correctly via `argv[0]`. TB-5 (prime/agent-manifest stay shell-out)
  holds; only the init path is library composition.

## `nxf-relay`

Stays a **separate** binary. It is the sync **relay server** (E4): the dumb durable op-carrier over
an axum/tokio HTTP surface — a different deployment artifact with a different (async server) stack,
not a CLI tool. Any future server consolidation belongs in its own "nxs server" track, not this CLI
multicall.

## Migration (`.nexusflow` → `.nxs`)

Independent of binary layout: the foundation migrates a legacy `.nexusflow/` workspace on open
(`nxs-foundation`), whichever binary opens it. Multicall does not touch this, so existing
`.nexusflow` project folders stay migratable. The 7a plan adds an explicit test: a legacy
`.nexusflow` dir is migrated correctly when reached through the multicall `nxf`/`nxs`.

## Consequences for the sibling subtasks (re-scoped per this design)

- **5jz.4 (Installed-Detection) — SHRINKS.** With one binary, every module is always "installed"
  (the union is linked in), and the standalone-binaries-on-PATH dimension disappears (`nxf`/`nxm`
  are symlinks to the same file). Re-scope to: indicate per module whether it is **active in the
  workspace** (`config.active_modules`) in the chooser/summary; drop the PATH/standalone scan.
- **5jz.5 (byte-stability + contract/doc + release-coupling) — PARTLY DISSOLVED.** "Release-Kopplung
  absichern" is moot once there is a single artifact (the 7 win). What remains: byte-stability
  goldens; the contract/doc rewrite (the two fallen UX contracts + TB-5-reversal-for-init) **plus**
  documenting the multicall architecture in `nxs-platform-foundation.md`. The release-pipeline change
  lives in 7b.
- **5jz.6 (#hai welcome copy + post-init success moment) — UNAFFECTED.** Pure UX polish in the same
  `nxs_ui` frame; orthogonal to the binary layout.

## Acceptance (7a)

`nxf <args>` ≡ `nxs flow <args>` and `nxm <args>` ≡ `nxs memory <args>`, byte-identical across the
whole command surface (not just init). Exactly one CLI binary is compiled+linked; `nxf`/`nxm` are
symlinks. `flow`/`memory` do not appear in `nxs --help`. All golden/trycmd tests green;
`prime`/`agent-manifest` shell-out still works; a legacy `.nexusflow` workspace still migrates.

## Out of scope (→ 7b / elsewhere)

Release pipeline + `install.sh` + `self-update` single-binary delivery (7b). Windows symlink story
(deferred with the Windows slice, 85y.20). Server consolidation ("nxs server").
