# 2026-11-15 · no ticket · The nxc sidecar is COMPILED INTO the nxs binary since v0.54.0 — the side-by-side route had a bootstrap hole no version could close.

- Decided by: recorded in the project memory (`nxm`, key `nxc-sidecar-ships-with-the-binary`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, sidecar, release

## The record

The nxc agent sidecar has been COMPILED INTO the `nxs` binary since v0.54.0 — not placed beside it
any more.

WHY: the place-it-beside route has a bootstrap hole that no version can close from the inside — the
code that places the second file lives in the NEW version, but the update is always carried out by
the OLD one. Every machine on 0.51.0 or earlier therefore ended up without a sidecar, and the fix
that was meant to ship it never effectively reached the installed base.

FORM: `agent-sidecar` bundles via esbuild into ONE self-contained ESM file
`dist/nxc-agent-sidecar.mjs` (~1.4 MB); `crates/chat/build.rs` compiles it into the binary with
`include_bytes!`, and `worker.rs` unpacks it on first use into
`<cache>/sidecar/<sha256[..16]>/nxc-agent-sidecar.mjs` (`NXF_CACHE_DIR`, else
`XDG_CACHE_HOME/nxf`, else `~/.cache/nxf` — the same ladder as `selfupdate.rs::cache_dir`).
Resolution order in `installed_sidecar()`: `NXC_SIDECAR` (developer override) -> EMBEDDED ->
`sidecar_beside(exe)` (legacy installs at 0.53.0 and earlier, plus the source tree's `main.mjs`
under `target/{debug,release}`). Content-addressed means binary and sidecar are a pair by
construction, which retires the whole "they carry no common version" problem outright. The tarball
has TWO members again (`nxs`, `nxf-relay`); `install.sh` and `selfupdate.rs` only place a separate
sidecar when a `--version` pin selects 0.52.0/0.53.0, and otherwise REMOVE a superseded copy.

TRAP 1: `build.rs` is tolerant (no bundle -> `None`) so a contributor without Node can build.
`NXF_EMBED_SIDECAR=require` is what turns that tolerance off — and it is set in exactly TWO PLACES,
not pipeline-wide (measured 2026-09-18, correcting this entry's earlier "ci.yml AND release.yml set
it", which read like a guarantee over every job): `ci.yml`'s `quality-gates` job, exported to
`$GITHUB_ENV` right after the bundle-build step, and `release.yml`'s `build` job, as a step-level
`env:` on the two binary-build steps. The other six ci.yml jobs never build the bundle at all and
rely on `build.rs`'s documented fallback to the source tree. So the rule is: wherever a job builds a
binary that MUST be able to start an agent, build the bundle first and set `require` — otherwise you
get a binary that installs cleanly and cannot start an agent, and nothing in that job says so.

TRAP 2: because the bundle has no `node_modules` beside it, the Claude Agent SDK cannot resolve its
native `claude` binary itself — the HOST resolves `claude` on `PATH` and passes it as
`spec.claudePath`. `is_bundled_sidecar()` keys on the FILE NAME, which is why the unpacked file
keeps exactly that name and only the directory carries the hash.

TRAP 3: SIX places must spell the file name identically, and the gate
`every_place_that_ships_the_sidecar_spells_it_the_same` (`crates/chat/tests/worker.rs`) counts
OCCURRENCES, not files: `agent-sidecar/package.json` 1, `crates/chat/build.rs` 2, `install.sh` 4,
`release.yml` 2, `ci.yml` 3, `crates/cli/src/selfupdate.rs` 1.

TRAP 4: NEVER `npm ci --omit=optional` for the bundle build — that also skips esbuild's
integrity-pinned `@esbuild/<platform>` package.
