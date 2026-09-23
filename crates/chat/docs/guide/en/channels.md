# Channels

A **channel** is a declared group and, since the order of its members can be made binding, it is
also the only workflow this system has. Both live in one file, `.nxs-personas/channels.yaml`, as a
YAML list:

```yaml
- name: review
  members: [general, integrity]
  description: the review quorum — one round asks both reviewers and hands back one verdict
```

`name` and `members` are the only required keys. Address it exactly as you address a persona:

```bash
nxc send --to review --ref nxf_ids=ab12.0008 "Judge the export change."
```

**There is no `nxc channels create`, and there never will be.** A channel that a verb minted lived
in one machine's database, had no reviewable membership, and did not survive the run. A channel that
is declared is a file in your repository — which is also why `members:` is the membership: there is
nothing to join and nothing to leave.

## What a send to a channel actually does

One call opens two levels, and knowing which is which makes every later reading of `nxc status`
obvious:

1. **The channel thread** — your conversation with the channel. It has exactly two ends: you, and
   the channel's **supervisor**. The supervisor is a reserved engine identity, `<origin>/__channel__`
   — it has no session, no transcript, and nothing ever resumes it. It is machinery, not a
   participant, and no declaration can claim its name because a handle may not begin with `__`.
2. **One slot thread per step** — the supervisor's conversation with each member, hung under the
   channel thread. That is the thread a member answers into, and its id is in the member's own
   trigger message.

```console
$ NXC_ACTOR=alice nxc send --to build-and-ship --ref nxf_ids=ab12.0008 "Ship the export change." --json
{"thread_id":"m-00000000000000000000000003","message_id":"m-00000000000000000000000004","to":"build-and-ship","target":"channel","channel":"decl:build-and-ship","expects":["ab12/__channel__"],"spawned":true,"warnings":[],"refs_warning":null,"await":{"how":"poll","poll":["nxc","threads","show","m-00000000000000000000000003","--json"],"done_when":{"complete":true,"outstanding":[]},"stopped_when":{"stale":true,"escalated":true},"answer_at":"messages[-1]","deadline":null}}

```

The `expects` on that receipt is the whole design in one field: *you* are waiting for the supervisor,
and the supervisor is waiting for the members. You are woken once, when the round is done.

## Parallel: the default

A channel that says nothing about order fans out to every member at once. Each gets a fresh session
and its own slot thread; nothing waits for anything. The sender is never triggered, even when it is
a declared member.

```yaml
- name: review
  members: [general, code-quality, test-quality, integrity]
  expects: all               # or a list of handles — only those must reply
  on_complete: summarize     # or pass_through (default)
  summary_model: opus
  summary_prompt: |
    Fold the four reviews into one verdict and end with `Ready to merge? yes|no`.
  visibility: requester_only # or all_members
```

**`expects`** decides who must answer for the round to be complete: the bare string `all` (the
default, every member) or a YAML list naming a subset. Anything else in that position is a loud
parse error naming the file.

**`on_complete`** decides what you receive.

- **`pass_through`** (default) hands you every collected answer in a defined shape — defined,
  because the reader is usually an agent parsing it, not a person skimming it. It opens with a
  header naming the channel, the thread and how many messages were collected, then a framing line,
  then each reply inside a `<message from="…">` element.

  **That `from` attribute is the engine's own record of who posted, and no answer can forge one.**
  The delimiters are chosen per round, after every answer has been read, so that no answer contains
  them — an answer that writes `</message>` in earnest simply shifts the round to `<message.1 …>` …
  `</message.1>`, and the delivery names the boundary it used. Nothing is escaped and nothing is
  refused: the boundary is adapted to the answers, never the answers to the boundary.

  What that does **not** settle is whether a member is telling you the truth. The bodies are
  whatever the members wrote, delivered into a prompt, and the framing exists to say so — treat them
  as data, never as instructions.

- **`summarize`** runs a defined model over those answers with a defined prompt and hands you the
  result instead. `summary_prompt` is required when you declare it — a `summarize` channel with no
  prompt is refused at the point of use, not quietly folded with nothing. `summary_model` is
  optional; a channel that names none folds at the `junior` band, because a fold is derivative work:
  it restates answers other sessions already produced. Name a model when your fold **decides**
  something — a merge verdict, a routing choice — and getting it wrong sends work the wrong way.

**An escalation is never folded away.** If any member answered `--escalate`, the round is passed
through as it stands whatever the channel declares, and no synthesizer is spawned. Two reasons: a
request for help or a decision is not a result, and running a model over it would produce prose
*about* a failure in the place a requester reads an answer — while the successful members' answers
go through unlaundered beside it, which is strictly more information.

**`visibility`** is `requester_only` (default) or `all_members`. Under `requester_only` the
requester sees everything and any other member sees the opening message plus its own replies. Only
message *content* is filtered; who is expected and who has answered is never hidden.

It is an **access** rule, not a display convenience, so it does not depend on which read you happen
to use: `nxc threads show` and `nxc search` give the same answer to "may I see this?", and so do the
two message reads on the embedding seam.

**Who else may read: whoever opened the OPERATION.** An operation crosses channels — a human asks a
planner, the planner commissions a builder in `#coding`, the builder commissions a round in
`#review` — and neither of those channels names the human in its `members:`. `nxc status` has always
shown that whole tree from its root down, across every channel border; since then the messages
follow. Whoever opened the operation reads any thread in it, whatever channel it sits in.

It does not widen `requester_only` for anyone else: that rule is between the participants of *one
round*, and a member still never sees its neighbour's answer. The operation's opener is not in the
round — it is the party the whole chain answers to, and every consolidation is delivered towards it
anyway.

And `members:` is genuinely the membership, in both directions: an edit to the file reaches the door
on the next *read*, not on the next `send`. Add a handle and it can read at once; remove one and it
cannot, without anything having to be re-sent in between. That holds for **every** read of the
channel's messages, `nxc search` included — one list, one answer, whichever verb asks.

**Nothing on the CHANNEL declares when a member is resumed**, and nothing needs to: a fan-out
always starts a member's turn on a fresh session, and a `nxc reply --thread` into that member's own
thread resumes the session it already has. A `member_session: fresh | resume` key stood here until
it was removed — it steered nothing and refused its own second value — and a `channels.yaml` that
still sets it loads unchanged, the key ignored.

The one place a declaration *does* decide is a **step**, which has a `resume:` of its own — see
[Continuing a session, or starting a fresh one](#continuing-a-session-or-starting-a-fresh-one). Per
step rather than per channel, for the reason the removed key failed: what a round should carry
across depends on which edge it arrived over, and one answer for a whole channel cannot say that.

## Sequential: the order is the flow

```yaml
- name: build-and-ship
  members: [coder, review]
  flow: sequential
  description: the declared order a work order runs through
  timeout: 2h
```

One field turns a group into a workflow. `flow: sequential` says the order of `members:` **binds**:
one target at a time, each started only once the one before it has settled.

**`flow: sequential` has no step list of its own, deliberately.** A second field naming its own
sequence of targets would be a second membership list beside `members`, with its own referential
rules and its own way of disagreeing with the first. `members` is already an ordered list; what was
missing was not an order but a declaration that the order binds. So that is all this field says, and
everything else about such a flow is read off `members`.

That also fixes its ceiling: a flat list is a straight line. When you need a **cycle** — work, check,
back to work — declare `steps:` instead, which is the next section.

A step's target is addressed exactly as `send --to` addresses one: a declared persona handle, or a
declared **channel** name. That is how a review round becomes a *step* rather than something
somebody opens by hand — and what a channel step contributes is its own consolidated answer.

**How a step moves on.** The member answers its slot thread with `nxc reply --thread <id>`. That
reply is the whole mechanism: the supervisor runs inside that same write path, sees the set is
settled, and opens the next step. There is no "step done" verb, no branching token, and nothing to
poll. If a member ends its session without replying, the operation stands still until the declared
`timeout` strikes.

**"Settled" means `complete || stale`** — a step that answered and a step whose window ran out with
nothing in it both release the next one. A step that stayed silent is *reported*, never absorbed:
the call that moved the flow past it carries a `step_unanswered` warning naming the silent thread,
and so does the consolidation that carries no answer from it.

**A step that escalated releases nothing — it ends the chain.** `nxc reply --escalate` says "I
cannot reach the result on my own — I need help, or a decision", and a task that did not reach its
result must not commission the work that was to follow it. So the successor is not started, the
round is handed up as it stands with the escalation on it, and whoever commissioned the channel
decides what happens next. On a `parallel`
channel nothing changes: everyone there was asked at once, so there is no successor to withhold.

**A round that was taken back ends the chain too, whatever its shape.** `nxc withdraw` belongs to
the person who sent the operation's first commission, names that thread, and takes back everything
below it: a queued step is removed, a running one is stopped and its work parked on a branch.
Nothing is commissioned after that — not the next step, and not the step that was taken back, on a
`flow: sequential` channel and on one that declares `steps:` alike — and nothing is consolidated:
the channel thread is discharged with a message saying the chain was interrupted. The work comes
back when that person commissions the channel thread again. See [commands](nxc-commands).

**A step that commissioned a round of its own says "not yet" by saying nothing.** `reply` has two
answers and both of them end the turn — so a step that consulted somebody and is waiting for the
answer simply ends its turn without replying at all. The engine can see what you commissioned (it
opened those threads for you), so it treats that as waiting rather than as a missing answer: no
reminder, nothing posted in your name, and the flow stands until the round comes back and wakes you.
`nxc status` says so on the row — `waiting on its own sub-round` — and the operation is not marked
`NEEDS DECISION`.

Two consequences worth knowing before you meet them. A plain `nxc reply` while your own round is
still open is **refused**: it means "I am finished", which is not true yet. And do not keep working
while you wait — the wake starts a second process under the same session in the same working copy,
and it overwrites what the first one was still editing.

`--escalate` stays available and still means what it says: "I cannot reach the result without help
or a decision." Reaching for it to hold the flow was the way to do this before, and it is now the
wrong one — it costs you the rest of the chain and tells your caller the work will not arrive.

A **stepped** channel does have a third answer, and it is the reason the next section exists.

**…and on `working_tree: exclusive`, the step's SESSION has to be over too.** A reply is a message,
and the process that wrote it can keep working: measured in a real run, a coder answered at 00:16:33
and went on editing until 00:34:27, while the next step had already started at 00:16:36 — into the
very checkout the channel declared it needs alone. So a channel that claims the working copy waits
for both facts. What supplies the second one is the session announcing its own end as it tears down
(`nxc session ended`, which the sidecar calls for you); if that never arrives, the runtime's own
process check answers the next time anything asks — and something asks on a clock, see below. A `shared` channel is untouched by any of this —
two live sessions there are not a defect.

One consequence worth knowing before you see it: on such a channel the next step starts a moment
after the answer rather than in the same instant, because it starts when the previous session
exits.

**A clock of its own watches this wait.** `timeout:` below cannot: it is a deadline for an
*answer*, and a member that has replied has answered. So a decline whose only remaining blocker is a
live session schedules a re-check of that same board **a minute later**, and keeps doing so for as
long as the process is still there. In practice the announcement arrives a second later and you
never see any of it. When it does not — a sidecar killed before its teardown, an `nxc` older than
this feature, a host runtime that never calls `Engine::session_ended` — the re-check finds the
process gone and the flow moves on by itself, about a minute late.

You can always ask by hand, and the answer is a diagnosis rather than a shrug:

```text
$ nxc tick --thread <the channel thread>
tick declined on thread …: the set is settled, but a session behind it is still running.
```

To see *which* session and what became of it, `nxc session state --thread <the step's thread>`.

A process that hangs rather than exiting is the one case the clock cannot resolve: it keeps looking
and keeps finding it alive, which is the same wait as before — but a visible one, with something
looking at it. If you are building a host runtime, wiring `Engine::session_ended` keeps you out of
all of this.

Two things are refused on a sequential channel, both at the point of use and both with nothing
started:

- **`expects: <subset>`** — the fan-out returns a subset verbatim, so it would be a second list
  quietly deciding the order. The refusal names both lists.
- **the same target twice** in `members`, and any set of channels that can reach itself through its
  members. A cyclic catalogue is refused outright when the declarations are loaded.

The second refusal is not tidiness: on such a channel the engine works out which step a thread serves
by matching its target against `members`, so a repeat would make two different steps
indistinguishable. `steps:` below lifts it by giving every step a name.

## Steps: when the flow has to come back

```yaml
- name: coding
  members: [coder, review, finisher]
  working_tree: exclusive
  steps:
    - id: build
      target: coder
      next: check
    - id: check
      target: review            # a persona, or a whole channel
      on_needs_rework: build    # the way BACK
      max_passes: 3             # attempts IN TOTAL — the original plus two reworks
      next: ship
    - id: ship
      target: finisher
```

`steps:` is a small state machine. `members:` keeps meaning what it always meant — who belongs to
this channel — and `steps:` says what happens in which order. **A channel that declares `steps:` may
not also declare `flow: sequential`**: that would be two lists deciding one thing.

- **`id`** names the step, and it is what makes a cycle possible. Two steps may address the same
  persona, and the same step may run twice in one round, because the engine keys a step's thread on
  its id rather than on its target's name.
- **`next`** is the normal way on. A step with no `next` is the end of the round: the channel
  consolidates there and answers whoever asked.
- **`on_needs_rework`** is the way back, and the only branch there is.
- **`max_passes`** counts **attempts in total, not reworks**: the original run is pass 1, so
  `max_passes: 3` allows two trips back. Anything below `2` is refused — it would declare a back
  edge that can never be taken, and a step that should never send work back simply declares no
  `on_needs_rework`. It sits on the back edge, never on the channel: a channel may have several
  cycles and one counter would add them together. It is required wherever the edge is — a back edge
  with no ceiling is a loop nothing ends.
- **`resume`** decides whether the step CONTINUES the session its target already ran in this round,
  or starts a fresh one. Leave it out and the *edge* decides, which is what you want almost always —
  see below.
- **`task`** says what the target is to do *here*, and becomes a fourth layer of its prompt, under
  the persona's own (`nxc guide personas`). See below.
- **`stage`** runs the target at another band than the one it declared — `junior | senior |
  principal`, the persona vocabulary (`nxc guide personas`). See below.
- **`input`** says what this step is given out of the round: the request, the answer of a named
  step, both, or nothing. Leave it out and the defaults apply, which is what you want almost always
  — see below.

### The same role, a different job

A persona says who it is: which dimension it speaks for, how it grades, when it sends work back. A
step says what it is looking at. Keeping the second out of the persona is what lets one declared
reviewer serve two stations:

```yaml
    - id: check
      target: review            # the same three reviewers as the coding chain uses
      stage: senior             # …but a design judgement is dearer thinking than a diff
      task: |
        You are judging a SPECIFICATION, not a diff. Read the document named in the task and ask
        what would happen if it were built as written.
```

Both keys are optional, and a step that declares neither behaves exactly as it did before they
existed.

- **`task` adds, it never replaces.** It arrives under the persona's own prompt, marked as the
  step's, and the engine puts one sentence of its own above it: where the two seem to disagree, the
  role wins — how it judges, how it answers and where its threshold lies are the role's, not the
  caller's. So a channel cannot take a reviewer's rubric away, and you do not have to write "and
  keep doing everything your role says" into every task.
- **`stage` moves the band and nothing else.** A step may **not** name a model — the key is refused
  by name when the team is loaded, and it tells you to write `stage:` instead. A persona that
  declared its own `model:` runs on that model whatever the step asks for; a persona that declared
  only a band, or none, runs at the step's.
- **A step whose target is a whole channel passes both to every member.** That is the only placement
  that can mean anything: a channel thread runs no session, its members do. It reaches the step's
  target and no further: somebody the step's own session goes on to commission with `nxc send` is
  doing that session's errand, not the step's, and is told nothing about it.

### Continuing a session, or starting a fresh one

A step reached over `on_needs_rework:` is the same party being handed its own work back. It is woken
with a notice that says *"what you handed over"* and *"your claim on the working copy is still
yours"* — sentences that are only true of the session that did hand it over. So **a back edge
continues that session**: the findings arrive as its next turn, with everything it built still in
its context, and it does not have to rediscover its own work from the checkout.

A step reached over `next:` is the next piece of work. That it happens to name the same persona is
not by itself a reason to carry a session across, so **a forward edge starts a fresh session**.

That is the default, and `resume:` on the step overrides it either way — the same two steps as
above, with both defaults turned round:

```yaml
    - id: build
      target: coder
      next: check
      resume: false             # a rework goes back to a CLEAN session every time
    - id: check
      target: review
      on_needs_rework: build
      max_passes: 3
      next: ship
      resume: true              # …while the reviewer keeps its own session across the passes
```

A reviewer that keeps its session is not a curiosity: on its second look it remembers what it asked
for, so it can judge whether it was done.

Either way the step gets a **fresh thread**, which is what the pass counter counts. Continuing is
about the session, not about the slot. A round that has never run this target has nothing to
continue and simply starts it — that is the first pass, not an error. And a step whose target is a
whole **channel** cannot be continued at all: a channel has no one session behind it, its members
each have their own, so `resume: true` there is reported as a bad declaration when the team is
loaded.

### The third answer

A step whose declaration names an `on_needs_rework:` edge gives the party serving it a third way to
end its turn:

```text
nxc reply --thread <id> -                   # done
nxc reply --thread <id> --needs-rework -    # somebody else's work goes back
nxc reply --thread <id> --escalate -        # I need help or a decision; the round goes up
```

(There is a fourth flag, `--accept`, and it is deliberately not in that list: it is not one of the
endings a step has. It belongs to whoever **commissioned** the round, on the round's own thread — see
[When the ceiling is reached](#when-the-ceiling-is-reached).)

The `-` reads the message from STDIN — `nxc reply --thread <id> - <<'EOF'`, your text, then `EOF` on
its own line — so that nothing in it is evaluated by the shell. That matters most on exactly these
three: a verdict names files, quotes commands and pastes their output, and in the argument form
`nxc reply --thread <id> "…"` the backticks and `$(…)` in it are rewritten before `nxc` sees them.
A one-line answer may still be an argument.

The three are a trichotomy over *done*, *again*, *not on my own*, and there is no fourth case.
"Good but incomplete" is rework. "I need X first" is escalation — and so is "this decision is not
mine to make", even when you could carry out either answer. "Fine" is the normal way on. Note which
work each is about: `--escalate` is a statement about **your own** work, `--needs-rework` about
**somebody else's**. Setting both is refused.

**A step that declares no back edge is never offered the bit at all** — the session is told about two
endings, not three. That is deliberate: if the flow shows no reaction to a verdict, then asking for
one was asking for an opinion, not for a decision. Set it there anyway (you found it in `--help`) and
it falls away, with a `verdict_dropped` warning on the reply's own receipt saying that this channel
declares no transition for it.

**If the step's target is a whole channel, every member is offered the third answer and any one of
them is enough.** The step names the channel, but nobody stands on the step itself — its members do,
one session each — so each of them reads the same three endings a role addressed directly would, and
the verdict of a single one routes the whole round back. A reviewer who found something is not
outvoted by three who did not, and the channel's own `on_complete: summarize` still folds the answers
into the one judgement it was declared to produce.

The nearest declaration wins, so that sentence has one exception worth knowing: if the channel a step
commissioned declares `steps:` of its own, whoever serves one of those steps reads THAT step's
endings — its own `on_needs_rework:`, or two endings where it declares none — and does not inherit
the outer step's.

### What comes back with the work

The party sent back is woken with the verdict itself — the findings *are* the task — under a notice
that says what happened, which attempt this is, and that its claim on the working copy is **still its
own**:

```text
NEEDS REWORK — this is NOT an approval. What you handed over was reviewed and did not meet the
standard of the party that reviewed it; their reasons are in the body below, and they are the work.
This is pass 2 of 3. Your claim on the working copy is STILL YOURS and is held across this round — do
not acquire it again. …
```

Declare `rework_notice:` on the channel to say it in your own words; `{pass}` and `{max}` are filled
in for you.

### What a step is given: `input`

**The defaults carry the ordinary channel and you will rarely write this key.** The first step of a
round is given the request that opened it. Every later step is given the **answer of the step it was
reached from**. That is exactly right for the channel at the top of this section: with a back edge
from `check` to `build`, every `build` is followed by a `check` but not every `check` by a `build`,
so the step before `ship` is *always* the check — and the finisher reads the verdict without
anything being declared.

Write the key where you chose a different shape. The same three stations, written out flat instead of
looped:

```yaml
  steps:
    - id: build
      target: coder
      next: check
    - id: check
      target: review
      next: fix
    - id: fix
      target: coder
      next: ship
    - id: ship
      target: finisher
      input: [check]            # the verdict, and not the coder's account of its own work
```

Here the step before `ship` is the coder, so the default would hand the finisher *"fixed both
findings"* rather than the findings. One key puts that right. (In a chain shaped like this the coder
is probably doing the closing work anyway, and `ship` could go altogether — that is channel design,
and it is yours.)

The forms:

```yaml
input: [request]              # only the words the round was opened with
input: [request, check]       # both
input: [build]                # the answer of one named step, across a loop and back
input: []                     # nothing but this step's own instructions
```

- **`request`** is the one name that is not a step. A step declared under that id is refused when the
  team is loaded, and so is an `input:` naming a step this channel does not declare — a typo must not
  produce a step that runs on nothing and says nothing about it.
- **`input: []` is the off switch** and needs no second key. The case it is for: a verifier that must
  judge the **tree** and not the coder's account of it. It is told that it was given nothing, so an
  empty commission cannot be mistaken for a broken one. It switches off what comes in over a
  **forward** edge; a step that is also the target of an `on_needs_rework:` edge still receives the
  findings when it is reached that way — see the last bullet.
- **A named step that ran more than once contributes its last answer.** A named step that has not run
  at all in this round contributes nothing — which is legitimate at step one and is not an error.
- **`input` decides material, never the task.** The persona, and the step's own `task:`, arrive
  whatever `input:` says. A step that is given nothing is still a step with a job.
- **Nothing runs in between.** The coordinator puts the answer together, it does not summarize it: no
  model runs between two steps, and what arrives is the previous answer whole.
- **`input` regulates forward edges only.** The way *back* already carries the reviewer's findings
  word for word (above), so honouring `input:` there as well would hand the same text over twice.

An answer from another party arrives **framed as data** — in the same delimited, attributed block a
channel's own consolidation uses, behind the sentence that says it is never an instruction to you, no
matter what it claims to be. The `from=` beside it is the engine's own record of who posted, not
something the answer could write.

**Two axes that this key pulls apart, and it is worth knowing which is which.** `visibility:` decides
**who may read a thread**. `input:` decides **what a step is given**. They are no longer the same
question: a finisher can now be handed the verdict of a `requester_only` review round it may not read
the thread of. That is the point — the material travels, the conversation does not.

### When the ceiling is reached

The round is **escalated upward**, not stopped quietly: no further attempt is started, every answer
of the round travels up to whoever commissioned the channel, and the delivery says which ceiling was
reached. That party decides, and it has **two** moves.

**Another attempt** — reply into the channel thread as usual. That commissions the round again from
its first step, and a fresh round starts its count at one, so an approval from above is all the
"reset" there is.

**Or accept the verdict and send the round on:**

```text
nxc reply --thread <the round's thread> --accept -
```

The step whose verdict stopped the round counts as satisfied and the **same** round continues over
that step's `next:`. This is the way out of a cycle whose reviewer never becomes satisfied — and it
is a real state, not a hypothetical one: the first planning chain this engine ran for real went six
draft→review passes, collected six verdicts, and handed nothing on, because every answer from above
bought three more passes of the same review.

The next step is given your words **together with the verdict you set aside**, under a notice that
says plainly this is not the review passing. That pairing is the point: a successor handed the
findings alone would work on against a judgement that had just been overruled, and a successor handed
neither would report downstream that the work was reviewed clean.

Two rules make it a decision rather than a shortcut:

- **Only the party that commissioned the round may give it.** On a slot you are *serving* it is
  refused by name and sent to `--escalate`: the party that produced the work may not declare its own
  reviewer satisfied, which is the separation the round exists to create. `--escalate` is how an
  agent asks for this decision; `--accept` is how the party above it answers.
- **Nothing about it reads the counter.** It is not a privilege the ceiling unlocks — a round that
  stopped short of its ceiling (a member escalated, say) can be accepted just the same. What it does
  need is a round that has *stopped*: while a step is still working, accepting is refused, because
  opening the successor beside it would put two sessions in one working copy.

The acceptance is recorded as its own kind, so an override is findable afterwards as an override —
`nxc threads show` renders it in the round's thread, and it is not "the review passed".

### What the round reports at the end

A round that went round a back edge hands over two answers from one step that contradict each other
on purpose — *"the lock order is wrong"* and *"good now"*. So the answers do not arrive as a flat
list: every one of them carries the step it served and which pass of that step it was, and the one
that carried the verdict is marked as such.

```text
<message from="local/coder" step="build" pass="1">
first cut is in: added the cache
</message>
<message from="local/review" step="check" pass="1" needs_rework="true">
the lock order is wrong in two places
</message>
<message from="local/coder" step="build" pass="2">
lock order fixed
</message>
```

Above the block, the delivery says the one thing nobody can work out from the messages themselves:
**where one step appears more than once, the later pass supersedes the earlier one** — the earlier
passes are how the round got here, not findings that still stand.

It also states, in the same place, how many times each step really ran — counted by the engine from
its own record. That is the one thing a reader cannot work out by reading: how many passes there
were to expect, which is what makes "the later pass supersedes" a rule about a known number of
things. The marks themselves are the engine's own, and no answer can write one — see `pass_through`
above for the boundary that guarantees it.

Both output forms carry it. `on_complete: pass_through` renders it for whoever asked; `on_complete:
summarize` hands the same structure to the session that writes the closing report, so what comes
back is *built* out of the round rather than passed along. Your `summary_prompt:` says what the
report is for; the run's shape is supplied for you.

A channel that declares no `steps:` has no passes, and its delivery is exactly what it always was.

## Timeouts

`timeout:` is how long a round may wait. It is declared here or nowhere — the per-call `--deadline`
override is gone, because a declared window and a caller's override are two answers to one question.

```yaml
timeout: 20m                      # a window: 20 minutes of silence
timeout: 2026-08-16T10:30:00Z     # an instant: this moment, whatever happens
```

The parser tries RFC3339 first, so both forms are expressible in one field — and the difference is
real. **A duration is a per-member, resettable window**: each member's clock is restarted by every
write to its own session transcript, so a member that is *working* is never struck for taking a
while. What it protects against is a member that went silent. **An absolute instant arms no clock**
and nothing moves it.

A value the grammar cannot read is a `validation` error naming the field and the value, refused
before anything is persisted — never a silent "then there is no cap". Units are `s`, `m`, `h`, `d`,
`w`.

When a window is declared, chat arms a deadline for the moment it first falls due. When that moment
arrives, a hidden verb runs — `nxc tick --thread <id>` — which re-checks the thread and, if it is
due, routes it through the channel's policy. Because the window moves as members show signs of life,
the chain re-arms itself: a tick that finds nothing due arms the next one. It is idempotent — a
thread already handled is a clean no-op, never a second wake.

Nobody types `tick`. It is named here so that finding it in a log or a process list is not a mystery.

### The deadline is kept by the nexus-flow background service

Arming a window writes one line into the workspace's own book, `.nxs/timers.json`. The nexus-flow
background service — one process, the same one that keeps your workspaces in sync — reads that
book on every pass and runs the check when the moment comes.

By default it is called `nexus-flow`, and on a Mac that is the name you will find in
**System Settings › General › Login Items**. Set it up once:

```
nxs sync daemon install          # macOS: a launchd agent that starts it at login
nxs sync daemon                  # anywhere: run it in the foreground under your own supervisor
```

Its log is `~/.nexusflow/logs/service.log`. `nexus-flow` is also a command — it *is*
`nxs sync daemon` — so `nexus-flow status` answers "is it running, and what did it last do".

#### It keeps the machine awake while a run is working

A run that works unattended for hours does not survive the Mac falling asleep. While any session is
alive in any workspace the service attends, it holds an idle-sleep assertion, and it gives it back
the moment the last one ends. You can see it, by name, in `pmset -g assertions` — it says which
instance is holding it.

It prevents *idle* sleep only: your display still sleeps, a closed lid still sleeps, and choosing
Sleep from the menu still works. And it is held by the service process itself, so if the service is
killed the assertion goes with it — a crash cannot leave your Mac permanently awake.

#### More than one service on one machine

If you develop against several checkouts at once, you do not want them sharing a clock: whoever
installed last would own it, for all of them. Give a checkout its own **instance name** and it gets
its own everything — its own launchd agent, its own `~/.nexusflow-<name>` directory, and therefore
its own registry, lock, heartbeat and log:

```
export NXS_SERVICE_INSTANCE=nexus-flow-dev    # in your .envrc, once per checkout
cargo build                                   # …and install it by running the BUILD:
./target/debug/nxs sync daemon install        # installs *that* instance, running *that* binary
./target/debug/nxs sync daemon status         # says which instance it is answering for
```

**The running binary decides, and the variable only names.** A binary out of `target/debug` or
`target/release` is a development instance, and `NXS_SERVICE_INSTANCE` says *which* one — a machine
has several checkouts. The installed `nxs`, `nxf`, `nxm` and `nxc` on your `PATH` are the machine's
own service, and standing in a checkout does not change that: the variable has no vote over them,
so your everyday commands keep talking to the service that keeps your machine's time.

That is why the install above runs the built binary rather than the installed one. An install points
the instance's alias at the binary that ran it, so installing from the build is what makes a
development service actually run your development build — and installing from the installed one
would just have re-installed production under a development name. `nxs sync daemon install` says so
when the variable named an instance it could not give a vote to.

A name is `nexus-flow` or `nexus-flow-<something>`. The instance's name is also the command:
`nexus-flow-dev status` is that service's own `status`, and it is how the running service knows
which one it is — the launchd agent runs a link with that name, and the name is the whole
instruction — the alias stands for `nxs sync daemon`, so the verb follows it directly. That link
is also how you reach an already-installed instance from anywhere:
`~/.nexusflow-dev/bin/nexus-flow-dev uninstall` removes *that* service, whichever binary happens to
be behind the link.

Two instances cannot get in each other's way: separate directories mean separate locks, so both run
side by side and installing one never removes the other. The one thing they *can* share is a
workspace — nothing stops you registering the same checkout with both — and then both attend it,
both read its deadline book, and a window that comes due can be started twice. Register a workspace
with exactly one instance; `nxs sync bind`, `nxs sync daemon status` and the running service all say
so if you have not.

That answer also says **which binary it is running**: the program the live service started from,
its version, and what the `nexus-flow` link points at *now*. The two can differ — `nxs self-update`
writes a new binary and deliberately does not move the link, so which build keeps your machine's
time stays a choice — and `status` says so when they have drifted apart. The link pointing at a file
that is *gone* is the one failure launchd cannot report at all (its only symptom is a service that
never starts), so that case is not left to `status`: every `nxs`, `nxf`, `nxm` and `nxc` invocation
says it, names the dead target, and gives you the one command that repairs it.

What this buys, compared to asking the operating system for a job per deadline (which is what
happened before):

- **It is honoured to the second.** Nothing rounds up to the next whole minute.
- **A board closed early leaves nothing behind.** The line is overwritten or removed; there is no
  agent left registered in your login session for a window that no longer exists.
- **The same mechanism everywhere.** macOS, Linux, and anything else the service runs on read the
  same book.

**This changed, and it is worth knowing if you worked around it.** `nxc` used to schedule through
`at` — and macOS ships `atrun` *disabled*, so `at` accepted the job, exited 0, printed a job id, and
nothing ever ran it. A stock Mac accumulated a queue of jobs whose due date was long past, and a
declared `timeout:` **never fired** there: a member that went silent held its round indefinitely,
because nothing ever made the board `stale`. If you enabled `atrun` by hand for that, you no longer
need to. `NXC_TIMER=at` still selects the `at` backend explicitly, on any platform, if you want it.

### And what it costs, said out loud

There is no clock without the service. `at` was self-carrying — submit the job and the operating
system owns it, whether anything of ours is running or not. This is not, and `nxc` does not pretend
otherwise: arming a window in a workspace no service attends, or while no service is running, still
**records** the deadline (a service that starts later honours it) and still reports it on the
receipt, as a `tick_unscheduled` entry in `warnings`, naming the board and what to do. It is
deliberately not a non-zero exit: the board is open and its members are running; what is missing is
the safety net.

The two things it can tell you apart, because they need different answers:

- *this workspace is not registered with the service* — register it (`nxs sync bind`, or from your
  app), then `nxs sync daemon install`;
- *no service is running* — start it.

When you do see it, `nxc tick --thread <id>` is the same verb the service would have run, idempotent
and callable at any time. Nothing is lost — the window is simply checked when somebody asks rather
than on its own.

If a round must not be able to hang and you cannot run the service, do not rely on `timeout:` for it
— treat the fuse as absent and check the board.

## Exclusive use of the working copy

```yaml
- name: review
  members: [general, code-quality, test-quality, integrity]
  flow: sequential
  working_tree: exclusive
```

Declared on the *channel* when it is the round that needs the repository, rather than any one member
on its own account — a review quorum where every member runs the project's own build and test is
exactly that shape. It also works the other way: if a declared member says `working_tree: exclusive`
for itself, the whole channel counts as needing it and this line is not repeated. That inheritance
is **one hop only** — a channel does not catch it from another channel through a shared member.

What it protects against, and what it does not: it protects this round from *other* chains, which
wait instead of running their own build against the same target directory. It does **not** separate
the members of one round from each other — they are one claim area, the first acquires and the rest
inherit, so they all run at once. The unit of exclusion is the area, not the session.

**That is why the example above also says `flow: sequential`, and it is not decoration.** Four
members that each run the project's build in the same checkout are four builds in one target
directory. Two of the same shape merely take the build tool's own lock and wait — correct, and
several times as slow. Two of *different* shapes overwrite each other's artifacts, and what you then
read is a broken build that is really a shot artifact. `working_tree:` cannot separate them; the
*order* can, because the next step of an ordered flow opens only once the previous one has answered
**and** its session is over. So: a quorum whose members only read may fan out and give you four
opinions at once; a quorum whose members BUILD wants both lines. The price of the second is
wall-clock, plus the ordered-flow rule that an escalating reply ends the chain — a member that cannot
run stops the round instead of leaving the others' verdicts behind, which is usually what you want
when they all share one checkout anyway.

**And the area is the OPERATION** — the whole thread tree the work belongs to, the same unit
`nxc status` groups by, not just the branch the exclusive step happened to start in. The claim still
*arises* at the first exclusive step, so a chain that never reaches one holds nothing; but once it is
taken it is given back when the operation is finished. That is what keeps a planner's second coding
round from having a gap in front of it. If you want the copy held from the operation's first move —
through the planning too — declare `working_tree: exclusive` on the channel that starts it.

Here is what that looks like when a round is parked behind a conversation that is already holding
the copy:

```console
$ NXC_ACTOR=alice nxc threads list
m-00000000000000000000000003  decl:build-and-ship  0/1 in  [waiting]  · waiting for working tree (#1)
m-00000000000000000000000004  decl:build-and-ship  0/1 in  [waiting]  · waiting for working tree (#1)
m-00000000000000000000000001  dm:9445dbc93dfdad401585c42b  1/1 in  [complete]
m-00000000000000000000000002  dm:9445dbc93dfdad401585c42b  0/1 in  [waiting]  · holding working tree

```

Note what the send receipt did *not* say: a channel send reports `spawned: true` even when every
member is parked, because it is one receipt for the whole fan-out. The board above is where a
requester finds out. Full mechanics in [limits-and-safety](nxc-limits-and-safety).

## Preconditions: what must hold before a step starts

```yaml
- name: coding
  members: [coder, finisher]
  flow: sequential
  working_tree: exclusive
  preconditions:
    - name: remote-not-ahead
      run: git rev-list --count HEAD..@{u}
      expect: "0"
    - name: tree-clean
      run: git status --porcelain
      expect: ""
```

A precondition is a hurdle the supervisor puts in front of **every step of this channel**. It runs
in the working copy the sessions run in, *before* the step's session is started — after the start
the session is there and whatever it was going to do is under way with it.

**A hurdle passes when its command exits zero**, and — if the declaration names an `expect:` — when
its trimmed standard output is exactly that. Omit `expect:` for a command that is already a
predicate (`test ! -f .nxs/build.lock`); write `expect: ""` for one that must print nothing. The
exit status is always checked, so a command that fails *and* happens to print the expected string is
still refused: `git rev-list --count HEAD..@{u}` with no upstream configured fails and prints
nothing, and reading that as "zero commits behind" is exactly the wrong answer.

**Everything else refuses too.** A command that does not exist, crashes, or does not finish within
its bound stops the step — and a hurdle that runs out of time is killed *with everything it started*
(it runs in its own process group), so nothing of it is left behind in the working copy the next step
wants. A hurdle that lets a doubt through is not a hurdle — and a runtime that
cannot run commands at all refuses every declared hurdle rather than skipping them, which is the
same rule seen from the other end.

The hurdles are asked in the order they are declared and the **first refusal wins** — nothing after
it is run, and what comes back names one hurdle and shows its output. Where the refusal appears
depends on which step it stopped: a `nxc send --to <channel>` that cannot open its first step fails
outright, with the hurdle named; a step further along the flow is reported in the `warnings` of the
reply that would have started it, under the class `precondition_refused` and distinct from
`step_skipped`, which means something broke rather than a rule said no. Nothing retries a refused
step: a hurdle refuses because the world is not ready, so asking again is your move.

**What you do not have to declare.** Some things hold whether or not you write anything, because
they are properties of the machinery rather than of your project — chiefly that no session of an
earlier step of the same channel is still running when the next one is about to start, on a channel
that claims the working copy. Those are deliberately not expressible here: a rule you have to
remember to ask for is a rule half the workspaces will not have.

**What it costs, said out loud:** one process per declared hurdle per step. A quorum of four
checkers with two hurdles is eight extra processes per round. Keep hurdles to fast, read-only
questions; they run inside the call that would have started the step, and while they run nothing
else can use the workspace.

**So a step's hurdles share ONE time budget** — a minute, all of them together. It is checked before
each hurdle, so a hurdle is never *started* once the budget is gone; it comes back refused, saying
that the ones before it used the budget up. That is why there is no limit on how many hurdles you
may declare: the number was never the thing worth capping, the time was.

**And they are frozen with the rest of the declaration.** A hurdle is executable code in a file the
declared agents can edit, so an operation checks against the declarations it *opened* under: editing
`preconditions:` while a chain is running does not switch it off for that chain, and the change
takes effect in the next operation. That is a real limit as well as a guarantee — an agent that
slips between two operations is not caught by it.

## Front doors

```yaml
- name: front-desk
  kind: public
  members: [triage]
```

`kind:` is `group` (the default) or `public`. A public channel is the project's front door: its
messages and boards are readable without membership, and it is discoverable across workspaces that
share a sync stream. It is a **read opening and nothing else** — the membership-scoped listings stay
member-only, and writing was never membership-gated for any kind.

`direct` is deliberately not a declarable kind: a direct conversation is *derived* from two handles
and minted for you, so declaring one is not something that could be honoured — and letting the word
parse would mean a typo silently producing a group channel.

## What happened to `nxc workflow`

There was a `workflow` verb group — `start`, `step`, `status`, `tick`, `done`, `bind`, `append`,
`show`, `expect` — with a run record behind it. All of it is gone, and this page is its replacement,
not its documentation.

The reason is one sentence from the design: *a channel is a work sequence; a "real" workflow is the
same term with a fixed order and defined write permission; there is no second term for it.* A fixed
order of a role step followed by a channel step — the shape every declared workflow actually had —
is `flow: sequential`. And "defined write permission" needed no field at all: every thread has
exactly two ends, only a thread's opener may re-declare its expectation, and the supervisor identity
is reserved as a class. Who may write what, and where, is settled by the data model rather than by a
declaration that could disagree with it.

So the mapping is:

| Was | Is |
| --- | --- |
| `workflow start` | `nxc send --to <declared channel>` |
| `workflow step done` | `nxc reply --thread <slot>` — the supervisor decides what comes next |
| `workflow status` / `list` | `nxc status --thread <id>`, or bare `nxc status` |
| `workflow expect` | the channel's `expects:`, declared |
| `workflow tickets add` | `--ref nxf_ids=<id>` on the `send` that opens the round |

## Next

- [writing-declarations](nxc-writing-declarations) — when a round carries and when it dilutes, what
  a `description` has to answer, and what a `summary_prompt` must demand.
- [personas](nxc-personas) — the members these channels name.
- [commands](nxc-commands) — the verbs that address them.
- [limits-and-safety](nxc-limits-and-safety) — depth, leases, and the human gate.
