# Limits and Safety

Every agent chat starts is a real process and a paid model call, and every one of them can start
more. This page is the set of brakes around that, and — just as importantly — the list of things an
agent is **not** allowed to decide for itself.

Read it as a design statement rather than a configuration reference. Almost nothing here is tunable,
on purpose.

## What an agent does not decide

The surface was cut deliberately so that a running agent cannot answer questions nobody asked it.
These are decided by a **declaration** you wrote, and there is no per-call override:

| Question | Where it is answered |
| --- | --- |
| Which model, and how much thinking is this worth? | the persona's `stage:` or `model:` |
| How long may a round wait? | the channel's `timeout:` |
| Who is in this round, and in what order? | the channel's `steps:` (or its `members:` and `flow:`) |
| Who may address whom? | the persona's `addressable:` |
| Does this need the working copy to itself? | `working_tree:` on the persona or the channel |
| What happens to the answers? | the channel's `on_complete:` and `visibility:` |

`send --model`, `send --deadline`, `send --kind`, `send --priority`, `send --disposition` and `reply
--kind` all existed and were all removed. The stated reason is worth repeating because it is the
rule behind the whole table: *otherwise an agent might over-eagerly change options.* A per-call
override standing beside a declared value is two answers to one question, and the caller answering
is exactly the party with the least standing to.

What an agent *does* decide is what it says, whom it says it to among what is declared, whether it
can carry the task out at all — and, where a channel's step declares somewhere for the answer to go,
whether the work it was asked to assess is deliverable (`--needs-rework`).

## The depth guard

A spawn chain is capped at **32 hops**. Past that, a trigger is rejected before any work is done —
which is what stops two agents that only ever push work at each other from looping forever, each
iteration costing a process and a model call.

Three details make it a real brake rather than a formality:

**It counts the OPEN chain**, not every hop a session ever took part in. Work that was commissioned
and *answered* unwinds: the target re-enters at the depth it already had, rather than one below the
session that answered it. This is not a detail — a raw counter killed the very shape it was meant to
protect, because an orchestrator and a channel owner talking over many rounds gained four hops a
round and went permanently uncallable after seven. Commissioning goes down; answering comes back up.

**It is anchored in the store, not in the environment.** The depth of a session is written by
whoever spawned it, before it ever ran. `NXC_HOP` in a process's own environment is a *claim* that
may raise the depth and never lower it. That distinction exists because every persona in the shipped
examples declares `tools: [Bash]`, so `unset NXC_HOP` before its own `nxc send` would otherwise have
reset the chain to zero on every hop and defeated the guard outright.

**What it does not close, stated plainly.** A caller presenting no session at all resolves to its
claimed depth, because a genuine human at a terminal is exactly that shape. So an agent determined
to escape can discard its whole ambient identity and look like a fresh chain — at the price of
everything that identity buys it: no return address on its posts, no resume, no ambient session for
anything to be attributed to. Closing that last gap needs an unforgeable credential handed to the
spawned process rather than an environment variable, which is an access-control design, not this
guard.

## The human gate

**No human stands inside the flow.** A person opens an operation and a person reads its result;
nothing in between waits for one. That is why `awaiting_human` on `nxc status` is true only at the
**root** of a finished operation, never at a thread in the middle. A finished operation is not in
the default listing at all — `--all` is where it is:

```console
$ NXC_ACTOR=alice nxc status --all
operation m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1 thread(s), 0 open  · finished
  m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  awaiting you (answered by ab12/coder)

```

The flag is a fact about one end of the chain, beside the thread's state rather than instead of it,
and it exists for one reason: an operation that has **stalled** and one that has **finished and is
waiting for you** look identical to a counter, and they call for opposite responses. If you build a
dashboard over `nxc status --json`, that is the field to branch on.

The other human gate is the one you write yourself. A persona that must not act without approval is
a persona whose `system_prompt` says so and whose declaration gives it no tools to act with —
`tools: []` is a real, distinct state, not the same as leaving the key out.

## Escalation: the declared "I need help, or a decision"

An agent has exactly two things it may say with `reply`: *I am finished*, and *I cannot reach the
result on my own*. The second is `--escalate`, and it is a **declared** signal rather than a
phrase:

```bash
nxc reply --thread <id> --escalate "The migration needs a production credential I do not have."
```

That matters because the supervisor branches on a bit whose meaning is written down, not on a token
an agent invented and no reader can enumerate. The turn is discharged either way — the session ends,
and the debt with it. What differs is the outcome, and the channel decides what becomes of it: an
escalating round is passed through rather than folded, so the hand-back reaches the requester as it
stands and never as a paragraph *about* a failure.

**"I cannot" is only half of it, and the other half is the one that gets missed.** A session that
can carry its task out perfectly well, but has hit a question that is not its to settle, belongs
here too — that is what "need a decision" means. The direction is the reason: an escalation goes
UP, to whoever commissioned you. The same question in a plain reply goes DOWN to the next step
instead, which has no more standing to settle it than you had, and the work then runs on around an
open question. Measured on 2026-09-12: a PM ended its turn with a plain reply and two open product
questions in the text; three review rounds judged a draft whose central decision was still open,
and the questions reached the owner only when the pass limit ran out.

It is not a wastebasket for anything that is not a result. It is also, today, the only way to hand
something back at all: `reply --kind question` was removed with the rest of `--kind`, so a real
follow-up question is either said with `--escalate` or asked by opening a conversation of its own.
That is a recorded loss, not a collapse.

**Waiting is not an escalation, and this is what keeps the signal worth reading.** A session that
commissioned a round of its own and is waiting for it ends its turn and says nothing at all — see
[channels](nxc-channels). An escalation in an operation therefore means one thing: a chain has
STOPPED and somebody has to decide what happens to it. Spend the verb on the ordinary case twice and
the alarm stops meaning anything; a plain `nxc reply` while your own round is open is refused for the
same reason, because it would claim a result you do not have yet.

## The working-copy lease

A repository has one working copy and one build directory. Two chains branching, committing and
running `cargo test` in it at the same time is not a race you can win by being careful, so chat
takes a lease.

Declare the need (`working_tree: exclusive` on a persona or a channel) and a trigger that would
collide **queues** instead of colliding. The mechanics, in the order they bite:

- **What a claim covers is the OPERATION**: the root of the thread tree the work belongs to, plus
  everything hanging beneath it at any depth — the same unit `nxc status` groups by. The claim still
  *arises* only at the first exclusive step, so a chain that never reaches one holds nothing; but
  once it is taken, it is given back when the **operation** is finished rather than when the round
  that took it is. That is what removes the gap between two rounds of one epic: a planner that
  commissions coding twice keeps the copy across the pause in between, instead of watching a foreign
  chain step into it.

  This page used to say the opposite — the claim stopped short of your conversation with the agent,
  so that "one unanswered human would hold the machine's working copy until tomorrow" could not
  happen. That boundary cost more than it bought, and the risk it named is now answered by the bound
  below rather than by leaving the top of the operation unprotected.
- **Acquire or queue.** A trigger inside a held area **inherits** the lease rather than queuing —
  otherwise a nested exclusive step would queue behind its own parent, which could never release,
  because what it was waiting for is the trigger it had just parked. The queue is first-in
  first-out, and a release promotes the whole claim area at the head, never one entry: several
  members of one fan-out share a claim, and firing one while its siblings stay parked deadlocks the
  board.
- **What releases it.** A reply inside the area releases the lease when nothing in that area still
  owes an answer *and* the last word was not a hand-back. An **escalation holds the lease** — the
  question is still travelling upward, the task is still mid-flight, and releasing the copy to a
  rival at that moment is exactly wrong.
- **How the hold clears.** Answering a hand-back **inside a channel** re-declares the round, and the
  hold is gone. This page used to say that without the qualifier, and the qualifier is the whole
  point: on a **direct** thread it is not true. A reply there counts as a hand-back's answer only
  when it comes from a handle the thread expects, and on an escalated thread the expected handle is
  the one that escalated — so an answer from anyone else, the opener included, posts and moves
  nothing. Measured, twice: `posted: true`, `woke: null`, and the lease still `holding` afterwards.
- **An unanswered escalation does not hold the queue for ever.** If — and only if — **another
  operation is waiting**, a 30-minute clock starts. When it runs out, that operation's work is
  committed to a branch (untracked files included; anything your `.gitignore` excludes is left in the
  tree), the checkout goes back to the branch that operation started on, and the waiting operation
  starts. When the answer finally arrives, the operation is put back on its own branch and told, in a
  fixed text, which branch and which commit its work is on — and whether the branch **or its base**
  has moved on since, which is the case you would otherwise never notice.

  Two things this deliberately does not do. **The clock starts at contention, not at the
  escalation**: with nobody waiting there is no cost to anybody, so nothing is measured and nothing is
  ever parked — this is not the ordinary way an escalation ends. And **a tree mid-rebase, mid-merge,
  mid-cherry-pick or mid-bisect is never parked**: committing a conflicted index would record a
  half-finished merge as if it were the work.

  **When a park is refused, one rule decides what happens to the checkout.** A refusal that can
  pass — a sequence in progress, or a git command that failed — **holds** it: the tick's `warnings`
  say `work_not_parked` and name what to finish, `nxc status` marks the operation `PARK REFUSED`
  with the same words, and the background service tries again on every tick, so the copy moves on
  by itself once the tree is fixed. A refusal that cannot pass — the runtime names no working copy,
  the directory is not a git repository, or no base branch was ever recorded for the operation —
  **hands the checkout on without parking**, as `work_handed_on_unparked`: whatever that operation
  left uncommitted is still in the tree, now in the next holder's hands. Holding the queue for an
  answer that no amount of waiting changes would be holding it for ever.

  The mark is there for as long as somebody is retrying, and no longer: if the reason the copy was
  wanted passes first — the escalation is answered and the operation works on, the operation on hold
  is taken up, nobody is waiting any more — the `PARK REFUSED` line goes on the next tick, even
  though the tree may still be mid-merge. Nothing is attempting that park, so nothing says it is.
- **A copy held by a chain you started and want back while it is still running: `nxc withdraw
  --thread <id>`.** Name the operation's first thread — the one your `send` handed back; `nxc
  status` lists the operation that holds the copy under that id, marked `holds working tree` — and
  it discharges the round, asks its session to stop, interrupts the chain below it, and once the
  process is gone parks whatever it left uncommitted onto a branch and hands the copy on; the next
  commission into the same thread brings the work back. Only the person who sent that first
  commission may do this: a session this workspace started is refused, and an agent that wants a
  round stopped escalates instead. A copy held by a chain that has simply died needs nobody to ask:
  the sweep below takes it on its own, past the bound or — since it can prove a chain is dead —
  inside it.
- **How long a lease lasts: as long as the operation itself declared.** The bound is the latest
  `timeout:` still outstanding in the claim area — a round that says it has six hours holds the copy
  for six hours, and an operation whose steps *all* say minutes is reclaimable in minutes after a
  crash. The "all" is load-bearing: **one** outstanding obligation with no window — and a conversation
  you opened with a plain `nxc send` is one — puts the whole operation back on a flat **two hours**,
  which is what every lease used to get regardless. So the two-hour number is now the answer for
  declarations that say nothing, not a budget imposed on ones that do — and a shorter bound is
  something you opt into by declaring `timeout:` along the chain.
- **The backstop.** An abandoned lease stops refusing *future* acquisitions once its bound has
  passed, and a `tick` past that point hands the queue's turn out — a commission that parks schedules
  one for exactly that instant, so the drain happens with nobody around. (This page used to say the
  queue never drains at all: nothing polled it, so a trigger already parked waited past the bound
  indefinitely. Something polls now.)

  **And that drain parks the dead operation's work first**, the same way an unanswered escalation's
  is parked: its uncommitted files go onto a branch, the checkout goes back to the branch it started
  on, and only then does the waiting commission start. A chain that crashed never escalated and never
  announced a boundary, so this is the only park it can ever get — it used to get none, and the next
  agent walked into its half-finished work. The refusal rule above applies here too: a tree
  mid-merge **keeps** the checkout and is marked `PARK REFUSED` until you finish or abort it — **or
  until you withdraw the commission that is waiting for the copy** (`nxc withdraw --thread <id>`),
  which ends the occasion rather than fixing it, and is the only way out you have when the refusal
  is a git command that failed rather than a merge you can see and finish.
- **A dead operation does not get to keep the copy until its bound runs out.** While anybody is
  queued behind a claim, a tick looks once a minute — not once at the bound — and asks whether the
  holder is *provably* gone. If it is, the copy is taken there and then, with its work parked first,
  exactly as the drain above does it. Typically that is within a minute of the death instead of up to
  two hours later.

  "Provably" is deliberately strict, because taking a checkout away writes into it. All of these have
  to be true: this workspace's chat worker can actually answer whether a session's process is alive
  (if it cannot, nothing is ever taken early — `nxc status` then ends with the note *this workspace's
  runtime cannot say whether a session's process is alive*, and `nxc status --json` carries the
  answer either way as `worker_answers_liveness`); at least one thread of the operation still owes an
  answer; nothing in the operation has a live process; no thread that owes an answer had its session
  report an end (a reported end is an orderly shutdown, not a crash); and nothing in the operation is
  on hold at an availability boundary, which has its own way back. An operation whose sessions all
  ended and handed the task back is **not** covered by this — it is waiting for you, and it keeps the
  thirty-minute rule above.
- **Whoever is already waiting goes first.** Reclaiming an expired lease hands the queue's turn out
  in the same step, so the chain that has been standing in line starts and the newcomer that just
  arrived takes its place at the back. This page used to be silent about it because the behaviour was
  the other way round: the next chain to *ask* got the copy, however long a rival had been waiting
  for it.

  **That hand-off parks the dead chain's work first too**, under the same rule as the drain above:
  it is the same occasion, reached by the other door, and a newcomer's own `send` is often what gets
  there before any tick does. So if that park is refused for a reason that can pass, **nobody** gets
  the copy: the newcomer simply queues behind the holder, exactly as it would have while the lease
  was still live, and the background service's retry is what eventually moves it.

  **It parks even when nobody else is waiting** — the plainest case of all: you run one agent, it
  crashes with work in the tree, and hours later your next `nxc send` is what takes the copy over.
  There is no turn to hand out, but the copy is still leaving an operation that came to grief, so its
  work goes onto a branch and the new session starts in a clean tree. The background sweep does not
  do this one: with nobody queued it hands nothing on, so there is nothing there to save.
- **A bound running out is not enough to take the copy.** A chain whose lease has expired but whose
  **process is still alive** keeps it — the same refusal the provably-dead check above makes for a
  holder still inside its bound, asked here on the other side of it.
  This page used to warn that a single step working for more than two hours loses the lease under
  itself and the next chain walks into the checkout it is still building in. It does not any more:
  what a rival meets is a live process, not a deadline.

Only a commissioning trigger is ever parked. A trigger carrying a *result* back up is never queued —
otherwise the answer whose arrival releases the lease would itself be withheld.

## What a handover records about the working copy

Every `nxc send` and every `nxc reply` writes down **where the working copy stood at that moment** —
the commit `HEAD` pointed at, the branch it was on, whether anything was uncommitted, and a
fingerprint that makes the whole state comparable later. It is on the message itself, so it comes
back wherever the message does.

`nxc threads show` prints it under the message it is a fact about — `working copy: 9f2c1ab4e7d0 on
feat/parser · uncommitted work NOT saved anywhere` — and `nxc status` prints the same facts as a
suffix on the thread's row: `· tree 9f2c1ab4e7d0 on feat/parser, uncommitted work NOT saved`. In
`--json` they are `refs.working_copy` on a message and `working_copy` on a status row, with
`commit`, `branch`, `dirty` and `fingerprint` as fields — branch through to the fields, never through
the prose.

What it is for, in the order it pays off: continuing an operation that was interrupted at a point
nobody recorded; reconstructing afterwards **which state of the tree a reviewer actually had in front
of it**; and naming the commit to rewind to.

**Three honest limits, and they are limits of git rather than of this.**

- **A dirty tree is recorded, not saved.** An agent hands over mid-work — that is the normal case —
  and then the commit names where the work *started from*, not what the tree looks like. The record
  says so in those words (`uncommitted work NOT saved anywhere`), because the opposite assumption is
  the one a reader makes. Nothing about a handover commits anything: your branch, your `HEAD` and
  your unstaged changes are exactly as you left them afterwards.
- **What git does not see, this does not see** — ignored files above all. A `.env`, a local database,
  `node_modules`: none of them is in the fingerprint, so a change confined to them reads as no
  change.
- **The repository rewinds; the record does not.** An anchor names a commit for the *working copy*.
  It does not roll back the board (`nxf` items you created or closed stay), the channel (messages are
  deliberately append-only), or anything that left the machine — a pull request, a release, an email.
  `.nxs/` itself is explicitly outside it: rolling the record back with the code would destroy the
  evidence of what happened along with the work.

A workspace whose runtime does not run its sessions in a directory this process can see records none
of this. `nxc status` says so once, in a closing `note:` line, rather than leaving you with an empty
column; `--json` answers it as `worker_names_a_working_copy: false` on the report.

## Content you did not write

Two places put text somebody else wrote into a model's context, and both are worth knowing about
before you decide what your agents may do.

**A channel's pass-through** renders every member's raw reply into the message the requester's
session receives. The shape says so: the replies are wrapped in an explicitly untrusted element, and
the delivery names the boundary it wrapped them with — one picked per round so that no answer
contains it, which is what makes the `from="…"` attribution the engine's own record rather than
something an answer could forge. That record says which handle a message was posted **under** — it
is not proof that the poster was entitled to that handle, and it is no evidence at all that what
they wrote is true. Treat a
member's words as data, never as instructions — and if a round's members are not all yours, say so
in the requesting persona's own prompt, because that is the layer that reads first.

**A transcript** contains raw tool inputs and results — whatever the agent read, wrote, or ran. So:

- Treat a dumped transcript like the workspace database, not like a message log. Do not paste one
  into a bug report unread.
- `nxc transcript show` is deliberately **not** membership-gated. A transcript has no channel to
  gate on, and the table is device-local and never synced, so a gate would buy nothing that anyone
  holding the file cannot already do with `sqlite3`. It is an operator surface; the file permissions
  are the boundary.

## What syncing means for privacy

In this milestone sync is **full-log and unfiltered**: a workspace bound to a stream exchanges the
whole log, with no per-channel authorisation on the wire. Anyone who can reach the stream can read
the messages in it.

So: do not put credentials or secrets in message bodies, and treat channel membership as a
convention about *who is asked*, not as a boundary about *who can read*. Transcripts are the one
thing that never leaves the machine.

### What to do instead

A secret belongs in the platform's own secret store, and the message says at most where it is.

- **At run time the persona fetches it itself** — from the host's secret mechanism (an AWS SSM
  parameter, a `secret: true` app setting, the OS keychain, a file the process can read). `nxc`
  needs to know nothing about it, and that is the right division: the message log is a record of
  what was said, and a credential was never something anybody said.
- **In the message, name the LOCATION, never the value** — the parameter name, the setting key, the
  path. `the DIP key is at .nxf/secrets/dip-api-key` is a fine thing to sync; the key is not.
- **If the location is a file, keep it out of the log and out of git** — mode `600`, and covered by
  `.gitignore`.

There is deliberately no secret-carrying channel and no encrypted message kind. Adding one would put
credentials into the very log this section is about, and every consumer of that log would inherit
them.

## Warnings that are warnings

Two things in `nxc` deliberately do not refuse, and it is worth knowing which:

- **A `send` that names no subject** still posts. The message is worth more posted than lost. But
  the receipt carries `refs_warning` as a field — not only a line on stderr — so an app sees the
  reminder too and a caller can branch on the value rather than parse prose.
- **A trigger that failed after the message was already written** does not discard the receipt. The
  `warnings` array on `send`, `reply` and `tick` is always present, even empty, and each entry
  carries a machine-readable class:
  - `step_skipped` — a step whose *triggering* failed. The message is durably in the channel and
    the obligation is registered, but nobody is working on it. **Nothing retries it.**
  - `step_unanswered` — a step that was started and never answered; its window ran out and what
    followed happened without it.
  - `requester_not_woken` — a board completed, the answer is sitting there, and the session that
    asked was never told.
  - `tick_unscheduled` — nothing is watching a declared `timeout:` right now, so that window has no
    clock: nothing will make the board `stale` on its own, and a member that goes silent holds the
    round indefinitely. The call itself succeeded — the board is open and its members are running —
    which is why this one class does **not** make the verb exit non-zero.
    `nxc tick --thread <id>` is the same check, callable by hand. The deadline itself is recorded
    either way, so a service that starts later still honours it. The message says which of the two
    halves is missing: this workspace is not registered with the nexus-flow background service, or
    no service is running. See [channels](nxc-channels) for how to set it up.
  - `service_not_running` — the same fact one level up, and the one that does not need a window:
    **there is nobody running any of it.** One background service keeps every deadline on this
    machine *and* is the only thing that syncs, so while it is down, declared windows do not fire —
    a round that should release hangs indefinitely — and this machine's op log neither pushes nor
    pulls. Both halves are named in the message, along with what to run. It rides every `send`,
    `reply` and `tick` while that is true, and like `tick_unscheduled` it does **not** change the
    exit code: your call did everything it was asked; the machine did not. You only ever see it in a
    workspace that has been registered with the service — one that never was is behaving exactly as
    arranged, and is never warned about a service it did not ask for.

  - `declaration_changed` — this persona was last started under a **different version of its own
    declaration**. Nothing failed: the session is running, and what you are being told is that the
    rules it runs under are not the rules the previous run used. `.nxs-personas/` lives in the same
    working copy the agents themselves work in, so one persona's `git switch` or `git stash` changes
    how the next one thinks — and that is how three declarations were silently rolled back here
    once. The entry names the role and both versions (`declaration.hash`,
    `declaration.previous_hash`), and every session's `spec.json` records the version it ran under,
    so you can check rather than guess. It does **not** change the exit code either: editing a
    persona and then addressing it is the ordinary way to work. A change is usually intended; an
    unnoticed one never is. See [personas](nxc-personas) for why the folder belongs in version
    control.

Those six call for different responses — plumbing to fix, a member to chase, a session to re-prime,
a background service to start, a declaration to look at — which is why they are distinct classes
rather than one bucket of prose. If you build anything automated on chat, this array is the thing to
watch.

Beside the class, each entry may carry a machine-readable **`reason`** for why a session was not put
in motion. One of them is new and worth knowing: **`already_running`** — the wake was refused because
the session it named still has a live process. One internal session runs one process, always; a
second start would put two agents in one working directory, overwriting each other. The message is
durably in the thread either way, and the running session is left to finish its turn.

One more field belongs beside them, on `nxc status` rather than on a receipt: **`escalated`**. A
thread whose current turn was handed back — a member that answered `nxc reply --escalate` "I need
help, or a decision", or a persona session that *died* and whose teardown settled the debt — is
`state: "answered"` like any discharged thread, because the turn really is over. `escalated: true`
is what tells the two apart. Branch on it wherever you would otherwise treat "answered" as "done".

And one beside *that*: **`substituted`** — the runtime wrote this thread's newest answer, standing in
for the agent that owed it. When a spawned session ends still owing its thread a reply, the sidecar's
teardown posts one in its name so the thread never goes quiet. That is the right behaviour, and it
used to be distinguishable only by *reading* the message body, which begins `sidecar:`. The two
fields are orthogonal and you want both:

| | `escalated` | `substituted` |
|---|---|---|
| the agent asked for help or a decision | `true` | `false` |
| the session died and the teardown said so | `true` | `true` |
| the session ended quietly without answering | `false` | `true` |
| the agent answered | `false` | `false` |

The third row is the one to watch. Nothing went wrong — so marking it as an escalation would wrongly
tell a supervisor to stop — and yet nobody answered. The commonest cause is a persona that was
*ordered* to reply and could not: check its declared `tools:`.

There is a fourth case and it is deliberately in none of these rows: a session that ended its turn
**waiting on a round it commissioned itself** produces no message at all. Its thread stays open and
still owed, because the session that owes it is coming back for it; `waiting_on_sub_round` on that
thread's `nxc status` row is what says so.

## Next

- [channels](nxc-channels) — where the timeouts and the exclusivity are declared.
- [personas](nxc-personas) — where the model and the tools are.
- [commands](nxc-commands) — the surface all of this constrains.
