# Import and Migration

Two on-ramps, for two different situations. **Import** brings an existing Claude-host memory store
in. **The judging migration** files what this workspace already holds and moves its hand-written
context documents into memory, section by section.

Both exist because the memory rule — durable project knowledge lives in `nxm remember`, never in an
ad-hoc `MEMORY.md` — has to be an on-ramp rather than a dead end for a project that already wrote
one.

## Import a Claude-host memory store

Claude Code keeps a per-project memory directory: a `MEMORY.md` index plus one markdown file per
fact, each with a small frontmatter block. `nxm import` reads it and writes the facts into this
workspace.

`nxm init` detects that directory for the current project and says so on its banner — *Found 2
existing Claude memories* — so the on-ramp is offered rather than hidden. Taking it is one command:

```console
$ nxm import
imported 2 memories from claude-memory

$ nxm memories
auth-jwt  auth uses JWT not sessions
race-flag  always run tests with the -race flag

```

Three properties are what make this safe to run without thinking about it:

- **The key is the host's own id.** Each fact comes in under its frontmatter `name:`, verbatim.
- **It is idempotent.** Because the key is stable, a second run upserts the same memories in place.
  Nothing duplicates:

```console
$ nxm import
imported 2 memories from claude-memory

$ nxm memories
auth-jwt  auth uses JWT not sessions
race-flag  always run tests with the -race flag

```

- **It is non-destructive.** The source directory is read and never written. Your Claude memories
  stay exactly where they were.

`--from <dir>` points at a directory explicitly; with no `--from`, this project's Claude memory
directory is auto-detected. A file with no frontmatter, and the `MEMORY.md` index itself, are not
facts and are skipped.

```console
$ nxm import --json
{"imported":2,"keys":["auth-jwt","race-flag"],"ok":true,"source":"claude-memory"}

```

Imported memories arrive `unsorted`, like any other unfiled memory — which is exactly what the
migration below is for.

Coming from **beads**? That migration is the umbrella's, because it moves tickets *and* memories
together: `nxm init --from-beads` consents to it and routes through `nxs`.

## The judging migration

A memory that nobody filed is `unsorted`, and a workspace whose memories are all `unsorted` has a
list rather than an argument — nothing decides which fact a reader meets first. Filing them is a
**judgement**: whether a fact is a rule or an architecture note, whether it holds everywhere or only
here. No heuristic makes that call, so the migration is built around a human (or a model a human
asked) making it.

That is why it is two verbs and not one. `plan` reads and proposes; somebody decides; `apply`
writes. One document travels between them, so **the thing that was reviewed is the thing that is
applied**.

### Where it stands

```console
$ nxm migrate status
3 unfiled memories — run `nxm migrate plan --with-judge`

```

```console
$ nxm migrate status --json
{"unsorted":3}

```

`status` also reports whether a migration already ran on this stream — the mark travels with the
op-log, so a second device sees the earlier run instead of starting a second one. `nxs prime` shows
the same reminder while anything is still unfiled, and goes silent once nothing is.

### Plan

`plan` collects two kinds of entry: every memory still filed as `unsorted`, and every `##` section
of this workspace's hand-written context documents (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`,
`NEXUS_MEMORY.md`). It writes nothing at all.

```console
$ nxm migrate plan
migration plan: 2 unfiled memories, 0 document sections
  classify   f-f64a6f027ce71533       unsorted / project
  classify   since-cutoff             unsorted / item
  skipped    AGENTS.md (generated)
  skipped    NEXUS_MEMORY.md (generated)

Nothing is judged yet. Run `nxm migrate plan --with-judge --json > migration.json` to have a coding assistant propose the filing, review it, then `nxm migrate apply --plan migration.json`.

```

**A generated document is skipped whole**, and the rule hangs on the content rather than the file
name: a file whose head carries a generated header, and any section overlapping an
assembler-managed block, is left alone. Importing a projection back into the store would declare the
projection to be the source — the exact loop `nxm doc --check` exists to prevent. Elsewhere
`AGENTS.md` is hand-written, so the check is on what the file says about itself, not on what it is
called.

**One section is one memory.** The cut is at `##`, so a `###` subsection stays with the argument it
belongs to. A coherent introduction has to survive as one thought.

`--json` emits the plan document — entries plus the instructions and the vocabulary (`categories`,
`scopes`) a judge needs to decide it. That document is what `apply` takes back.

Each entry also carries an `introduction`: the one line the session start replays for that memory
(6j6v.xbnh). It is carried forward for a memory that already has one, and left empty for one that
does not — a judge fills it in. An entry whose `action` is `remember` is refused without one, and
the refusal names the entry, before a single op is written.

### Judge

`--with-judge` pipes the plan through a coding assistant instead of leaving every decision at its
status quo. Bare `--with-judge` uses `claude`; `--with-judge <assistant>` names another.

```bash
nxm migrate plan --with-judge --json > migration.json
```

**This is the one command in memory that may ask a model, and it does not weaken the offline
promise.** What is forbidden is a model call on the *write* path — it would make every
`nxm remember` indeterminate and network-bound — and that is enforced structurally: the library
links no HTTP client at all. The judge here is an **external process** this command starts because
you asked it to, so nothing is wired to a vendor and the engine keeps linking nothing.

`--judge-timeout` is generous by default (`30m`), spelled `<n>` plus `s`/`m`/`h`, or `0` to wait as
long as it takes: a judge reading a large plan legitimately thinks for minutes, and a limit that
interrupts it costs the whole judgement.

Read the plan before applying it. Editing it by hand is expected — that is the whole point of the
document existing.

### Apply

```bash
nxm migrate apply --plan migration.json
```

`apply` files the memories, writes the chosen document sections in as new memories, keeps the plan's
sequence as the reading order, moves the migrated sections out of their source documents, and leaves
the mark. `--plan -` reads the document from stdin.

Four things are worth knowing before you run it:

- **It refuses a plan nobody judged.** An entry still sitting at `unsorted` is not a decision, and
  filing it as one would launder the status quo into a judgement.
- **It is a move, not a copy.** The migrated sections are removed from their source document,
  because a copy leaves two sources for one truth. `--keep-sources` opts out.
- **`--dry-run` reports what would happen and writes nothing at all.** Run it first.
- **It runs once per stream.** A second run — on another device, or from a plan saved before the
  first — is refused, because two independent filings of the same memories converge by
  last-writer-wins into a blend neither of them decided. `--again` overrides it when you mean it.

Afterwards `nxm migrate status` reports `0` unfiled, `nxs prime` stops nagging, and
[`nxm memories --ordered`](nxm-core-concepts) reads as the argument the plan put in order.

## Next

- [core concepts](nxm-core-concepts) — categories, reach, and the reading order a plan decides.
- [commands](nxm-commands) — the flags each of these verbs takes.
