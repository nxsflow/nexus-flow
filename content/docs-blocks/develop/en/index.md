# Build on it, or help build it

Everything above this point answers *how do I use this*. This part answers *how is it built, and
what may I build on* — and there are two doors, both open.

**Build on it.** nexus-flow is a library first. An app links `nexus-flow-facade`, holds one
`Engine`, and reads exactly the lanes the CLI reads — in-process, with no subprocess and no daemon
in between. That is not a claim the repository leaves to prose: `examples/tauri-board/` is a small,
real desktop app that does it in about 120 lines of Rust and one HTML file. Its window lists the
work and re-renders on its own whenever anything else writes to the same workspace, because it
subscribes to the engine's change stream instead of polling. If you are here to put your own
interface on top, that example is the shortest way in.

**Help build it.** The project takes contributions. The engine, the CLIs, the guides you are reading
and the tests that keep them honest all live in one Cargo workspace, and the whole quality bar is
four commands you can run on your own machine — the next page lists them, and says which of them CI
runs when. The seams are documented rather than guessed at, which is what the pages below are for.

The two doors share a first step: get the source, build it, and run something.
[Getting started](develop-getting-started) is that step, for both cases.

## Where to go from here

- [Getting started](develop-getting-started) — the source on your machine, the gates, and the
  example app.
- [Architecture](develop-architecture) — the map: five layers, which way the arrows point, and
  which seams are contracts.
- [The journey of one operation](develop-the-journey-of-one-op) — one `nxf close`, followed from
  the verb to a derived answer nobody wrote — and then
  [the same trip across two machines](develop-a-reply-across-two-machines).
