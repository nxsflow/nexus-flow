# Embedding nexus-flow in a desktop app — a Tauri example

A small, **real** Tauri v2 desktop app that embeds the nexus-flow engine **in-process**: it links
`nexus-flow-facade` as a library and calls `Engine` directly — there is no `nxf` subprocess. This is
a reference for the promise on the project README — *bring your own agent and your own UI;
nexus-flow is the engine underneath* — made concrete in ~120 lines of Rust and one HTML file.

The window shows the live **Next** and **Blocked** lists and re-renders **on its own** whenever
anything else writes to the same workspace — the `nxf` CLI, the MCP server, or sync from another
device. No polling, no reload: the app subscribes to the engine's change stream.

## What it demonstrates

- **In-process embedding.** `Engine::open(…)` opens the workspace and holds one long-lived handle in
  Tauri managed state. The app *is* the consumer — it links the engine as a crate, exactly as
  `manufakt.io` and `nexflow.it` do.
- **The same derivation as the CLI.** `next` / `blocked` are `#[tauri::command]`s that read through
  that handle. No new logic — the exact `nxf next` / `nxf blocked` the CLI computes, through the
  shared facade seam. `next` is the actionable set, `ready ∪ in_progress`: claimed work stays on
  the list, which is why the column is headed *Next* and not *Ready*.
- **Live, offline-first updates.** On startup the app `subscribe()`s to the change watcher and
  forwards each change to the webview as an event; the frontend re-queries on every event. This is
  the offline-first engine made visible: local, instant, and convergent.

The frontend is a single static `dist/index.html` — no build step, no framework — so the embedding
story (a Rust host + the engine + a webview) is the only moving part.

## Run it

Prerequisites: a Rust toolchain, the platform WebView (WebKit — preinstalled on macOS), and the
`nxf` CLI on your `PATH` (`cargo install --path ../../crates/cli`).

```bash
# 1. Make a workspace and seed a small graph (in a scratch dir):
mkdir /tmp/nxf-demo && cd /tmp/nxf-demo
nxf init --plugin issue-tracker
A=$(nxf create --type feature --title "Design new checkout flow" --description d --priority P1 -q)
nxf create --type chore   --title "Update the changelog"    --description d --priority P2 -q
B=$(nxf create --type feature --title "Ship the 1.0 release"  --description d --priority P1 -q)
nxf dep add "$B" "$A"          # B depends on A → B shows under Blocked, A under Next

# 2. Launch the app pointed at that workspace (from THIS directory):
cd -                           # back to examples/tauri-board
(cd src-tauri && NXF_DB=/tmp/nxf-demo/.nxs/db.sqlite cargo run)
```

> No Tauri CLI required — the frontend is static, so `cargo run` from `src-tauri` serves `../dist`
> directly (`cargo tauri dev` works too if you have it).

Then, in another terminal, drive the graph and watch the window move with **no reload**:

```bash
cd /tmp/nxf-demo
nxf close "$A" --reason done   # "Ship the 1.0 release" jumps from Blocked to Next, live
```

## Build note

The crate is kept **outside** the root Cargo workspace (`[workspace] exclude` in the repo-root
`Cargo.toml`), so the normal `cargo test` / CI run never has to compile Tauri + WebKit — build it on
its own from this directory. A deterministic, headless counterpart that exercises the same embed API
without a GUI lives in `crates/cli/tests/embedding.rs`.

Being outside the workspace is also how this example quietly stopped compiling twice, the second
time for seven weeks: nothing built it. `.github/workflows/example-check.yml` now runs
`cargo check` on it whenever it — or the facade, core or foundation it links — changes.
