# nexus-flow

**The board, the memory and the channel your agents work from.**

nexus-flow is an issue tracker for coding agents — with the memory and the channel beside it. Your
agents read and write it through a plain CLI: deterministic output, `--json` on every command. It
keeps the work in a local database inside your repository, out of git's reach: offline-first, fast
locally, convergent when connected.

Not an agent framework: you bring the agents (Claude Code, Codex, your own); this is what they
work from.

## Get started

One line installs it, and `nxs init` sets it up in any repository:

```console
$ curl -fsSL https://nxsflow.com/nxs/install.sh | sh
…
nexus-flow suite installed (nxf, nxm, nxc, nxs). Run: nxs init

$ nxs init
…
nexus-flow is set up
…
  modules:      flow
…

$ nxf create --type epic --priority P1 --title "CSV export for the reports view" \
    --description "Any report can be downloaded as CSV."
created x2gg

$ nxf next
x2gg  P1  open  [epic]  CSV export for the reports view
```

The installer puts one signed binary, `nxs`, into `~/.local/bin` — macOS and Linux, on arm64 and
x86-64 — with `nxf`, `nxm` and `nxc` linked beside it. It needs no sudo and tells you when
`~/.local/bin` is not on your `PATH`. It checks the download's signature before it installs, and
refuses to install when it cannot: on Linux a current OpenSSL is enough, on macOS run
`brew install minisign` first.

On a terminal, `nxs init` asks which of the three tools to set up; without one, as here, it sets up
the tracker alone. It also wires Claude Code's session start, and leaves every other agent a pointer
to `nxs prime` in `AGENTS.md`, so a new session opens knowing the rules of the board and what is
next.

Every transcript on this page comes from one run of nxs 0.66.0, the stable release on 2026-09-21,
in a fresh repository. Later releases print some of it differently. Prepared beforehand: the two
persona files (the coder's is shown below) and the serializer code the tickets refer to, committed
on the feature branch and merged off camera. `…` marks output that was cut; nothing was added.

## Coming from beads?

One flag brings a beads board over. Run it in place of the plain `nxs init` above, on a clean
working tree:

```sh
nxs init --from-beads
```

It writes a timestamped backup of the beads data, sets up the tracker and the memory, and imports
the tickets with their dependency graph and close reasons, and the memories. Then it takes beads'
own session hook and managed block out of your agent files and leaves your own text in them alone —
the clean tree is what lets you review that. It only runs while the repository has no `.nxs/`
workspace yet: once one exists, it imports nothing. On a terminal, `nxs init` finds a beads project
by itself and asks; the flag is the consent for a run that cannot ask. The CLI vocabulary is
deliberately familiar, so an agent fluent in `bd` is productive at once. Why this is a new engine
and not a fork is under [Prior art](#prior-art--acknowledgements).

## Set the direction.

Plan the work with your agents and put it on the board: an epic, its tickets, and which of them
waits for which. What is ready to work on is derived from that graph on every query — never stored,
so it cannot go stale.

```console
$ nxf create --type feature --parent x2gg --priority P1 --title "Write the CSV serializer" \
    --description "One row per report line, RFC 4180 quoting."
created 631z

$ nxf create --type feature --parent x2gg --priority P1 --depends-on 631z \
    --title "Export button in the reports toolbar" \
    --description "Downloads the open report through the serializer."
created c6eq

$ nxf create --type chore --parent x2gg --priority P2 --depends-on 631z \
    --title "Document the export format" \
    --description "Write docs/export-format.md: the columns, quoting and line endings reports/csv_export.py produces, for whoever imports the file."
created kahm

$ nxf next
x2gg  P1  open  [epic]  CSV export for the reports view
631z  P1  open  [feature]  Write the CSV serializer
    ↳ x2gg · CSV export for the reports view

$ nxf blocked
c6eq  P1  open  [feature]  Export button in the reports toolbar
    ↳ blocked by: 631z (open)
kahm  P2  open  [chore]  Document the export format
    ↳ blocked by: 631z (open)
```

The board does not move with your checkout. It lives in `.nxs/`, which keeps itself out of git, so
no branch switch rolls it back — a ticket closed on a feature branch is still closed on `main`, in
this session and the next:

```console
$ git switch -c csv-serializer
Switched to a new branch 'csv-serializer'

$ nxf close 631z --reason "Serializer done, with tests for quoting and empty reports."
closed 631z

$ git switch main
Switched to branch 'main'

$ nxf next
x2gg  P1  open  [epic]  CSV export for the reports view
c6eq  P1  open  [feature]  Export button in the reports toolbar
    ↳ x2gg · CSV export for the reports view
kahm  P2  open  [chore]  Document the export format
    ↳ x2gg · CSV export for the reports view
```

## Assign the ticket.

The channel and the memory are the other two tools. In the same repository, turn them on:

```console
$ nxs init --module memory --module chat
nexus-flow is set up
…
  modules:      flow, memory, chat
…
```

Who works a ticket, and in which order, is a file in your repository: a channel, declared in
`.nxs-personas/` next to the personas it names. One message hands it the ticket. Each member gets a
fresh agent session with the message in hand, does its part and answers on the record, and the next
member starts once the one before it has answered (or its declared timeout has run out). What a
member does is up to its prompt. This coder's prompt says to claim the ticket, do it, commit, and
close it:

```yaml
# .nxs-personas/coder.yaml
handle: coder
job_title: Coder
job_description: Takes one ticket off the board, does it, and closes it.
stage: junior
addressable: [ship]
system_prompt: |
  You are the coder, the first step of the `ship` channel.

  ## How you finish — read this first
  You finish by answering on your own thread:
  `nxc reply --thread <the thread id in your trigger message> "<one sentence: what you did>"`.
  If you cannot do the work, answer the same thread with `nxc reply --escalate "<why>"` instead.

  ## The work
  The trigger message names a ticket (its `nxf_ids` ref). Read it with `nxf show <id>`, claim it
  with `nxf claim <id>`, do exactly what it asks in this repository, commit it, and close it with
  `nxf close <id> --reason "<what you did>"`. Keep the change small.
tools: [Bash, Read, Write]
permissions: acceptEdits
```

`reviewer.yaml` has the same shape: read the last commit, change nothing, answer `approve` or
`changes requested`, with `tools: [Bash, Read]`.

```console
$ cat .nxs-personas/channels.yaml
- name: ship
  members: [coder, reviewer]
  flow: sequential
  description: a ticket goes to the coder, then to the reviewer

$ nxc list
## Who you can address

Address any of them the same way: `nxc send --to <handle> "<your message>"`.

**Ship** (handle: `ship`, members: coder, reviewer) — a ticket goes to the coder, then to the reviewer

$ nxc send --to ship --ref nxf_ids=kahm "Document the export format."
opened thread m-01M3172488SFEEK4TDGQRGE0MH in decl:ship — reply with `nxc reply --thread m-01M3172488SFEEK4TDGQRGE0MH "…"`

$ nxc status
operation m-01M3172488SFEEK4TDGQRGE0MH  decl:ship  2 thread(s), 2 open
  m-01M3172488SFEEK4TDGQRGE0MH  decl:ship  waiting on h250/__channel__
    m-01M317248ACNSZC74RRSQCXBAN  decl:ship  waiting on h250/coder
```

In this run, half a minute later the coder had answered and the reviewer had the ticket:

```console
$ nxc status
operation m-01M3172488SFEEK4TDGQRGE0MH  decl:ship  3 thread(s), 2 open
  m-01M3172488SFEEK4TDGQRGE0MH  decl:ship  waiting on h250/__channel__
    m-01M317248ACNSZC74RRSQCXBAN  decl:ship  answered
    m-01M3172XPSMHVTB56N03TR3SJ3  decl:ship  waiting on h250/reviewer
```

Less than a minute after the send, both had answered on their own threads, and the board already
knew:

```console
$ nxc threads show m-01M317248ACNSZC74RRSQCXBAN
thread m-01M317248ACNSZC74RRSQCXBAN in decl:ship
  expects:     h250/coder
  replied:     h250/coder
…
  · h250/__channel__ Document the export format.
  · h250/coder Wrote docs/export-format.md (…), committed it, and closed ticket kahm.

$ nxc threads show m-01M3172XPSMHVTB56N03TR3SJ3
thread m-01M3172XPSMHVTB56N03TR3SJ3 in decl:ship
  expects:     h250/reviewer
  replied:     h250/reviewer
…
  · h250/__channel__ Document the export format.
  · h250/reviewer approve: docs/export-format.md matches reports/csv_export.py (…), …

$ nxf list --status closed
631z  P1  closed  [feature]  Write the CSV serializer
kahm  P2  closed  [chore]  Document the export format
```

Members run as Claude Code sessions that `nxc` starts through Node.js, so a channel needs both on
your `PATH`, and each round runs on your Claude Code login like any other session. `tools:` decides
which tools a session gets. `Bash` among them is a full shell, so what keeps this reviewer read-only
is its prompt, not a sandbox. The board and the memory need none of this; they work with any agent
that can run a command. The full field set is in `nxc guide personas`.

## Keep what they learn.

When an agent gets something wrong and you tell it why, it records the lesson as a rule of the
project — `nxs prime` tells every session to keep what it learns this way:

```console
$ nxm remember "Write CSV exports as UTF-8 with a byte-order mark. Without it, Excel on Windows garbles every umlaut." --key csv-utf8-bom --category rules
remembered csv-utf8-bom
```

Since 0.68.0, `nxs prime` lists memories by a one-line introduction instead of in full, and
`nxm remember` takes that line as `--introduction` (0.98.0 requires it).

The next session — any agent, any day — opens with it:

```console
$ nxs prime
# nexus-flow
…
## Next (2)

- `x2gg` · P1 · in progress · [epic] · CSV export for the reports view
- `c6eq` · P1 · open · [feature] · Export button in the reports toolbar
  ↳ `x2gg` · CSV export for the reports view
…
## Memories (1)

### `csv-utf8-bom`

Write CSV exports as UTF-8 with a byte-order mark. Without it, Excel on Windows garbles every umlaut.
…
```

## One binary, three tools

| Tool | Command | What it is |
|---|---|---|
| **nexus-flow** | `nxf` | the board — projects, tickets and the dependencies between them |
| **nexus-memory** | `nxm` | the memory — durable project knowledge (`remember` / `recall` / `memories` / `forget`) |
| **nexus-chat** | `nxc` | the channel — messages between you and your agents, on the record |

`nxs` is the one binary; `nxf`, `nxm` and `nxc` are symlinks to it that dispatch by the name they
were called under, so there is one artifact to install, verify and update (`nxs self-update`). All
three write to the same local database through one change-log: the ticket, the rule that came out of
it and the conversation about it are one record. `nxs prime` prints what a session starts with, and
`nxs mcp` serves the board and the memory to MCP hosts. Every tool carries its own offline guides:
`nxs guide`. The design is in [`docs/specs/nxs-platform-foundation.md`](docs/specs/nxs-platform-foundation.md)
and the product visions under [`docs/vision/`](docs/vision/).

## What `nxs init` puts in your repository

- **`.nxs/`** — the database. It ignores itself (`.nxs/.gitignore`), so git never versions the
  board.
- **`AGENTS.md`** — a short block between `<!-- BEGIN NEXUS … -->` and `<!-- END NEXUS -->` that tells
  any agent landing in the repository to run `nxs prime`. The block is managed: `nxs init`
  regenerates it, so write around it, not inside it. Commit it — it is how an agent on another
  machine learns that this project runs on nexus-flow. The board itself does not travel with a
  clone; it stays in the local `.nxs/`.
- **`.claude/settings.json`** — the session-start hook for Claude Code, and `Bash(nxs:*)`-style
  permission entries so it may run the suite's commands without asking, `nxc send` among them,
  which starts agent sessions.
- **`CLAUDE.md`** — only if you have one, and only in 0.66.0: an `@AGENTS.md` line at the top.
  0.98.0 leaves it alone.
- **`.nxs-personas/`** (with chat) — where you declare personas and channels.
- **`NEXUS_MEMORY.md`** (with memory) — written by the first `nxm remember`: the memories, projected
  into a file for any reader without the hook. Generated; change it with `nxm remember`.

To take nexus-flow out of a repository again, delete `.nxs/`, the block in `AGENTS.md`, the
`@AGENTS.md` line in `CLAUDE.md`, the hook and the permission entries in `.claude/settings.json`,
and `NEXUS_MEMORY.md`. Delete `.nxs-personas/` too, unless you want to keep what you declared there.
Later releases also keep a machine-wide list of workspaces in `~/.nexusflow/` and, on a terminal,
offer a background service; `nxs sync unregister` and `nxs sync daemon uninstall` take those back.

## How it works

Everything lives in a **local database**, so reads and writes are instant and keep working
when the network is gone. Each change is appended to a **change-log** instead of
overwriting state, so when two devices reconnect their logs merge **with no conflict to
resolve** — concurrent edits converge to the same result on every device, automatically. Edits to
different fields both survive; two edits to the same field go to one deterministic winner. That
guarantee is a **CRDT** (a conflict-free replicated data type); nexus-flow uses a narrow,
hand-rolled one on plain SQLite.

The core (`crates/core`, `nexus-flow-core`) is an **op-log CRDT substrate**: an
append-only `ops` table is the single source of truth. Writes append an op and fold it
into materialized views with keep-if-beats LWW + observed-remove OR-set semantics.

- **Derivation, not stored state.** `nxf next` and `nxf blocked` are deterministic
  computation (pure SQL over the views), never a column you can desync.
- **Offline-first, convergent.** Items sync via a hand-rolled narrow relational CRDT on
  plain SQLite (LWW + OR-set + tombstones), chosen by the E0 bake-off. The long texts —
  description, design — are last-writer-wins fields like every other scalar; a
  character-level merge for them is not built. The server is the durable truth. Automerge
  is kept as a differential-test oracle only — never in the binary.
- **Agent-ergonomics is the measuring stick.** `--json` everywhere, deterministic
  output, a closed structured-error envelope. If it isn't pleasant for an agent to
  drive, it isn't done.

## Embed it in your app

Under the CLI is an engine — not the agent, not the UI. Ranking, vocabulary and presentation live in
plugins, and the engine itself is a library an app embeds. So an agent finds its way around the work
through the same deterministic `--json` surface while you build your own visualization on top.

The vocabulary is not fixed: a plugin declares the item types, their names and how the work is
ranked. The binary ships two plugins — the issue tracker above and a personal to-do list — and an
app that embeds the engine brings its own.

Link `nexus-flow-facade` (as a path or git dependency — it is not on crates.io), hold one
`Engine`, and read the same derived lanes the CLI does — in-process, no subprocess. A minimal, real desktop example lives in
[`examples/tauri-board/`](examples/tauri-board/): a Tauri window that renders the live lists and
re-renders on its own whenever anything writes to the workspace. It's the "powered by nexus-flow"
model in ~120 lines of Rust and one HTML file. Our own products, **manufakt.io** and **nexflow.it**,
are built exactly this way: each bundles the engine invisibly and puts its own product over it.

## Build & test

A Cargo workspace at the repo root:

```bash
cargo test                                  # unit + integration (incl. differential oracle)
cargo test --release                        # catches side-effects compiled out of debug
cargo clippy --all-targets -- -D warnings   # lint, warnings are errors
cargo fmt --check                           # formatting gate
```

Be careful with the use of `clippy` as it can take up a lot of compute resources. 

## Repository layout

```
crates/foundation   shared substrate: kind-tagged op-log + reducer registry + workspace
crates/core         nexus-flow-core — the engine (data model, derivation, store)
crates/facade       stable public API surface (SemVer contract)
crates/cli          nxf — the nexus-flow agent CLI
crates/memory       nexus-memory / nxm — the durable-knowledge product
crates/chat         nexus-chat / nxc — the channel product
crates/nxs          nxs — the single multicall binary (nxf/nxm/nxc dispatch by argv[0]); init/prime fan-out, migrate/doctor
crates/nxs-init     shared init/compose registry (modules self-register; the umbrella composes them)
crates/nxs-ui       shared CLI presentation layer
crates/guide        the embedded offline guides each tool serves
crates/sync         CRDT sync
crates/service      the background service seam: machine-wide home, workspace registry, single-instance lock
crates/server       durable sync server
docs/specs          engineering specs (data model, sync substrate, platform)
docs/vision         narrative product visions (flow, memory, chat)
```

## Prior art & acknowledgements

nexus-flow stands on ideas proven by **[beads](https://github.com/gastownhall/beads)**
(`bd`, by Steve Yegge). beads demonstrated that *agent-native* issue tracking
works — a CLI built for an AI agent to drive, with deterministic output and a
session-priming step (`bd prime`) that loads context automatically — and that the same
shape extends to durable agent memory (`bd remember` / `recall` / `memories` /
`forget`). Those ideas are good, and we use them on purpose — hence the familiar CLI
vocabulary.

We took that proven surface and rebuilt the foundation under it — and carried the idea
further:

- **A merge that never stops for a person.** beads keeps its data in Dolt and syncs it with
  `bd dolt push` / `bd dolt pull`
  ([Sync Concepts](https://beads.gascity.com/core-concepts/sync-concepts)), merging at the cell
  level: edits to different fields of an issue merge on their own, and when both sides changed the
  *same* field, beads settles it last-write-wins by the issue's `updated_at` timestamp. It stops
  the pull for a person where that timestamp cannot decide — two equal or unreadable `updated_at`
  values, a contested `notes` or `metadata` field, a row added on both sides or deleted on one —
  and on a few conflicts outside the issues themselves (all as of beads v1.3.0:
  [`bd sync --help`](https://github.com/gastownhall/beads/blob/v1.3.0/cmd/bd/sync.go),
  [merge rules](https://github.com/gastownhall/beads/blob/v1.3.0/internal/storage/versioncontrolops/automerge.go)).
  nexus-flow writes every edit as an op into its log and folds the log with a **CRDT**. Different
  fields merge the same way, and the same field goes to a single winner too — a last-writer-wins
  register — but ordered by a Lamport clock with the replica's site id as tie-break instead of a
  wall-clock timestamp, so there is never a tie to stop on ("last" means causally last, not last
  on the wall clock). Dependencies are observed-remove sets, so a concurrent add and remove keep
  the edge. Given the same set of ops, every replica folds to the same result, whatever order they
  arrive in: there is no conflict state and no side to pick. Both settle a collision by a
  last-writer rule — what differs is what "last" means, and what happens where beads' rule has no
  answer. It is a foundational decision you make up front, not one you bolt on later, which is why
  nexus-flow is a new engine rather than an extension of beads.
- **Derivation instead of stored state.** `nxf next` and `nxf blocked` are computed
  from the op-log, not persisted and kept in sync by hand.
- **One binary, three tools.** Where beads keeps memory *inside* the tracker, `nxs`
  separates concerns into distinct tools — flow (the board), memory (durable knowledge),
  chat (the channel) — on one shared op-log substrate and one local DB.

For full transparency: nexus-flow *used* beads as its own issue tracker during early
development, then migrated onto itself on 2026-07-06 — the "first consumer replaces beads"
step in its own vision. [`nxs init --from-beads`](#coming-from-beads) lets you make the same
move. Credit where
it's due — beads charted this territory; nxs builds a different engine for it.

## License

Licensed under the [Apache License 2.0](LICENSE).
