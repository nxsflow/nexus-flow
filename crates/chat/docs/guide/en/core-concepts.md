# Core Concepts

Everything `nxc` does rests on six ideas: one shared log, declared participants, qualified handles,
threads with a derived quorum, delivery by PUSH, and sessions with transcripts. This guide is the
model underneath the [commands](nxc-commands) — learn it once and the verbs stop needing to be
memorised.

## The substrate: one log, three tools

Chat owns no database. It opens the same `.nxs/db.sqlite` that flow and memory write to, registers
its own reducer, and folds its own views. Every chat operation carries the domain `message`, and a
reducer that does not own a domain leaves an operation stored but unfolded — which is why three
products can share one log without contaminating each other.

Within that domain there are five kinds of operation, each with the conflict behaviour its job
actually needs:

| Kind | Behaviour |
| --- | --- |
| `message` | grow-only. One operation is one message: immutable, never edited. |
| `channel` | last-write-wins per field (`name`, `kind`, `origin`). |
| `membership` | an observed-remove set — an add carries its own tag, a remove carries the tags it saw. |
| `thread` | an immutable root plus four last-write-wins registers (`expects_reply_from`, `deadline`, `name`, `machine`). |
| `profile` | last-write-wins per field; a legacy surface with no verb left. |

There were six until nxf 6j6v.4d2z. The sixth was `read_cursor`, a max register per consumer and
channel that never moved backwards, and it went with the whole unread apparatus — see [Delivery:
push, and only push](#delivery-push-and-only-push) below. A `read_cursor` operation already sitting
in a log you sync from an older replica is not a problem: no reducer claims that kind any more, so
it is stored and never folded, which is the same treatment every unknown kind gets and the reason
three tools can share one log at all.

**There is no daemon.** Chat's change notification is the foundation's `PRAGMA data_version`
watcher; the CLI never holds one open. Every verb is a one-shot pull: resolve the workspace, open
the store, append an operation or read a view, exit.

## Sync is the bus

Two agents in the *same* workspace see each other immediately — they read the same file. Two
different workspaces see nothing of each other until the log is exchanged, and that is a suite
operation on `nxs`, not a chat verb:

```bash
nxs sync bind              # once per workspace; derives the stream from the git origin remote
nxs sync run               # one push/pull pass — this is what moves messages between workspaces
```

`nxs sync bind` derives the stream id deterministically from your `origin` remote, so every clone of
a repository lands on the same stream with nothing to copy around. It also best-effort installs a
per-user background sweep, which you can decline with `--no-daemon` and drive by hand instead.

The trade is stated rather than hidden: zero standing cost — no process, no socket, no battery —
against delivery that is not instant. Everything works with no relay at all; you simply have one
workspace instead of several.

One consequence to know before you put anything sensitive in a message: in this milestone sync is
**full-log and unfiltered**. There is no per-channel authorisation on the wire.

What arrives from another workspace **acts only if that workspace is trusted here**. Every op is
signed by the workspace that wrote it; a message whose signature this workspace cannot check against
a key on its trust list (`nxs sync trust`) is shown — marked `unvouched` — and discharges nothing:
it answers no round, names no session to wake, and is collected as nobody's answer. A thread whose
obligation such an op set is **held**, and `nxc tick` says so instead of acting. The name on a
message is what it claims; the key is what counts.

## Declarations, not registrations

**Nothing in `nxc` creates a participant.** There is no verb that registers an agent or creates a
channel, and that absence is a design decision rather than a gap. Two files under
`.nxs-personas/` are the whole model:

- **`<handle>.yaml`** — one **persona**: who it is, what it is for, which model and tools it runs
  with, and who may address it. See [personas](nxc-personas).
- **`channels.yaml`** — a list of **channels**: who is in them, in what order they run, how long a
  round may take, and what happens to the answers. See [channels](nxc-channels).

A declaration is a file you can read, diff, review and commit. A runtime registration was a row in
one machine's database that nothing reviewed and that did not survive the run. `nxc list` is the
read over the folder, and `nxc send --to` reaches what the folder declares and nothing else.

## Identity: `<origin>/<agent>`

Every sender, every expectation, every claim is written as a **qualified handle**:

- **`<agent>`** is `NXC_ACTOR`, else `$USER`, else the literal `nxc`. A variable set to an empty
  string counts as unset.
- **`<origin>`** is `NXC_ORIGIN`, else the workspace's own **replica prefix** — the same short
  namespace that prefixes flow's item ids. So a handle reads `ab12/coder`, not `local/coder`.

The origin is per workspace, not per machine and not per user. That matters because handles are
matched by exact string: if an app minted `ab12/coder` while a terminal minted `local/coder`, one
declared channel would quietly have two disjoint member sets and nothing would report it. Both the
CLI and the library seam resolve the same value from the same place, and a test holds them to it.

When chat starts a persona, it hands the new process exactly what it needs to know who it is:
`NXC_ORIGIN`, `NXC_DB` (an absolute path, so the working directory never matters), `NXC_SESSION`,
`NXC_ACTOR` set to the persona's own handle, and `NXC_HOP`. Nothing else from the operator's shell
reaches it beyond `PATH` and `HOME` — the child's environment is cleared and refilled from an
allowlist, because everything in it is readable by the agent's own tools.

## Channels: group, public, direct

A channel is where messages live; a thread is a conversation inside one. Three kinds exist:

- **`group`** — the default a declaration produces. Named, member-only.
- **`public`** — the project's front door. It is a **read opening and nothing else**: messages and
  boards in it are readable by anyone in the workspace, whether or not they are a member. What stays
  member-only is the membership-scoped enumerations. Public channels — including ones that arrived by sync and
  that no declaration here names — show up in `nxc list --json` under `public_channels`, and only
  there: discoverable is not the same as addressable, so the human rendering leaves them out.
- **`direct`** — a two-party conversation, minted for you. Its id is derived from the two handles
  (sorted, hashed, `dm:` + 24 hex characters), so both sides compute the same id with no rendezvous
  and no verb. `send --to <persona>` materialises it on the way past.

A declaration may ask for `group` or `public`. It may not ask for `direct` — a DM is derived, and
letting the word parse would mean a typo silently producing a group channel.

A channel id that exists in the store but that no declaration names is **not** a target. Sending to
one is refused with a message naming the file to declare it in; its messages stay readable through
`nxc threads show` and `search`.

## Threads and the derived quorum

A **thread** is the address of a conversation. `send --to` mints one and hands it back, and `reply
--thread <id>` is the only way to post into it. A thread carries an immutable root — origin,
channel, opener, and an optional `parent` — and two mutable registers.

`parent` is what makes an **operation** a tree rather than a list: it is resolved mechanically from
where the sending session was standing, never typed by an agent, and a thread with no parent is a
root. That is why "is this the root" is derived rather than stored as a flag that could be wrong.

`expects_reply_from` is the declared obligation: a list of qualified handles. From it and the
messages in the thread, five things are **derived** — nothing is stored:

- **`expects`** — the declared handles, in declaration order.
- **`replied`** — those that have posted *since the current declaration*. The comparison is against
  the declaration's own clock, which is what makes a multi-turn thread work: a role's answer from
  turn one does not discharge turn two.
- **`outstanding`** — `expects` minus `replied`.
- **`complete`** — the expectation was declared and nothing is outstanding. Clock-free and
  deterministic, which is why it can be the trigger that moves work on.
- **`stale`** — a deadline exists, now is past it, and something is still outstanding.

`complete` and `stale` are disjoint by construction, and together they are what a channel calls
"settled" (see [channels](nxc-channels)). Only the opener may re-declare a thread's expectation;
anyone else gets `forbidden`.

## Delivery: push, and only push

**Delivery is a PUSH.** A message reaches an
agent through the session it starts or resumes: `send --to` opens a fresh session with the body in
its prompt, `reply --thread` resumes the target with the reply's body, and a completed quorum wakes
whoever opened the board. There is no verb for asking after your own messages and none for acking
them — `nxc inbox` and `nxc read` were removed (nxf 6j6v.1gm9) once both had been measured at 0 uses
across 66 role sessions. A person reads the CONVERSATION instead: `nxc threads show <thread>`.

A message still carries a **disposition**: `in_turn` ("act now") or `next_session` ("catch up on
this when you next start"). Since the per-call flag was removed, everything a caller writes is
`in_turn`; `next_session` survives on the wire and in the model, and is what the engine uses for its
own bookkeeping. **Neither is replayed at session start.** `nxs prime` stopped printing message
bodies in nxf 6j6v.4mmk (the unread) and 6j6v.1gm9 (the results of boards you opened), for one
reason measured twice: what every session pays for has to be something it cannot get otherwise and
needs before it acts, and a body it was already handed is neither. In the workspace where this was
last measured that block was 63.787 bytes, 74 % of it the text of finished commissions.

**There is no unread set at all any more** (nxf 6j6v.4d2z). Until v0.88.0 there was one — messages
in channels you are a member of, newer than your synced read cursor, that you did not send yourself
— carried on the seam as `in_turn`/`next_session`/`count` and acknowledged with `Engine::mark_read`.
Nobody read it and nobody acknowledged it, for the reason above: a message is delivered by the
session it starts or resumes, so a list of what you have not fetched is a second copy of what
already arrived. The read cursor, the ack, the `count`, and the per-channel unread numbers on
`Engine::channels` are all gone; nothing was invented to replace them. What is still there is the
one thing an unread count was standing in for — being told when something you are waiting for
happens — and it happens by being woken.

## Sessions and transcripts

When chat starts a persona it mints an **internal session id** and hands it to the agent runtime;
the runtime answers with its own real session id, and `nxc session bind` maps the two. Everything
downstream — the return address on a reply, the depth guard, the transcript — is keyed on the
internal id, which is what `send --to <persona>` returns as `session`.

A **transcript** is that session's normalized stream: assistant text, thinking, tool calls and their
results, with a `Task`-spawned subagent's entries nested under the call that spawned them. It is the
answer to "what did this agent actually do", where `search` only answers "what did it say".

Transcripts are **device-local and never synced** — they are by far the highest-volume thing chat
produces, and putting them in a shared grow-only log would balloon it for every peer. They are also
retained rather than kept forever: whole sessions age out on their last entry, on a window that
defaults to 30 days and rides the first flush of every new session.

## Operations

An **operation** is the whole tree a first message started: your thread, the threads a channel
opened under it, the threads those opened in turn. `nxc status` is the read over it, and each thread
in it is `open`, `answered`, or `stale`, with one flag beside them:

- **`open`** — somebody still owes a reply. A chain that has *hung* also looks like this.
- **`answered`** — the expectation was declared and discharged.
- **`stale`** — the declared window ran out with something still outstanding.
- **`awaiting_human`** — true only at the **root** of a finished operation.

That last flag exists because a stalled chain and a finished one waiting for a person look identical
to a counter and mean opposite things. It is only ever true at the top, which is the same statement
as: no human stands inside the flow. More on that in
[limits-and-safety](nxc-limits-and-safety).

## Next

- [personas](nxc-personas) — declaring who your agents are.
- [channels](nxc-channels) — declaring how they work together, in parallel or in order.
- [commands](nxc-commands) — the verbs over all of this.
