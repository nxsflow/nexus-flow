# Personas

A **persona** is one agent, declared in one file: `.nxs-personas/<handle>.yaml`. `nxc` reads that
folder; nothing but you writes it. There is no `register` verb, no built-in personas, and no way to
conjure one at runtime — which is the point. A declared agent is one you can read, diff, review and
commit, and once committed it is still there tomorrow. **Commit it** — see
[the folder belongs in version control](#the-folder-belongs-in-version-control), which is not
housekeeping advice.

The minimum is two lines:

```yaml
handle: coder
system_prompt: |
  You are the coder. Do what the trigger message asks; an answer from you means it is done.
```

Everything else has a default that means what it meant before the field existed, so a declaration
never breaks by standing still.

## Who it is

```yaml
handle: coder
job_title: Coder
job_description: Implements a work order on a branch and merges it.
expected_output: A short report of what changed, and the branch it is on.
```

`handle` is the address — what you type after `send --to`. By convention it is also the filename
stem, though the YAML field is the source of truth.

`job_title` and `job_description` are not decoration. They are what `nxc list` shows a human
deciding whom to address, and what the persona itself is told at session start. A handle with no
description is a name in a directory that nobody can choose from — the one-liner does the work a
skill's frontmatter does, and it is worth a sentence of thought.

`expected_output` is the shape of the answer you want back. It reaches the persona's own identity
block, so it is the cheapest way to make several agents' replies comparable.

A handle may not start with `__`. That prefix is reserved for the engine's own identities (a
channel's supervisor is `__channel__`), which is what makes those unforgeable by a declaration.

## What it is for

`system_prompt` is the persona's job, in your own words, and it is the last thing the model reads
before the conversation itself. The composed prompt is layered, in this order:

1. **The prime block** — what `nxs prime --persona <handle>` composes, and you can print it and
   read it yourself. It is the suite's session start, assembled in module order: the board
   (`nxf prime`), the project's memories (`nxm prime`), then chat's own block — this persona's
   identity (title, job, expected output, seniority, how it is reachable), whom it may address and
   what for, the `nxc` command reference, and its answering rules. It goes first so that "who am I
   and how do I answer" anchors everything after it.
2. **The project's `CLAUDE.md`**, when the persona's `claude_md:` policy asks for it.
3. **The persona's own `system_prompt`** — the role, in your words.
4. **The task the step declared**, when this persona was summoned by a step of a channel's flow that
   carries a `task:` (`nxc guide channels`). It sits under the role because it is the narrower of
   the two, and it is what the role could not know: which of the several jobs it serves this station
   is. It arrives under its own heading, so the model can tell it from the role, and it **adds** —
   it can neither replace the persona nor switch a part of it off, and the prompt says so in a
   sentence of the engine's own. A step that declares no `task:` composes exactly the three layers
   above, byte for byte.

The answering rules in layer 1 are worth reading literally, because there are exactly **two** of
them and no third: answer in the thread you were handed, or say — in that same thread — that you
cannot. Both are `nxc reply --thread <id>`, and **both end your turn.** There is no way to say what
you are missing and carry on: you reply, your turn ends, and while the round is open the answer
resumes you with everything you already know. (There is one ending with no *reply* in it — waiting
for a round you commissioned yourself, which is ending the turn and saying nothing; it is not a
third answering rule, and the forced ending below spells it out.) Once a round has been answered and handed on it is
closed — a reply into one of its threads is refused, and says so — so the next thing starts with
`send --to`, which is also how you start something new with somebody else. `nxc list` shows who is
there.

Three switches shape those layers:

```yaml
prime: true            # default. false opts out of layer 1 for a narrow role whose prompt covers it
claude_md: inherit     # default | ignore (do not show project conventions) | override (reserved)
base_prompt: claude_code   # default. `none` runs without the Claude Code preset underneath
```

`prime:` also takes a map, when a persona should be given some of the suite and not the rest:

```yaml
prime:
  flow: false          # no board for THIS persona — a pure reviewer does not need one
  memory: true         # the project's memories (the default)
  chat: true           # its identity, address book and answering rules (the default)
```

Anything you leave out stays on, so the map above says one thing and changes one thing. The filter
is per persona: excluding the board here removes that section from **this** persona's block and
from no other.

> **`memory: false` can cost more than memories.** If this project has run `nxm migrate`, its
> conventions have moved OUT of `CLAUDE.md` and into the store — that is what the migration does —
> so `nxm prime` is where the project's rules now come from, and `claude_md: inherit` contributes
> nothing. A persona that excludes the memories in such a project has the rules from nowhere. The
> engine cannot tell a migrated project from an unmigrated one, so it does not refuse the
> declaration; it tells the session instead, in one line of its own prompt, that it has neither and
> should run `nxm prime` before changing anything it is unsure of.

Because layer 1 already teaches the answering loop, a trigger message can carry only the task. That
is the whole reason the field defaults to on.

**One sentence is not yours to switch off.** Whenever the trigger that starts a session declared
that a thread is waiting on its reply, the composed prompt carries the *forced ending* — and it
carries it even under `prime: false`:

```text
Obligation: thread <id> is waiting on your reply, and your turn may not end without one.
Give the message on STDIN, so that nothing in it is evaluated by the shell — the quotes
around EOF are what stop that:

nxc reply --thread <id> - <<'EOF'
<your message>
EOF

There are exactly two ways to end the turn, and they are that same command with one flag
added or left off:
- no flag — you are finished.
- `--escalate` — you cannot reach the result and need help or a decision.
A ONE-LINE answer may be an argument instead: `nxc reply --thread <id> --escalate "no
test workspace here"`. Never for longer text — backticks, `$(…)` and quotes in it are
rewritten by the shell before nxc sees them.
While this is the only conversation open for you, `--thread` may be left out: the same
command without it means this one.
```

Those forms are the whole set of REPLIES, and the paragraph goes on to name the one ending that has
no reply in it: a session waiting for a round it commissioned itself ends its turn and says nothing.
It is woken when the answer arrives, and a plain `nxc reply` while its own round is open is refused —
that would mean "I am finished". The rule sits in the engine rather than in your declaration on
purpose — a step of a declared flow ends when its session answers, so a declaration that forgot to
say it would leave the flow with nothing to read.

**A step that can send work back is told about a third form**, and only such a step: where the
channel declares `on_needs_rework:` for it (`nxc guide channels`), the same paragraph offers
`--needs-rework` as a third flag on that one command. The offer hangs on the forced
ending rather than on the usage block for the reason above — a role with `prime: false` is still
obliged to answer, and an obligation whose means nobody supplies is the defect this rule exists to
prevent.

## What it runs as

```yaml
stage: senior          # junior | senior | principal
model: opus            # fable | opus | sonnet — beats `stage` when both are given
tools: [Bash, Read, Write]
permissions: acceptEdits
```

**`stage` is a vocabulary that is not a model name.** You decide how much thinking a job is worth
without knowing which models exist this month: `junior` → Sonnet, `senior` → Opus, `principal` →
Fable. One table joins the two, so the declaration's spelling and the engine's choice cannot drift
apart. Reach for `model:` only when you mean something the band cannot express — and note the
precedence is deliberate: the band is the coarse, discussable choice, a named model is you
overriding it.

**A step of a channel's flow may raise or lower the band** (`stage:` on the step, `nxc guide
channels`) — the same reviewer judging a diff at one station and a design at another, where the
second is dearer thinking. It moves the band and nothing else: a step may not name a model, and a
persona that declared its own `model:` runs on it whatever a step asks for.

`tools` has three states, and the difference matters. Omit it entirely and the agent runtime's own
full default toolset applies. Write `tools: []` and the persona explicitly has none — a narrow,
non-agentic role. Write a list and it gets exactly that list. An omitted key and an empty list are
*not* the same thing.

A persona that will run `nxc` at all needs `Bash`, since that is how it answers — and where the
engine *orders* an answer, it grants that itself. Every commission tells the persona, in its own
system prompt, to end its turn with `nxc reply --thread <id>`, so whoever imposes that obligation has
to make sure it can be carried out: the trigger adds `Bash` to what the session may run, on top of
whatever you declared, without narrowing the toolset an omitted `tools:` gives it. You still declare
`Bash` for a persona that needs the shell for its *work*; what you no longer have to remember is that
answering is itself work.

## Who may address it

```yaml
addressable: general        # the default: anyone may open a conversation directly
addressable: none           # nobody may — come through a channel that casts me
addressable:                # exactly these callers, and nobody else
  personas: [pm]
  humans: true
```

**The mapping is a whitelist, and an omitted half names nobody of that class.** That is what makes
the two cases people actually hit one form rather than two:

- `{humans: true}` — the person at the terminal may send a direct message, no persona may. The
  shape for a PM you want to be able to message yourself while every agent goes through the round
  it runs, so the sentence in its prompt ("that channel is the only way anybody reaches you") is an
  assurance rather than a convention.
- `{personas: [head-of-marketing]}` — one named peer may call this persona individually, a person
  addresses the channel. The specialist case, the other way round.

`{}` says what `none` says. And `addressable: [review]` — a list of channel names — still parses and
still means "nobody directly", but it is **deprecated**: which channels a persona takes part in is
the channel's own business (`nxc guide channels`), and writing it here too is one fact in two
places. Write `none`; `nxs prime` will tell you as well.

### Why a caller-derived limit is admissible here

The rule this surface is built on is that dropping your identity must never let you do *more* —
that is why the address book below is guidance and never enforcement. The distinction an earlier
version of this page drew too widely:

- `general` and `none` read the same for every caller, exactly as they always did.
- A whitelist can only ever **refuse** somebody `general` would have admitted. Anonymity resolves
  to "a human", so on a `{personas: [...]}` persona dropping your identity grants strictly *less* —
  the direction the rule demands.
- On a `{humans: true}` persona it grants *more*, and that is the honest cost of the second
  direction. What bounds it: `nxc send` has no `--persona`. The caller's class comes from the
  session stamp this workspace itself minted, and a persona that drops that stamp to slip through
  stops being itself for the call — no return address, no resume, nothing for the answer to be
  attributed to. It does not get the same thing plus; it gets a worse thing.

Either way, this is **correctness rather than defence**, like every check in `nxc`: on a single
machine no boundary here is stronger than access to the workspace file (`nxc guide
limits-and-safety`). What it buys is that a declaration means what it says.

### What it does to `nxc list`

A persona the reader may not address directly is not an entry of its own — the channel that casts
it is. That is usually what you want: if the way to reach four reviewers is to address the round
they are in, listing them individually invites exactly the call you do not want. The rendering
follows the *reader*, so the same declaration can show a persona to you and hide it from an agent.

## Who it may address

```yaml
address_book:
  - to: review
    why: to get a change judged before merging
  - to: pm
    why: to report that a work order is done
```

The **`why` is the load-bearing half** — it works like a skill's one-line description, a sentence a
model chooses from, which is why it is rendered everywhere the target is.

**It has three states, exactly like `tools:`, and the difference matters.**

```yaml
# key omitted            -> "nobody wrote this down": the persona is shown the whole declared team
address_book: []         # -> "commissions nothing": the persona is shown nobody at all
address_book: [{to: pm}] # -> that book, in the author's order
```

`address_book: []` is the declaration for a role at a **leaf** of the tree: a pure reviewer, a
summarizer, anything whose only outbound call is the reply on its own thread. Saying it in the file
is the point — before this, the alternative was a sentence in the `system_prompt` ("you commission
nothing; ignore the list"), which is a prompt argued against a declaration.

An **omitted** key stays "not written down" and shows the whole team, so every persona declared
before this existed means exactly what it always meant.

One thing the derived list leaves out: **the channels the persona is a member of.** Offering a
reviewer the `review` round it sits in is help in no reading, and commissioning your own channel is
not declaration cyclicity — that is refused when the catalogue loads — so nothing else would catch
it. A book you write yourself is honoured entry for entry, that channel included: the file is the
authority.

Note what the address book deliberately does *not* do in this milestone: it does not restrict. It is
guidance a persona reads, for the same reason as above — deriving a limit from a droppable identity
would be worse than useless.

## How it learns the answers it commissioned

A persona is never told to poll. When it sends work out and its own turn ends, the coordinator holds
whatever comes back and starts the session again with **all of it at once** — a head naming how many
messages arrived, one delimited block per message carrying who posted it and which thread it answers,
and a line saying what is still outstanding. That is the same shape a channel round's answers arrive
in, so a persona that has read one can read the other.

Two things about that delivery are worth knowing before you write a persona's prompt:

- **A hand-back is set apart.** "I need help, or a decision" stops a chain, so it is never mixed
  into the stack: it arrives first, under its own notice, ahead of the ordinary answers.
- **"Everything that arrived" is not "all the answers".** Two of three may answer in seconds and the
  third in twenty minutes. The delivery says which commissions are still owed, by thread and by
  handle, so the persona does not have to keep count itself.

If nobody could wake it — its session was killed, or the machine was off — nothing is lost: its next
**session start** names the commissions that finished while it was away. That notice is a window, not
a debt: it covers what finished since this persona's previous session ended, so it is seen once and
does not have to be acknowledged. `nxc status` answers the same question at any time.

## What it needs to work

```yaml
working_tree: exclusive        # shared (default) | exclusive
```

`exclusive` says this persona's sessions need sole use of the repository's working copy and build
directory. A second chain that would collide waits instead. It is declared rather than inferred
because a persona may be named anything and the runtime cannot guess from a task that a checkout is
about to be branched. The full mechanics — what a claim covers, what releases it, and what it does
not protect you from — are in [limits-and-safety](nxc-limits-and-safety).

## Where it runs

```yaml
machine: studio                # a machine id or name from `nxs sync machines`; default: where the chat starts
```

A chat with this persona runs on exactly **one** machine. Every other machine that syncs the
workspace sees the chat and starts nothing. The machine is decided when the chat starts, in this
order: `nxc send --to <persona> --machine <m>` for that one chat, then this `machine:`, then the
machine that starts the chat. The answer is written into the chat itself, so every machine reads the
same one, and a later `nxc reply --thread <id> --machine <m>` hands the chat to another machine.

When the machine a chat would run on is **not online**, nothing is sent and you are asked instead:
the refusal lists the machines that are online, and naming one with `--machine` is the answer. The
designated machine's service picks the chat up within about half a minute of it being written, but
only from a machine whose key it trusts (`nxs sync trust add`): an order from anywhere else stays
visible in the thread and starts nothing.

All of this applies to a workspace that syncs with other machines (`nxs sync bind`). One that syncs
nowhere has a single machine, so nothing is designated there and a chat starts where it is written.

`machine:` is read only when a person starts a chat. A chat that a running persona starts runs on
that persona's machine, because the answer has to come back to the session that asked. A persona
commissioned as a channel's member runs wherever the channel was started.

## A worked example

```yaml
handle: coder
job_title: Coder
job_description: Implements a work order on a branch and merges it.
working_tree: exclusive
system_prompt: |
  You are the coder. Implement the work order in the message you were handed: on a branch of
  its own, with the project's own gates green before you merge it.

  An answer from you means the branch is merged and those gates were green on the tree that was
  merged. If you cannot get there — something is missing, or a decision came up that is not
  yours to make — say what you are missing in your thread rather than answering.
tools: [Bash, Read, Write]
permissions: acceptEdits
```

**Three things in that prompt are worth copying into your own, and one absence carries all three.**
It names a ROLE and never a position — nothing in it claims to be the first step of anything, so
the same file is still true the day the channel gains a step, and a persona that asserts its
position gets it wrong (measured: one claimed three steps where there were four). It says what
counts as FINISHED *here*, which is the one part of the answering loop that is yours to declare.
And it names what the coder must bring back rather than how to send it.

The absence is the point. There is no "How you finish" section, no `nxc` invocation, no heredoc:
the engine writes the forced ending into every turn that owes a reply, carrying the real thread id
and each way the turn may end — see "One sentence is not yours to switch off" above. A copy here
would say the same thing with a placeholder where the truth goes, and would go on saying it after
the engine had moved on. `nxs prime` reports one as a **declaration warning** when it finds it, and
`nxc guide writing-declarations` is the topic that works through that class and six others.

## Declared but not yet read

Trustworthy documentation says which fields are inert. These parse, round-trip and validate, and
nothing in the engine acts on them today:

- **`session: fresh | continue`** — inert because **the persona is not what decides**. `nxc
  send --to <persona>` mints a fresh session; a `nxc reply --thread` into that persona's own thread
  resumes the session it already has, with everything it already knows; and inside a channel that
  declares `steps:`, the step being entered decides for itself with its own `resume:` (`nxc guide
  channels`). A policy on the persona has nothing left to decide that the verb or the step has not
  decided already.
- **`sub_agents: true | false`**.
- **`reports_to: <handle>`**.

Declaring them costs nothing and documents intent; do not build a process on them behaving.

**`claude_md: override` is the one to be careful with**, because it is only HALF unread. The
per-persona replacement document it names is not specified yet, so nothing composes one — but the
tag is not idle while it waits: it is simply not `inherit`, so a persona that declares it is
composed WITHOUT the project's `CLAUDE.md`, exactly as `ignore` would be. Declare `ignore` when that
is what you mean. `inherit` (the default) and `ignore` are both live.

## The folder belongs in version control

`.nxs-personas/` is not documentation of how your agents work — it *is* how they work, read fresh at
every spawn. And it lives in the same working copy the agents themselves are told to work in. That
is a feedback loop no other configuration in this system has: a `git switch`, a `git stash` or a
`git checkout -- .` by one persona changes how the **next** persona thinks.

So an uncommitted edit to a declaration is not really an edit at all. It is a change one branch
holds and every other branch does not, in a folder whose contents decide the rules — and the thing
that reverts it is ordinary, correct branch hygiene by somebody who has no way to know your file
mattered. That is not a hypothesis: it is what happened here. Three declarations were rolled back by
exactly that route, one of them a rule telling a persona to end every turn with an answer; the
persona then ran without it and the runtime answered in its place on four threads before anyone
noticed. It was found by hand, by comparing a stored session spec against the file on disk.

Commit the folder, and review changes to it the way you review code. Two things help you notice when
it slipped anyway, and neither is a lock — the files stay editable at every moment, because what has
to be stable is a running **operation**, not the directory:

- Every spawned session records the version of the declaration its prompt was built from, as
  `declarationHash` in `.nxs/agent-logs/<session>.spec.json`. "Did this session run under the rule I
  wrote?" is a comparison, not a text search through a prompt.
- When a persona is started under a different declaration than the last time it ran, the receipt of
  the call that started it says so — a `declaration_changed` entry in `warnings`, naming both
  versions. It is a **warning and not a refusal**: changing a persona and then addressing it is the
  ordinary way to work, and the exit code stays `0`. A change is usually intended. An unnoticed one
  never is.

Both of those watch **the persona's own file**, and nothing else in the folder. A `channels.yaml`
that was rolled back the same way — different members, a different `working_tree:`, a different
`timeout:` — steers your agents just as much and is **not** reported by either. So the advice above
is not "the tooling has your back": it is version control that has your back, and this is a second
pair of eyes on the half of the folder it can see.

## When a declaration is wrong

A malformed file is a loud `validation` error naming the path — never a silently skipped persona. A
missing folder is *not* an error: a workspace with no declarations resolves cleanly and simply has
nobody to address, and `nxc list` and `nxs prime` say so in as many words.

Referential problems — a channel naming a persona that does not exist, a persona declaring itself
reachable through a channel it is not a member of — are surfaced in `nxs prime`, and only in an
interactive context. A spawned persona is not shown its author's mistakes; a human at a keyboard is.

**Quality warnings arrive in the same place, under the same rule.** A declaration that copies text
the engine supplies anyway, or whose `job_description` cannot tell a caller when to call it, is not
broken — so it is a *warning*: it is listed for the human, it excludes nothing, and the declaration
loads and runs exactly as it would otherwise. What is checked, what deliberately is not, and the
four error classes no check can decide are in [writing-declarations](nxc-writing-declarations).

## Next

- [writing-declarations](nxc-writing-declarations) — how to fill these fields so the answers are
  usable: seven measured error classes, and a checklist.
- [channels](nxc-channels) — putting several personas to work together.
- [commands](nxc-commands) — addressing what you just declared.
- [limits-and-safety](nxc-limits-and-safety) — the caps around a running persona.
