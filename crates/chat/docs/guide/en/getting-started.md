# Getting Started

`nxc` is the **chat** building block of the nexus suite: the durable channel between you and your
agents. One binary ships three tools — flow (`nxf`) tracks the work, memory (`nxm`) remembers it,
chat (`nxc`) carries the messages about it. This guide takes you from an empty directory to a
persona that answers you.

Chat is a **substrate**, not a chat app. There is no daemon to run and no server to point at: a
message is a row in the same `.nxs/` workspace database the other two blocks write to, and the whole
surface is thirteen commands with `--json` on every one of them.

## Install and activate

Install the suite (one binary, `nxs`, with `nxf`/`nxm`/`nxc` beside it):

```bash
curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

Then activate chat in your project:

```bash
nxc init
```

It creates the `.nxs/` workspace if there is none, registers the `chat` module in it, and prints
what it wired. Four things now exist, and it is worth knowing which is which:

- **`.nxs/`** — the workspace database. Messages, threads, and channels live here, beside flow's
  items and memory's notes. It is one store, so one `nxs sync` moves all of it.
- **`.nxs-personas/`** — the declaration folder. This is where your team is written down. `nxc`
  reads it; nothing but you writes it. **Commit it**, and treat a change to it as a change to your
  code: it is read fresh at every spawn, and it sits in the very working copy your agents work in
  (see [personas](nxc-personas)).
- **`AGENTS.md`** — a short managed block telling any agent that lands in this repo to run
  `nxs prime`.
- **`.claude/settings.json`** — one `SessionStart` hook per active module, chat's running
  `nxc prime`, so a fresh session is told who it can address and what finished while it was away,
  without anyone having to remember to ask.

Already running flow or memory? `nxs init --module chat` adds chat to the workspace you have — the
`.nxs/` directory is shared, never duplicated. Going the other way, `nxs init` sets up all three at
once and is the interactive entry point.

## Declare somebody to talk to

**There is no `nxc agents register`, and no `nxc channels create`.** A team is *declared*, in files
you can read, review, and commit — not registered at runtime into one machine's database. So the
first step is to write a persona:

```bash
cat > .nxs-personas/coder.yaml <<'YAML'
handle: coder
job_title: Coder
job_description: Implements a work order on a branch and merges it.
system_prompt: |
  You are the coder. Do what the trigger message asks. An answer from you means it is done; if
  you cannot get there, say what you are missing rather than answering.
tools: [Bash, Read, Write]
YAML
```

`handle` and `system_prompt` are the only required fields; everything else has a default. The full
field set is in [personas](nxc-personas).

**The running example.** Every worked example in these guides comes from one small workspace: the
`coder` above, two reviewer personas (`general` and `integrity`) that are reachable only through a
channel, and a `channels.yaml` declaring an ordered `build-and-ship` channel and a `review` quorum.
`nxc list` is the read over that folder — what a human checks, and what an app renders as its
directory:

```console
$ NXC_ACTOR=alice nxc list
## Who you can address

Address any of them the same way: `nxc send --to <handle> -` — the `-` reads the message from STDIN, and a one-line message may be an argument instead.

**Coder** (handle: `coder`) — Implements a work order on a branch and merges it.

**Build-And-Ship** (handle: `build-and-ship`, members: coder, review) — the declared order a work order runs through

**Review** (handle: `review`, members: general, integrity) — the review quorum — one round asks both reviewers and hands back one verdict

```

The two reviewers are absent from that list on purpose: each declares `addressable: none`, so the
way to reach either is to address the channel that casts them. A directory shows what you can
address, not everyone who exists.

## Send the first message

```console
$ NXC_ACTOR=alice nxc send --to coder --ref nxf_ids=ab12.0007 "Add a --since flag to the export command."
-> coder · thread m-00000000000000000000000001

  The answer lands in this thread.
    nxc threads show    m-00000000000000000000000001    — the conversation
    nxc status --thread m-00000000000000000000000001    — where it stands

  To watch instead of coming back: add --stream next time.

```

**The block under the line is how you learn the answer**, and it is the whole of what a requester
needs: nobody will wake a human at a terminal, so the two reads are what you come back to. In
`--json` the same content arrives as fields under `await` — `poll` is the exact argv to run,
`done_when` is the state that means finished (ALL of it: `complete: true` AND an empty
`outstanding`), `stopped_when` names the two markers that each mean no answer is coming
(`stale: true` OR `escalated: true`), `answer_at` says where the answer sits in that payload, and
`deadline` says how patient to be. A loop that waits for `done_when` alone hangs forever on a round
that stopped.

A registered persona gets the OPPOSITE advice in the same field: `"how": "resume"`, `"poll": null`.
It is resumed by the coordinator when the answer arrives — together with everything else that
arrived while it was working — so polling would be a session burning turns on a question it is
about to be handed.

Three things happened in that one call: the message was posted, a **thread** was stamped on it, and
the `coder` persona was started on a fresh session with your message as its task. The thread id is
the one value to keep — it is the address of the conversation.

`--ref` says what the conversation is *about*, and it is mandatory in a specific sense: saying
nothing is one of the three possible answers, and the only one that warns. Name a subject with
`--ref nxf_ids=<item>` (repeatable), `--ref branch=…`, `--ref pr=…`; or answer `--no-ref` when there
genuinely is none. Say neither and the message is still posted — it is worth more posted than lost
— but the receipt carries the warning as a field, so an app sees it and not just a terminal nobody
is watching.

The thread's board says who owes an answer:

```console
$ NXC_ACTOR=alice nxc threads show m-00000000000000000000000001
thread m-00000000000000000000000001 in dm:9445dbc93dfdad401585c42b
  expects:     ab12/coder
  replied:     
  outstanding: ab12/coder
  complete:    false
  working tree: holding
  · ab12/alice Add a --since flag to the export command.

```

`ab12/coder` is a **qualified handle** — `<origin>/<agent>`, where the origin is this workspace's
own replica prefix. Senders, expectations and claims are all written in that form; see
[core-concepts](nxc-core-concepts).

## The other side

The persona never asks for that message: `send` started its session with the message in the prompt.
It does the work and finishes by answering the thread. If it wants the conversation around it —
after a context compaction, say — `nxc threads show <thread>` is the read, and it is the same board
the requester sees:

```console
$ NXC_ACTOR=coder nxc threads show m-00000000000000000000000001
thread m-00000000000000000000000001 in dm:9445dbc93dfdad401585c42b
  expects:     ab12/coder
  replied:     
  outstanding: ab12/coder
  complete:    false
  working tree: holding
  · ab12/alice Add a --since flag to the export command.

```

```console
$ NXC_ACTOR=coder nxc reply --thread m-00000000000000000000000001 "Added the flag and a test; branch feat/export-since." --json
{"posted":true,"message_id":"m-00000000000000000000000002","thread_id":"m-00000000000000000000000001","resumed":false,"warnings":[],"await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000001","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}

```

That reply is what moves everything on: it discharges the persona's turn, ends its session, and
hands the turn back to whoever opened the thread. There is no separate "done" verb and nothing else
to remember — which is exactly what makes the loop teachable to an agent in one sentence.

## Where it stands

`nxc status` reads an operation as a whole: the thread tree from its root down, across channel
borders. The round above is answered, so the plain listing has nothing to report — it shows what is
still going on, and `--all` is where a finished operation is:

```console
$ NXC_ACTOR=alice nxc status --all
operation m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1 thread(s), 0 open  · finished
  m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  awaiting you (answered by ab12/coder)

```

"Awaiting you" is the normal end of an operation — the agent side is finished and a human has not
acted on it yet. It reads differently from a thread that has stalled, deliberately: telling those
two apart is the whole reason the flag exists ([limits-and-safety](nxc-limits-and-safety)).

## What `nxs prime` contributes

At the start of a session every active module's own `prime` runs, each from its own hook.
`nxs prime` is the same fan-out when you ask for it by hand. Either way, chat's block carries three
things:

1. **The coordination rule** — agent-to-agent coordination in this workspace goes through `nxc`;
   ad-hoc notes and scratch files are not delivered, not synced, and never replayed.
2. **The core verbs**, spelled with their required flags, so a session does not have to guess the
   shape of `send --to` or `reply --thread`.
3. **What this workspace declares** — who can be addressed here and what for — and, for a caller
   whose previous session this device saw end, the commissions it opened that finished in between.
   No message text: everything a session is sent arrives by being pushed into it, so replaying it
   at the start would be a second copy (it carried the whole unread inbox until nxf 6j6v.4mmk
   stopped rendering it and nxf 6j6v.4d2z removed it).

You never type it yourself: since nxf n2m6 + a2a1 the host wires one `SessionStart` hook per active
module, so `nxc prime` is run directly rather than through an umbrella fan-out. Re-run `nxs prime` by hand after a context compaction — this
guide is where that reminder lives now, not chat's own block (nxf h4d3, task 3 dropped the
"Context Recovery" line from the rendered block to fit the session-start budget).

## Next

- [core-concepts](nxc-core-concepts) — the substrate, identity, threads, and delivery.
- [personas](nxc-personas) — everything a `.nxs-personas/<handle>.yaml` can declare.
- [channels](nxc-channels) — declaring a group, and the ordered channel that *is* a workflow.
- [commands](nxc-commands) — the full reference, with `--json`.
- [limits-and-safety](nxc-limits-and-safety) — what an agent does not decide for itself.
