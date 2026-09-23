# Getting Started

The first step is the same whether you came to contribute or to build your own application on the
engine: get the source, prove it builds, and then run something real.

## The source

```bash
git clone https://github.com/nxsflow/nexus-flow.git
cd nexus-flow
cargo test -p nexus-flow-core
```

A Rust toolchain is all it takes; there is nothing else to install and no service to start. The
repository is one Cargo workspace, and the layout is worth two minutes before you go looking for a
file:

| Where | What |
|---|---|
| `crates/foundation`, `crates/core` | the substrate and the engine — the op-log, the reducers, the derivation |
| `crates/facade` | the public API an application links. The one SemVer contract in the repo |
| `crates/cli`, `crates/memory`, `crates/chat`, `crates/nxs` | the four command surfaces, thin callers over the engines |
| `crates/*/docs/guide/{en,de}` | the guides — the same text `nxs guide` prints and this website serves |
| `content/` | the assembler that turns those guides into the published documentation feed |
| `examples/tauri-board/` | the example application, outside the workspace on purpose |

## The four gates

Four commands are the gates this project holds itself to:

```bash
cargo test                                  # unit + integration (incl. differential oracle)
cargo test --release                        # catches side-effects compiled out of debug
cargo clippy --all-targets -- -D warnings   # lint, warnings are errors
cargo fmt --check                           # formatting gate
```

A full run is long — long enough that it is not what you do between two edits. Test the crate you
are working in (`cargo test -p nexus-flow-core`) while the change is in your hands, and let CI be
the gate: it runs the debug pass on every pull request, and the release pass and the macOS lane on
`main`. Behaviour is written test-first here, so the red test comes before the code that answers
it; and a change users would notice brings a changelog fragment in `changes/`, in both languages.

## The example application

`examples/tauri-board/` is the one to copy from. It embeds the engine **in-process** — it links
`nexus-flow-facade`, holds one long-lived `Engine` in Tauri's managed state, and answers its window
from that handle. There is no `nxf` subprocess anywhere in it, which is the whole point: the seam it
calls is the same seam the CLI calls, so nothing it does is a private path an application could not
take.

It sits **outside** the Cargo workspace, so a root `cargo test` never compiles Tauri and WebKit on
your machine. That is deliberate, and it costs the example its coverage — which is why a dedicated
CI job type-checks it whenever the example, or the facade, core or foundation under it, changes.

To run it you need the platform WebView (preinstalled on macOS) and a workspace to point it at; the
example's own `README.md` has the exact commands, including the small graph it seeds so the window
has something to show. Watch it once with `nxf` writing to the same workspace from another terminal:
the window re-renders on its own, because it subscribes to the engine's change stream rather than
polling it.

## Then read the map

[Architecture](develop-architecture) is the map of what you just cloned — five layers, which way the
arrows point, and which seams are contracts. After that,
[the journey of one operation](develop-the-journey-of-one-op) follows a single `nxf close` from the
verb to a derived answer nobody wrote, and
[a reply across two machines](develop-a-reply-across-two-machines) takes the same trip with a second
replica in it.
