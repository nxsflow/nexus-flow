# Commands

The complete `nxc` surface: thirteen commands, in the order a session tends to reach for them. Run
`nxc <command> --help` for the generated reference (flags, types, defaults); this guide adds the
narrative and the `--json` shapes. Two flags are global — `--json` on every command, and `--db
<path>` (also `NXC_DB`) to point at a workspace database instead of discovering `.nxs/` from the
working directory.

The surface is small on purpose. There are exactly **two ways to say something** — `send` starts a
conversation, `reply` answers one — plus `withdraw`, with which the person who sent a commission
takes it back, queued or running. Everything else reads. If you are looking for a verb that
creates a channel, registers an agent, advances a workflow step, or gives a held working copy back
by name, see [what is deliberately absent](#what-is-deliberately-absent) at the end.

## Set up

### `nxc init`

Activate chat in the current directory: ensure a `.nxs/` workspace, register the `chat` module, then
hand the shared agent file and the SessionStart hooks to the `nxs` assembler,
which re-assembles them for *every* active module. `--quiet` does the same and prints no banner —
that is the seam the umbrella drives, and the non-interactive entry for an agent.

```bash
nxc init                 # human, with the banner
nxc init --json          # the machine-readable receipt
```

`--quiet` is silent success — it prints nothing at all, which is what makes it drivable:

```console
$ nxc init --quiet

```

### `nxc agent-manifest`

What chat declares it contributes to the shared agent file — the prime command and the hook — as
data rather than as prose:

```console
$ nxc agent-manifest
nexus-chat agent manifest
  prime command:  nxc prime
  hook:           SessionStart → nxc prime

Run with --json for the machine contract the `nxs` umbrella assembles from.

```

`--json` is that contract; the umbrella reads it from all three blocks and assembles one
`AGENTS.md` and one hook, so activating a second module never overwrites the first one's block.

## Who is here

### `nxc list`

The directory: who can be addressed in this workspace, and what for. It is a read over
`.nxs-personas/` — nothing else — so it shows exactly what is declared.

```console
$ NXC_ACTOR=alice nxc list
## Who you can address

Address any of them the same way: `nxc send --to <handle> -` — the `-` reads the message from STDIN, and a one-line message may be an argument instead.

**Coder** (handle: `coder`) — Implements a work order on a branch and merges it.

**Build-And-Ship** (handle: `build-and-ship`, members: coder, review) — the declared order a work order runs through

**Review** (handle: `review`, members: general, integrity) — the review quorum — one round asks both reviewers and hands back one verdict

```

Without `--persona` a human sees the whole declared team, while a persona running under its own
session sees its own address book, recognised from that session. `--persona <handle>` projects that
persona's view whoever is asking — useful for checking what an agent will actually see before you
start it.

`--json` carries `personas` (`handle`, `job_title`, `job_description`, `direct`), `channels`
(`name`, `members`, `description`), a `public_channels` array of the front doors it can see
(omitted when there are none, and absent from the human rendering on purpose — a door no
declaration here names is discoverable, not addressable), and a `declarations` block naming the
folder it read and how many files were in it. That last block is the one to check when the list is
emptier than you expected.

## Say something

### Where `<BODY>` comes from

Both verbs take the message three ways, and the notation is `nxf`'s own (`--description-file`,
`--set field=-`):

```text
nxc reply --thread <id> "lgtm"                # an argument — for ONE line
nxc reply --thread <id> - <<'EOF'             # STDIN — for everything else
<your text, exactly as it should arrive>
EOF
nxc reply --thread <id> --body-file verdict.md   # a file; a path of `-` is STDIN again
```

**Anything longer than a line belongs on STDIN, and that is not a style note.** In the argument form
the *shell* gets the text first: backticks and `$(…)` in it are executed and replaced before `nxc`
sees a byte. The longest texts this system produces are exactly the ones full of both — a review
verdict quoting commands and their output, a session report, an evidence trail. At best such a body
arrives silently mangled; a role that is *quoting somebody else's* text runs it. On the STDIN and
file paths the text passes no quoting at all, and the quotes around `EOF` are what stop the heredoc
itself from expanding (`<<EOF` without them still does).

An empty read is refused: `nxc reply --thread <id> -` with nothing on STDIN would post an empty
answer and end the round with no result, so it fails instead of posting.

### `nxc send --to <persona|channel> <BODY>`

Open a conversation. `--to` is required, and its target must be **declared**: a persona handle or a
channel name from `.nxs-personas/`. The declaration decides what happens next — a persona is started
on a fresh session, a channel fans out under its own policy — and the call returns the thread id
either way.

```console
$ NXC_ACTOR=alice nxc send --to coder --ref nxf_ids=ab12.0007 "Add a --since flag to the export command."
-> coder · thread m-00000000000000000000000001

  The answer lands in this thread.
    nxc threads show    m-00000000000000000000000001    — the conversation
    nxc status --thread m-00000000000000000000000001    — where it stands

  To watch instead of coming back: add --stream next time.

```

Flags:

- **`--ref k=v`** (repeatable) — what this conversation is about: `nxf_ids` (a flow item; repeatable
  because a first message may name several), `branch`, `pr`, `session_id`.
- **`--no-ref`** — the explicit answer that there is genuinely no subject. Saying "I looked" where
  saying nothing says nothing.
- **`--stream`** — watch the thread *and* the personas' transcripts until somebody answers, instead
  of returning immediately. For a human at a keyboard; refused inside a running persona.
- **`--body-file <path>`** — read the body from a file instead of the argument (UTF-8, verbatim); a
  path of `-` is STDIN. See *Where `<BODY>` comes from* above.
- **`--machine <id|name>`** — run this chat on that machine (a persona chat only). Without it the
  persona's `machine:` decides, else this machine. A machine that is not online is **asked about**:
  nothing is sent, and the refusal lists the machines that are online. A machine named here is
  honoured even when it is not online — the chat waits in the log until it is back. When the chat
  runs elsewhere the receipt says so (`handed_to`), and nothing starts on this machine. See
  [personas](nxc-personas) for where the machine comes from.

Naming neither `--ref` nor `--no-ref` still sends, and warns — as a line on stderr for a reader
**and** as a `refs_warning` field for an app, so the reminder cannot be lost between the two:

```console
$ NXC_ACTOR=alice nxc send --to coder "One more thing." --json
{"thread_id":"m-00000000000000000000000002","message_id":"m-00000000000000000000000003","to":"coder","target":"persona","channel":"dm:9445dbc93dfdad401585c42b","session":"m-00000000000000000000000002","expects":["ab12/coder"],"spawned":true,"warnings":[],"refs_warning":"nothing was named as the subject of this conversation. Say what it is about — `--ref nxf_ids=<id>`, repeatable — or `--no-ref` if there really is nothing.","await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000002","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}
warning: nothing was named as the subject of this conversation. Say what it is about — `--ref nxf_ids=<id>`, repeatable — or `--no-ref` if there really is nothing.

```

Sending to a channel returns the same receipt with `target: "channel"`, the declared channel's id
(`decl:<name>`), and the channel supervisor as the expected replier:

```console
$ NXC_ACTOR=alice nxc send --to build-and-ship --ref nxf_ids=ab12.0008 "Ship the export change." --json
{"thread_id":"m-00000000000000000000000003","message_id":"m-00000000000000000000000004","to":"build-and-ship","target":"channel","channel":"decl:build-and-ship","expects":["ab12/__channel__"],"spawned":true,"warnings":[],"refs_warning":null,"await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000003","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}

```

The receipt's fields, in the order they are emitted: `thread_id`, `message_id`, `to`, `target`
(`persona` or `channel`), `channel`, `session` (the internal session id, when a persona was
started), `queued_behind` / `queue_position` (present only when the trigger is parked behind a
working-copy lease), `expects`, `deadline` (when the channel declares a `timeout:`), `spawned`,
`warnings`, `refs_warning`. `warnings` and `refs_warning` are always present — even empty and even
`null` — so a reader never has to tell "no key" from "nothing to report".

A target no declaration names is refused, and the refusal says what was searched:

```console
$ NXC_ACTOR=alice nxc send --to nobody --ref nxf_ids=ab12.0008 "Anyone there?" --json
? 1
{"error":{"kind":"not_found","msg":"no such target: nobody (not a declared channel, not a declared persona, not a channel in this workspace) — `nxc list` shows what can be addressed"}}

```

So is a persona that declared it is reachable only through a channel — for everyone alike, human or
agent, named or anonymous:

```console
$ NXC_ACTOR=alice nxc send --to general --ref nxf_ids=ab12.0008 "Have a look?" --json
? 1
{"error":{"kind":"validation","msg":"general is addressable only through review — send --to review instead"}}

```

### `nxc reply [--thread <THREAD>] <BODY>`

Answer in a thread. A thread id identifies a conversation on its own, with no channel context, and
the reply is routed back to whoever is on the other side of it.

**`--thread` may be left out while exactly one conversation is open for you.** That is the ordinary
case for a commissioned role, and it is the whole point: a role knows what it has to do and whom it
may answer, so carrying an id it was handed minutes ago is bookkeeping it should not be doing.
`nxc reply -` answers the one conversation waiting for you.

With more than one open it is required again — you owe your commissioner an answer *and* the role
you consulted has come back to you, and the engine will not guess which you meant. It does not have
to be worked out: the message that woke you names each open conversation with the command it takes,
and so does the refusal if you try the short form anyway. The id-bearing form is always allowed.

```console
$ NXC_ACTOR=coder nxc reply --thread m-00000000000000000000000001 "Added the flag and a test; branch feat/export-since." --json
{"posted":true,"message_id":"m-00000000000000000000000002","thread_id":"m-00000000000000000000000001","resumed":false,"warnings":[],"await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000001","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}

```

`resumed` is `true` only when the reply woke the *target's own* return-address session — the direct
one-to-one path. `false` covers two different things (there was no session to resume, or there was
and the resume did not land), and `wake_skipped` beside it is what separates them. A quorum board's
completion wake is not a "resume" in this sense and never sets it. `posted` is `false` only in the
`--if-unanswered` no-op below.

- **`--escalate`** — "I cannot reach the result on my own: I need help, or a decision." One of the
  three things an agent may say with `reply`; the others are "I am finished" (a bare reply) and
  `--needs-rework` below. It is a *declared* signal, which is what makes it usable: a channel's
  supervisor branches on a bit whose meaning is written down, not on a phrase it has to interpret.
  The turn is discharged either way — what differs is the outcome.

  **The decision half is the one that gets missed.** The flag is not only for a task you are
  blocked on. A question you could act on either way, but that is not yours to settle, is exactly
  what it carries — and the direction is what makes it worth a flag: an escalation goes UP, to
  whoever commissioned you. Put the same question in a plain reply and it travels DOWN to the next
  step, which has no more standing to settle it than you had.

  **On an ordered channel the outcome is that the chain ends.** The step after yours is not started,
  the round goes up as it stands, and whoever commissioned it decides. That is the point of the flag:
  a task that could not be carried out must not commission the work that was to follow it.

  **It is not a way to say "not yet", and it is not how you wait.** Every answer `reply` has ends
  the turn, and a caller reads this one as work that will not arrive. If you have commissioned a
  round of your own and are waiting for it, end your turn and say nothing at all: the engine sees
  the round you opened, treats that as waiting rather than as a missing answer, and wakes you when
  it returns. A plain `nxc reply` while your own round is open is refused — see
  `waiting_on_sub_round` under `nxc status` below, and [channels](nxc-channels).
- **`--needs-rework`** — "what you handed me does not meet the standard." The third answer, and the
  one about **somebody else's** work rather than your own. Name what must be put right in the body:
  those words are handed to the party that produced the work, and they are its next task.

  **It is offered only where the channel declares somewhere for it to go** (`steps:` →
  `on_needs_rework:`) — that is what makes it a signal rather than a token you invent. A step without
  that edge is told about two endings, not three; set on such a step the bit falls away, with a
  `verdict_dropped` warning on the reply's own receipt. It may not be combined with `--escalate`: the
  two are statements about different work with opposite consequences. See `nxc guide channels`.
- **`--accept`** — "I know what the review said. This goes on anyway." The one answer on this verb
  that is **not** an agent's statement about work in front of it: it belongs to the party that
  **commissioned** a round, said on that round's own thread.

  It overrules the verdict that stopped the round. The step whose `--needs-rework` sent the work back
  counts as satisfied, and the run continues over that step's `next:` — the *same* run, not a new
  one. The next step is handed your words together with the verdict you set aside, so it knows the
  judgement was overruled rather than met, and cannot report downstream that the work was reviewed
  clean.

  **Without it a cycle whose reviewer never becomes satisfied has no exit.** `max_passes` bounds one
  run; answering the round any other way commissions a new run and re-arms the same ceiling. That is
  not a corner case — it is what happened the first time this engine ran a planning chain for real:
  six draft→review passes, six verdicts, and nothing handed on.

  **You may not accept your own reviewer.** It is honoured only on a thread you opened; on a slot you
  are serving it is refused by name, because the separation between producing and checking is the
  whole reason a round has two parties. What an agent does instead is `--escalate`, which asks the
  party above it to decide — and this is the verb that party then has.

  **Refused rather than posted when it would move nothing**: no such round, not yours, no verdict to
  accept, or a step of the round still working. Each refusal names what is missing and what to do
  instead. It is recorded as its own kind (`accepted`), so an override stays findable afterwards as
  something other than a review that passed.
- **`--if-unanswered`** — post only if the caller still owes a reply on this thread; otherwise a
  deliberate no-op (exit 0, `posted: false`, nothing written), never an error. It exists for one
  caller: the agent sidecar's teardown, which says something when an SDK session ends without ever
  answering the thread it owed, so the thread does not go quiet forever. You do not need it.

  **It is the fallback, not the first move.** Before speaking in the agent's name, the teardown
  gives the session its turn back once and tells it — naming the thread, both ways to end a turn,
  and any round it commissioned that is still open. Most sessions answer then. One that is reminded
  and ends silent again is handed back as an **escalation**: at that point no result exists and
  somebody has to decide what happens to the round, which is a different fact from a turn that
  simply produced nothing, and `escalated: true` on `nxc status` is where the caller reads it.

  **Whatever it posts is marked as the runtime's.** Any reply written through this door carries
  `substituted: true` on `nxc status`, so an app never has to read a message body to tell an answer
  the agent wrote from one written in its name. See
  [limits and safety](nxc-limits-and-safety) for the pairing with `escalated`.
- **`--body-file <path>`** — as on `send`: read the body from a file (`-` is STDIN). See *Where
  `<BODY>` comes from* above.
- **`--stream`** — as on `send`: watch the answer arrive.
- **`--machine <id|name>`** — hand this chat to another machine. The persona is started there with
  what is owed since its last answer, and told where the conversation so far is; this machine starts
  nothing. A person's decision: refused from inside a running session. A reply into a chat whose
  machine is not online is asked about the same way `send` asks.

There is no `--kind`, no `--priority`, no `--disposition` and no `--ref` on `reply`. A reply
inherits its subject: the thread already says what the conversation is about, which is why the
`--ref` obligation sits on `send` and nowhere else.

### `nxc withdraw --thread <THREAD>`

Take back a commission — one still **waiting for the working copy**, or a round that is
**running**.

**It is a person's verb.** Only the caller who sent the operation's first commission may take it
back, and only by naming that thread — the one their first `send` handed back. Everything else is
refused before anything changes, by name:

- **A session this workspace started** — an agent's own `nxc` — is refused whatever it names, and
  the refusal names the session, the role it was started for, and the way it has instead: `nxc
  reply --thread <its own thread> --escalate -`, which goes up to whoever commissioned it. This is
  what the engine can tell from its own session records, and all it claims: a process that hides
  its session is not caught here, and the rule after this one checks the identity a caller
  presents, so it is no second barrier. What stands then is that the work is parked rather than
  lost, and that the act is on the record.
- **A caller who did not open the operation** is told who did (`… did not open operation <id> —
  local/carsten did, with the nxc send that began it`).
- **A thread below the first one** — a step of a channel round, a sub-round an agent opened — is
  refused with the operation's first thread named instead (`nxc withdraw --thread <root>`).

On success it prints one line per commission it took back (`withdrew <role> on thread <id> (never
started)`), one per session it stopped (`stopped <role> on thread <id> (session <id> was asked to
end)`), one saying what becomes of a stopped round's work, and one saying the chain is interrupted;
`--json` carries the same as `withdrawn` / `started_meanwhile` / `stopped` / `will_park`, plus
`warnings` when something did not land.

**A commission that never started.** A persona or channel that declares `working_tree: exclusive`
runs one chain at a time; a commission that arrives while another chain is holding is **parked** in
a queue and starts when the holder lets go. Until then nothing has happened: no session, no
transcript, no model call. Taking it back loses nothing.

**A round that is running.** Its thread is discharged and its session is asked to stop — a request
delivered to the process, which then ends its own way (the shipped sidecar aborts the turn, saves
its transcript and announces its end). If the round holds this workspace's working copy and a park
of it can produce a branch here, the receipt says `will_park: true`: whatever the session left
uncommitted is **parked on a branch** by the background service once nothing in that operation is
running any more — tracked and untracked files
alike, ignored ones left where they are — and the working copy goes on to whoever is waiting. Nothing is ever rolled back.
`nxc status` lists the branch under the operation (`parked on <branch> at <commit> since
<instant>`) until somebody comes back for it, and the way back is **the next commission into the
same thread**: on a direct persona thread, `nxc reply --thread <id>` resumes the stopped session
with its own transcript; for a channel round, a follow-up into the channel thread starts the next
pass. Either way the tree is put back on the park branch and the session is told, in a fixed text,
which branch and which commit its work is on. A fresh `send` opens a new claim and does not find
it. A round that holds no working copy — a shared persona — is stopped and `will_park` is `false`.

**And when the work is in the checkout but cannot be parked at all**, the receipt says `will_park:
false` with `cannot_park` naming the reason: this host's chat worker runs sessions in no directory
it can name, the directory is not a git repository, or the operation never recorded a base. The
working copy is still handed on once nothing in that operation is running — unparked, with the
`work_handed_on_unparked` warning, and the work left in the tree. Nothing is rolled back there
either. `nxc withdraw` prints that case in its own words, and both park lines name `nxc tick
--thread <id>`, which does the park (or the unparked hand-off) by hand where no background service
is running — and says so with a `service_not_running` warning when there is none.

A host whose chat worker **cannot stop a session** refuses the whole call before changing
anything, by name (`this host cannot stop a running session; nothing was withdrawn`). A host whose
worker cannot even *tell* whether a session is running is refused by name as well, naming the
sessions it cannot answer for — rather than reporting that there is nothing to withdraw, which would
be a statement about processes nobody looked at. A stop the worker supports and then could not
deliver — a pid file gone, a claim that cannot prove which process it names, a signal refused —
leaves the thread discharged all the same and is reported as a `session_not_stopped` warning naming
the session; the command then exits non-zero, because the process may still be running and that is
the one fact a script has to learn.

A session that had **already ended** by the time the stop was sent is not that case. A round that
was about to finish anyway, or one whose process is long gone and whose pid the system has handed to
something else, is exactly the state you asked for: the receipt says so in words, there is no
warning, and the command exits 0. Nothing is ever signalled to a process this workspace cannot prove
is the session's own.

A session that is **still there five minutes after the stop** gets one further `SIGTERM` from the
background tick — once, through the same identity check, never more and never `SIGKILL` — and the
tick's warnings name it with its pid and pid file (`withdrawn_holder_wedged`); `nxc status` shows
the operation `WITHDRAWN` with the sessions still pinning it. Past that, ending it is yours to do by
hand. Know what that further signal can do: the shipped agent sidecar is already stopping when it
arrives, so it notes it and changes nothing — the signal helps a host's own worker whose first stop
was lost or ignored, and for the sidecar the warning is the part to act on. A session you put back in motion yourself — a follow-up into the withdrawn thread before the
park — is your work again, and is neither signalled nor named.

Everything under the thread you name is withdrawn, and each thread is discharged with a message
saying who took it back and what became of its work — so it is a record in the conversation, not an
absence.

**A withdrawal interrupts the chain below it.** Every thread between what was taken back and the
thread you named that is still waiting — a channel thread that owes its round's result, a persona
thread waiting on the round it commissioned — is discharged the same way, with a message saying the
chain was interrupted, and stops counting as open in `nxc status`. So nothing below it moves on: an
ordered channel commissions neither the next step nor the step that was taken back, whether it
declares `flow: sequential` or `steps:`, not after the park either; and nothing is consolidated,
because there is no answer to deliver. The answers steps had already given stay in their threads.
The work comes back when you commission the same thread again, as described above.

If a commission starts between this command reading the queue and writing to it, it is reported as
`started_meanwhile` and left strictly alone — the next call finds it running. The chain above it
is interrupted all the same, so it runs on its own, and the closing line says so: it names the
exception and the call that stops it too (`nxc withdraw --thread <id>` once more).

A thread with nothing waiting and nothing running under it is a `not_found` saying so.

### `nxc resume --thread <THREAD>`

Take up an operation that stopped because **the model was not available to it** — an exhausted
quota, a provider that could not be reached, a broken connection. Three causes, one state, and it is
the state in which nothing is broken and nobody did anything wrong.

Such a round is **not handed back**. The thread still owes its answer and the session is waiting to
give it, so `nxc status` marks the operation `INTERRUPTED` rather than `NEEDS DECISION`, the row
says which window was hit and when it lifts, and `nxc session state` says the same about the session
itself. That the fact is readable at all is the larger half of this: before it, a session that ran
into a weekly window reported a clean `ended` and its thread reported `answered`, so the one thing
nobody could establish was that anything had happened.

**The background service does this by itself** once the boundary falls — it is the only clock in the
system, and the runtime states the reset instant. Run the verb by hand to go earlier: `--force`
starts it although the window is not up, which is what you want with a second account. If the
runtime named no instant at all, nothing will take the operation up on its own and the line says so.

**The session continues with its own transcript**, rather than starting over. That is not a
convenience, it is the safety argument: the transcript carries every command the session ran *and
what that command returned*, so a session that already created a ticket sees the id it got back and
does not create it twice. A restart from nothing does exactly that. Tool calls are never replayed.

**It checks the world before it starts anything**, and most of what it prints is a decision not to:

- the round was **already answered** before the interruption — nothing to continue, and the hold is
  closed. This is the common case rather than a corner: a limit strikes the expensive turns, and an
  expensive turn is usually one that has just finished;
- a process is **already running** for that session — one session, one process, so it is left alone.
  This is what keeps a hand-run resume from colliding with the service arriving at the reset time;
- the **working copy moved** while the operation waited — a question goes back to the thread that
  started the operation, and nothing is started. The machine does not decide that one;
- this workspace's **runtime cannot continue** a conversation it began — refused by name, because
  continuing would silently open a fresh session with none of this one's history.

While the window is closed the operation's uncommitted work is **committed onto a park branch** and
the working copy handed on: a weekly window is up to a week, and holding this workspace's only
checkout for a week would stop everything else. Resuming brings it back, and says whether the base
moved underneath it. A park that is refused follows the rule in
[limits and safety](nxc-limits-and-safety): a tree in the middle of a merge keeps the checkout,
shows `PARK REFUSED` on `nxc status`, and is parked by the next tick after the merge is finished
while somebody is waiting; a workspace that cannot be parked in at all hands the checkout on
without parking.

What is outside all of this, in both directions: **ignored files** — `.env`, local databases,
`node_modules` — are seen by neither the anchor nor the park. And the repository is what rewinds;
the record does not. Tickets that were created stay created, and messages that were posted stay
posted.

`--session <ID>` names the interrupted session directly instead of a thread. A person names a
thread, because that is what `nxc status` puts in front of them; the scheduled job names the session,
because the hold is keyed on it.

## Read

None of these write anything, and none of them consume a message. `--consumer <handle>` reads as
another qualified handle; without it, a command reads as the caller's own.

> **`nxc inbox` and `nxc read` were here, and both are gone** (nxf 6j6v.1gm9). There is no verb for
> asking after your own messages, and none for acking them, because **people pull and agents get
> pushed**: an agent's message arrives in the prompt that starts its session, or in the turn that
> resumes it, so a pull verb was the duplicate of what had already been delivered — measured at 0
> uses in 66 role sessions for each of the two. What a person reads instead is the CONVERSATION,
> which is what the two verbs below are for. The unread RECORD outlived the two verbs by a
> fortnight, travelling on the app seam as `prime --json`'s `in_turn`/`next_session`/`count`, and
> then went too (nxf 6j6v.4d2z): nothing ever read it and nothing ever acked it, so the read cursor
> underneath it was a write path with no purpose.

### `nxc threads list` / `nxc threads show <THREAD>`

The quorum boards. `list` is every board the caller is a member of, each with its bulk state;
`--channel` narrows it.

```console
$ NXC_ACTOR=alice nxc threads list
m-00000000000000000000000003  decl:build-and-ship  0/1 in  [waiting]  · waiting for working tree (#1)
m-00000000000000000000000004  decl:build-and-ship  0/1 in  [waiting]  · waiting for working tree (#1)
m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1/1 in  [complete]
m-00000000000000000000000002  dm:9445dbc93dfdad401585c42b  0/1 in  [waiting]  · holding working tree

```

`show` is one board in full — the quorum state plus the replies in order:

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

The `--json` records carry `thread_id`, `name`, `channel_id`, `opener`, `expects`, `replied`,
`outstanding`, `complete`, `stale`, `working_tree` and `working_tree_queue_position`. `name` is what
the conversation is CALLED (see `nxc threads name` below) and is absent for a thread nobody named. Reading many
boards costs the same number of queries as reading two — that property is measured by a test, not
merely intended, because a coordination UI reads all of them at once.

### `nxc threads name <THREAD> [<NAME>]`

What a conversation is **called**. A direct conversation's id is a hash over the two handles — it is
derived that way so both sides reach the same id without agreeing on one first — and a hash is not
something a person can read, so `nxc send` gives the thread it opens a short display name of at most
seven words, derived from the message you sent. The name rides on every thread read above, so an app
can show *what a conversation is about* instead of `dm:d9ab4f601a08…`.

That derivation runs out of band, in a run the coordinator commissions — it never holds up or fails
a `send`, and a thread nobody could name stays an ordinary thread that every surface renders exactly
as before. This verb is the same act by hand: with a `<NAME>` it states one, without one it derives
one from the thread's opening message.

**A thread is named once.** A thread that already has a name keeps it and the receipt says
`named: false` — that is a no-op, not an error, because a name that moves under a reader is worse
than no name at all. It is a display name and nothing keys on it; the id is still the id.

Deriving a name reads the thread's opening message, so this verb answers to the same membership rule
`nxc threads show` does: a conversation you may not read is one you may not have named either.

### `nxc machine --to <PERSONA> | --thread <THREAD> [--machine <M>]`

Which machine a chat runs on — a new one with a persona, or one that exists — and whether sending
now would ask instead: the machine, where that came from (`choice`, `chat`, `persona` or
`started_here`), whether it is online, and the machines that are. `--machine` shows what a choice
you are about to make would do. It writes nothing; `send` and `reply` make exactly this resolution
before they write, which is what lets an app render the choice first.

### `nxc status`

Where an **operation** stands: the whole thread tree from its root down, across channel borders. A
board tells you about one conversation; `status` tells you about the whole piece of work it started.

**It answers one question: is anything still going on here?** So a finished operation is not in it.
The round above is answered, and the plain listing says so by staying quiet:

```console
$ NXC_ACTOR=alice nxc status
nothing open (`nxc status --all` also shows finished operations)

```

```console
$ NXC_ACTOR=alice nxc status --all
operation m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1 thread(s), 0 open  · finished
  m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  awaiting you (answered by ab12/coder)

```

```console
$ NXC_ACTOR=alice nxc status --all --json
{"operations":[{"root":"m-00000000000000000000000001","channel_id":"dm:9445dbc93dfdad401585c42b","live":false,"open":0,"needs_decision":false,"holds_working_tree":false,"interrupted":false,"threads":[{"thread_id":"m-00000000000000000000000001","channel_id":"dm:9445dbc93dfdad401585c42b","depth":0,"state":"answered","awaiting_human":true,"escalated":false,"substituted":false,"opener":"ab12/alice","expects":["ab12/coder"],"stale":false}]}],"worker_answers_liveness":false,"worker_names_a_working_copy":false}

```

- **`--thread <id>`** — the one operation that thread belongs to, shown from *its* root, whether it
  is still running or already finished.
- **`--channel <name>`** — the live operations whose root sits in that channel; a declared channel
  name or a raw channel id. An entry point, not an anchor: an operation crosses channels, and the
  tree follows it.
- **`--all`** — the finished ones too, marked `· finished`. It takes `--channel` with it, so
  `--all --channel review` is that channel's history.
- **none of them** — everything still going on here.

**What "still going on" means, exactly**: the operation has an open thread, or an unanswered
hand-back, or this device's working copy, or a dead end that *failed* — a thread whose answer
arrived and whose consequence never did (`ORPHANED`, marked `· dead end` on the operation line).
Two more keep it listed once everything else about it has settled: **work parked on a branch that
nobody has come back for** (a line `parked on <branch> at <commit> since <instant>` under the
operation, `parked` in `--json`), and **a park that was refused and is being retried** (the mark
`· PARK REFUSED`, a line `park refused (<refusal>): <what to fix> — retrying since <instant>`,
`park_refused` in `--json`). Those are the things a reader can act on, and together they are the
`live` flag on every operation the command prints. Note what is *not* on that list: a root
that has been answered and not yet read. That is `awaiting_human`, and it used to keep an operation
listed — which meant nothing ever left the view, because the flag is derived from facts that never
stop being true (the root asked; the root was answered). After a day of real use the view showed
thirteen operations and 424 lines, nine of them with nothing open at all. `awaiting_human` is still
reported, on `--thread` and `--all`, where it says what it always said: this one is finished and it
is yours.

**The rows are trimmed the same way.** In the two listing forms the terminal prints only the threads
that say *why* their operation is still there — the root, anything not `answered`, a hand-back, a
thread holding or waiting for the working copy — and counts the finished middle of the chain in one
line. That is what makes the view fit on a screen: in the workspace this was measured on, six live
operations were 328 lines with every row printed and 32 with the rest counted, because four of them
were alive on one handed-back thread apiece and carried a hundred answered ones behind it. `--thread`
and `--all` print the whole tree, and `--json` is never trimmed in any form.

Each thread carries a `state` of `open` / `answered` / `stale`, plus `awaiting_human`, which is
`true` only at the root of a finished operation. That distinction is the point of the field: a
stalled chain and a finished one that is waiting for a person look identical to a counter, and they
mean opposite things.

**And each thread says what became of the session working on it.** `session` names the internal
session of whoever owes the answer; `session_state` says whether it is `running`, `ended`, or
`unknown` — which is the difference between a thread that is thinking and one that is hung, without
a second call:

```text
m-01M0…  #coding  waiting on ab12/coder — ITS SESSION HAS ENDED, nothing is coming
```

The terminal says this only on an **open** thread, because that is where it is a finding; `--json`
carries it wherever there is a session. `unknown` means nobody could answer, and the report says in
the same breath whether that is because the session died without a word or because this runtime
cannot be asked at all: `worker_answers_liveness`. It reads `false` in the example above — this
guide's worked session records its triggers instead of starting real sessions, so there is no
process anywhere to ask about — and `true` under the shipped sidecar.

**And a thread says when the party it is waiting for is itself waiting.** `waiting_on_sub_round`
lists the rounds that party commissioned out of this thread and has not got back; the terminal reads

```text
m-01M0…  #positioning  waiting on 47jy/head-of-marketing — waiting on its own sub-round (4 open)
```

That is the difference between a chain that has stopped and four sessions that are working. Nothing
declares it — the engine opened those threads on that party's behalf, so it is a fact about the
record — and its absence is as meaningful as its presence: an open thread with no such list is one
where nothing further has been handed out. The key is omitted entirely when there is nothing to say.

Two flags sit on the **operation** rather than on a thread, and both answer a question the per-thread
rows can only answer by being read one at a time:

- **`needs_decision`** — somewhere under this root a task was handed back and nobody has taken it
  up. Read it next to `awaiting_human`: both say "the human is up", about situations of completely
  different urgency. A finished operation waiting to be read is the normal end; one whose root looks
  exactly the same while an unanswered escalation sits under it is a chain that has *stopped* — and
  an escalation holds the working copy, so it stops the machine too. It clears itself: re-commission
  the round and the flag goes.
- **`holds_working_tree`** — somewhere under this root the working copy is held. It tells you
  whether to go looking; the per-thread `working_tree` field still says *which* thread.

### `nxc search <QUERY>`

Case-insensitive substring over message bodies in the caller's channels, deterministically ordered.

```console
$ NXC_ACTOR=alice nxc search "since flag" --json
[{"message_id":"m-00000000000000000000000001","channel_id":"dm:9445dbc93dfdad401585c42b","sender":"ab12/alice","body":"Add a --since flag to the export command."}]

```

It searches bodies only, and only in channels the consumer is a member of. For a **declared**
channel that is the `members:` list itself, in both directions and with no `send` in between: written
into the file, you find its boards at once; struck from it, you stop finding them. It is the same
answer `nxc threads show` gives — one membership rule, whichever read you ask.

Membership is not the whole gate: a channel declared `visibility: requester_only` reserves each
member's answer for whoever asked the round, and search obeys that declaration exactly as `nxc
threads show` does — you find the request and your own words, never another member's answer.

**And membership is the whole SCOPE, which is the one place this read is narrower than
`nxc threads show` on purpose.** A `public` channel you never joined is not searched — finding one is
`nxc list`'s job, and reading what is behind the door is its own read. Neither does this follow an
*operation* across channel borders the way `nxc status` and `nxc threads show` do: whoever opened an
operation reads every thread in it, but the text search stays inside the channels that opener is a
member of. To find *what a session did* rather than what it said, read its transcript.

## Sessions and transcripts

These are the role runtime's own plumbing. The agent sidecar writes them; one of them is a read you
will want by name.

### `nxc session bind <INTERNAL> <REAL>`

Bind an internal (nxc-minted) session id to the real Claude Agent SDK session id the SDK handed
back. The sidecar calls it once a session starts. An unknown internal id is reported as `not_found`.

### `nxc session state <SESSION>` / `--thread <ID>`

**Is that session still alive, or is it dead?** The read opposite of the two writes the sidecar
makes on this seam — `session bind` above and the `session ended` it calls last of all.

```bash
nxc session state m-01M0…            # one session
nxc session state --thread m-01M0…   # every session that ran on a thread, ended ones included
```

Three answers, and the third is not a hedge:

- **`running`** — a live process stands behind it.
- **`ended <instant>`** — the session announced its own end, and this is when. That is the fact a
  channel declared `working_tree: exclusive` opens its next step on.
- **`unknown`** — nothing announced an end and no live process answers for it: a session killed
  hard, or one this machine never ran. Reporting `ended` there would state a fact nobody
  established.

`unknown` has a third reading, and the report tells you when you are looking at it. Whether a
session is running is a question the *worker* answers, and not every worker can: one that starts no
process has none to look at. So the answer carries `worker_answers_liveness` — `false` means nothing
here was ever asked, and no `unknown` under it says anything about a session. The shipped sidecar
answers it, so on the normal command line the flag is `true` and an `unknown` really is a session
that died without a word.

Use the session form at a thread that has gone quiet, before you conclude it failed. Use
`--thread` before you touch a working copy a previous step may still have hands on: it lists **every**
session that ran there, ended ones included, and what became of each. `nxc status` answers the
narrower question — the state of the ONE session it names per thread — so reach for this when you
need the whole history of a thread rather than who is on it now. An unknown session is `not_found`;
a thread nothing ran on is an empty answer, not an error.

### `nxc transcript show <SESSION>`

A session's normalized stream, rendered as a timeline: assistant text, thinking, tool calls and
their results, with each `Task`-spawned subagent's entries nested under the `tool_use` that spawned
them. The session id is the internal one — what `send --to <persona>` returns as `session`.

```console
$ NXC_ACTOR=alice nxc transcript show m-00000000000000000000000001
transcript m-00000000000000000000000001  role=coder  (0 entries)

```

`--from-seq <n>` and `--limit <n>` page a long session: pass the largest `seq` the last chunk
showed, and a chunk shorter than `--limit` is the end. An unknown session renders an **empty**
transcript rather than failing — a session whose sidecar never flushed is indistinguishable from one
that had nothing to say.

Two things to know before you paste one anywhere. A transcript contains **raw tool inputs and
results** — whatever the agent read, wrote, or ran — so treat a dump like the workspace database,
not like a message log. And it is deliberately not membership-gated: a transcript has no channel to
gate on, and the table is device-local and never synced, so a gate would buy nothing that anyone
holding the file cannot already do with `sqlite3`.

### `nxc transcript append --session <ID>`

Append normalized entries, read as JSON-lines from STDIN. This is the sidecar's callback contract —
the flag name, the stdin framing and the `--json` record are all load-bearing because an
already-shipped producer depends on them. A line that does not parse is a loud `validation` error
naming the line, never a silent skip.

### `nxc transcript prune`

Retire the transcripts of sessions nobody has written to for a while, and say what went. Whole
sessions, aged on their *last* recorded entry: a session still being written to is never a
candidate however long it has been running, and a long one is never cut in half.

```bash
nxc transcript prune --dry-run          # report, remove nothing, take no write lock
nxc transcript prune --keep-days 7      # tighter than the configured window
```

You do not have to run it to keep the table bounded — the same retention rides the first flush of
every new role session. Reach for it to clear a workspace that has gone quiet, or to apply a tighter
window once. It stops the database growing; it does not make the file smaller (that needs a
`VACUUM`, which is not run here because it rewrites the whole shared workspace under an exclusive
lock). Sessions carrying no readable timestamp are of unknown age, so they are **kept** and reported
separately — a visible gap rather than a silent one.

## Documentation

### `nxc guide [TOPIC]`

The guides you are reading, compiled into the binary. No workspace is needed and no network:

```bash
nxc guide                     # list chat's topics
nxc guide core-concepts       # print one
nxc guide --json              # [{topic, summary}, …] — the agent contract
```

`nxs guide` fans out over the active modules and lists all three blocks' topics at once. Where a
topic name exists in more than one block — `getting-started` and `commands` do — it refuses to pick
for you and names the per-tool commands instead.

## Not verbs you type

Three subcommands exist and are hidden from `--help`, because nobody should type them:

- **`nxc prime`** — the session bootstrap. Chat's own `SessionStart` hook runs it directly, and
  `nxs prime` fans out to it when you ask the umbrella by hand.
- **`nxc tick --thread <id>`** — a clock's hand. The one-shot job a channel's declared `timeout:`
  schedules runs it to re-check a thread and, if due, route it through the channel's `on_complete`
  policy. It is idempotent: a thread whose completion was already handled is a clean no-op, never a
  second wake.
- **`nxc pick-up`** — the executing machine's hand. The background service runs it after a pass
  that left a chat for THIS machine: it starts or resumes the persona of every chat designated here
  that is owed a turn, once per message, and only for messages whose origin this machine trusts. It
  is idempotent.

They are named here so that finding them in a process list or a log is not a mystery. Do not build
on them.

## What is deliberately absent

Half of a trustworthy reference is what it says is *not* there. These verbs existed and were
removed; each entry says what to do instead.

- **`nxc ask`** — folded into `send --to`. A channel is addressed exactly like a persona, and one
  verb that mints a thread beats two that disagree about whether they do.
- **`nxc channels create` / `dm` / `join` / `leave`** — gone with the raw channels. A channel is a
  **declaration** in `.nxs-personas/channels.yaml`; membership is its `members:` list. A direct
  conversation is minted for you the moment you `send --to` a persona, under an id derived from the
  two handles, so there is nothing to create. Cross-project discovery of public channels left the
  agent surface entirely and is an app's job — `nxc list --json` still carries the front doors it
  can see.
- **`nxc agents register` / `list` / `search` / `show`** — a team is declared, not registered. `nxc
  list` is the read, and it searches the same `job_title` / `job_description` fields `agents search`
  did. A profile row registered at runtime lived in one machine's database, nothing reviewed it, and
  it did not survive the run.
- **`nxc workflow start` / `step` / `status` / `tick` / `done` / `bind` / `append` / `show` /
  `expect`** — the declarative run engine is gone, along with the run record. A channel declares its
  own order (`flow: sequential`), `send --to <channel>` starts it, and `nxc status` is where an
  operation's position is read. See [channels](nxc-channels).
- **`send --role` / `--session`** — `--to` is the one way to name a target. `--role` collapsed into
  it; `--session` is a recorded gap rather than a collapse, and is named as such in the source.
- **`send --kind` / `--priority` / `--disposition` / `--model` / `--deadline`, and `reply --kind`** —
  removed together. The first three let a caller answer a question nobody had asked it; the last two
  are declared at the persona and at the channel, and a per-call override beside a declared value is
  two answers to one question. Say how much thinking a job is worth in the persona's `stage:` or
  `model:`, and how long a board may wait in the channel's `timeout:`.
- **`nxc release --thread <THREAD>`** — offered on the agent surface, it could take the working copy
  away from a running coding operation, and on a worker that never implemented its liveness check it
  was an unconditional release with no guard at all. Every hand-off now parks the holder's work
  first: a working copy held by a chain that has died is parked and handed on by the background
  service on its own — past its two-hour bound, or inside it once the chain is provably dead — and a
  working copy held by a chain that is still running is taken back by the person who started it,
  with `withdraw`, which stops its session and parks whatever it left uncommitted; an agent
  escalates instead. See [limits and safety](nxc-limits-and-safety).

## Next

- [core-concepts](nxc-core-concepts) — what a thread, a channel and a handle actually are.
- [channels](nxc-channels) — the declaration these verbs address.
- [limits-and-safety](nxc-limits-and-safety) — the caps and gates around all of the above.
