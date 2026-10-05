# Personas

A **persona** is one agent, declared in one folder: `.nxs-personas/<name>/SKILL.md`. `nxc` reads
that folder; nothing but you writes it. There is no `register` verb, no built-in personas, and no
way to conjure one at runtime — which is the point. A declared agent is one you can read, diff,
review and commit, and once committed it is still there tomorrow. **Commit it** — see
[the folder belongs in version control](#the-folder-belongs-in-version-control), which is not
housekeeping advice.

The file is a **skill**, in the form of the [Agent Skills specification](https://agentskills.io):
a YAML frontmatter between two `---` lines, then the instructions. The minimum is three lines and a
sentence:

```markdown
---
name: coder
---
You are the coder. Do what the trigger message asks; an answer from you means it is done.
```

Everything else has a default that means what it meant before the field existed, so a declaration
never breaks by standing still. The specification's own fields sit at the top of the frontmatter;
everything nexus-flow needs beyond them sits under ONE key, `nxs:`. A skill that has no `nxs:` at
all is a persona with every default — see
[a published skill is a persona](#a-published-skill-is-a-persona).

The older form, one `<handle>.yaml` per persona, is still read — see
[the older YAML form](#the-older-yaml-form) for how its fields map, and `nxs personas migrate` for
the command that rewrites it.

## Who it is

```markdown
---
name: coder
description: Implements a work order on a branch and merges it.
nxs:
  title: Coder
  expected_output: A short report of what changed, and the branch it is on.
---
```

`name` is the handle, the address — what you type after `send --to`. By convention it is also the
folder's name, though the field is the source of truth. The Agent Skills specification has a rule
for it: 1 to 64 characters, lowercase letters, digits and hyphens, no hyphen at either end and no
two in a row, and the same as the folder's name. `nxs prime` warns when a name breaks it; nothing
refuses one, but a skill tool that checks the rule will.

`description` and `nxs.title` are not decoration. They are what `nxc list` shows a human
deciding whom to address, and what the persona itself is told at session start. A handle with no
description is a name in a directory that nobody can choose from — the one-liner does the work a
skill's frontmatter does, and it is worth a sentence of thought.

`nxs.expected_output` is the shape of the answer you want back. It reaches the persona's own
identity block, so it is the cheapest way to make several agents' replies comparable.

A handle may not start with `__`. That prefix is reserved for the engine's own identities (a
channel's supervisor is `__channel__`), which is what makes those unforgeable by a declaration. And
no persona may be called `channels`: that is the folder holding one file per channel (`nxc guide
channels`).

## What it is for

The **body** of `SKILL.md` — everything after the frontmatter — is the persona's job, in your own
words, and it is the last thing the model reads before the conversation itself. The composed prompt
is layered, in this order:

1. **The prime block** — what `nxs prime --persona <handle>` composes, and you can print it and
   read it yourself. It is the suite's session start, assembled in module order: the board
   (`nxf prime`), the project's memories (`nxm prime`), then chat's own block — this persona's
   identity (title, job, expected output, seniority, how it is reachable), whom it may address and
   what for, the `nxc` command reference, and its answering rules. It goes first so that "who am I
   and how do I answer" anchors everything after it.
2. **The project's `CLAUDE.md`**, when the persona's `claude_md:` policy asks for it.
3. **The body of the persona's `SKILL.md`** — the role, in your words, unchanged.
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

**A relative path in the body means the persona's own folder.** That is the Agent Skills rule: a
skill keeps `references/`, `scripts/` or `examples/` beside its `SKILL.md` and says
"load `references/guide.md`". It holds wherever the persona runs — in this repository, or from the
user-level folder in somebody else's — because the session is told at its start where its
declaration lives (the "Declared in" line) and that a relative path means that folder, unless the
instructions say a path is relative to the repository. The session's working directory is still the
repository it works in. (The older YAML form keeps the opposite rule: a relative path there means
the repository.)

A skill body is often written as instructions — "when X, do Y" — rather than as a role. That
carries, because the engine supplies the identity and the answering rules in layer 1 itself.

Three switches shape those layers, all under `nxs:`:

```yaml
nxs:
  prime: true            # default. false opts out of layer 1 for a narrow role whose body covers it
  claude_md: inherit     # default | ignore (do not show project conventions) | override (reserved)
  base_prompt: claude_code   # default. `none` runs without the Claude Code preset underneath
```

`prime:` also takes a map, when a persona should be given some of the suite and not the rest:

```yaml
nxs:
  prime:
    flow: false          # no board for THIS persona — a pure reviewer does not need one
    memory: true         # the project's memories (the default)
    chat: true           # its identity, its directory and answering rules (the default)
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

```markdown
---
name: coder
allowed-tools: Bash Read Write
nxs:
  stage: senior          # junior | senior | principal
  model: opus            # fable | opus | sonnet — beats `stage` when both are given
  permissions: acceptEdits
---
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

`allowed-tools` has three states, and the difference matters. Omit it entirely and the agent
runtime's own full default toolset applies. Write it with an empty value (`allowed-tools:`, or
`""`) and the persona explicitly has none — a narrow, non-agentic role. Write a list and it gets
exactly that list. An omitted key and an empty one are *not* the same thing.

The list is the specification's space-separated text (`Bash Read Write`); a comma-separated text
and a YAML list are taken too. An entry may be a rule rather than a bare tool — `Bash(git add *)` —
and is split only outside its parentheses. In Claude Code `allowed-tools` PRE-APPROVES tools
without restricting the rest; here it is both, because a persona runs with nobody at the keyboard to
approve a prompt: the rules are approved as written, and the tools they name are the persona's
toolset.

A persona that will run `nxc` at all needs `Bash`, since that is how it answers — and where the
engine *orders* an answer, it grants that itself. Every commission tells the persona, in its own
system prompt, to end its turn with `nxc reply --thread <id>`, so whoever imposes that obligation
has to make sure it can be carried out. What the trigger adds depends on what you declared, and
never narrows it: a persona that omits `tools:` gets `Bash`, as it always did; a persona that
declares a list without `Bash` — `tools: []` included — gets `Bash(nxc reply:*)`, which runs its
reply and no other command; a persona that lists `Bash` gets nothing more. You still declare `Bash`
for a persona that needs the shell for its *work*; what you no longer have to remember is that
answering is itself work. (What this paragraph says of `tools` holds for `allowed-tools`: it is the
same field.) `limits-and-safety` says exactly what the narrow grant lets through.

## Who may address it

```yaml
nxs:
  addressable: general        # the default: anyone may open a conversation directly
  addressable: none           # nobody may — come through a channel that casts me
  addressable:                # exactly these callers, and nobody else
    personas: [pm]
    humans: true
```

(Three alternatives, not one block — a frontmatter takes one of them.)

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
that is why the directory below is guidance and never enforcement. The distinction an earlier
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

### From another workspace

A persona of ANOTHER workspace on the same machine can commission this one, if the declaration says
so. Its address from outside is the workspace's name and the persona's handle:
`nxc send --to nxsflow/nexus-flow/pm`. A workspace's name is `<owner>/<repo>`; `nxs name` prints it
(derived from the git origin and stored on first use) and `nxs name <owner>/<repo>` sets it where
there is no origin. Inside a workspace nothing changes: `pm` stays `pm`.

```yaml
nxs:
  addressable:
    humans: true
    external:
      - "*/pm"                     # the PM of every workspace this one trusts
      - nxsflow/manufakt-io/pm     # or exactly this one
```

- **No `external`, no way in from outside** — under every form, `general` included: "anyone" means
  anyone in this workspace. A bare handle in `personas:` means this workspace's persona of that name
  and never admits a foreign one: `personas: [pm]` does not let another workspace's `pm` in.
- **Two checks, both required.** The caller's workspace is on this workspace's trust list, AND its
  role is listed in `external`. Trust a neighbour by name with
  `nxs sync trust add --workspace <owner>/<repo>` — **in both workspaces**: the answer comes back
  signed by the receiver's key, and an answer from a workspace the caller does not trust wakes
  nobody, so `send` refuses up front until it is trusted.
- **The role is stamped by the caller's coordinator**, from the session it started — never by the
  agent. A person does not come in from outside: they go to that workspace and write there.
- **A refusal says why**, in the caller's answer: *workspace unknown*, *not on this machine*, *not
  trusted*, *role not admitted*, *no such persona*, or *depth limit reached*.

What happens then is a consultation with one handover in the middle. The caller's commission is a
thread in its own log, and only that thread crosses: the receiver starts its persona in ITS working
copy, with ITS board and memory, and whatever that persona commissions to answer stays in its own
workspace. A question (`--escalate`) goes back to the commissioner — never past it to the other
workspace's owner — and the commissioner's answer on the same thread resumes the persona. The depth
cap holds for the whole chain across both workspaces — on the caller's word: the depth travels in
the commission's signed stamp, and the receiver cannot check it against a chain it never sees.
Trusting a workspace means trusting its coordinator, and a hostile one could stamp depth 0; trust
only workspaces whose coordinator you would run yourself. `nxc status` shows the thread in both
workspaces with the other side and one of four states: `submitted`, `working`, `input-required`,
`completed` (or `rejected` / `canceled`). Two hours without a sign of life — no message on the
thread, nothing in the receiving persona's transcript — cancel it and wake the commissioner with the
reason; `nxc withdraw` on the operation takes it back in the other workspace too.

`send` and `reply` carry a border thread across at once, so delivery does not need the background
service; the service's pass carries the rest. One case does need it, as it does inside a workspace:
an answer that arrives while its commissioner is still finishing its turn is held, and handed over
by the service when that turn ends.

**Rolling it out.** `addressable`'s mapping refuses keys it does not know — that is what catches a
typo like `persona:` — so an engine older than `external` cannot load a persona file that carries
it. Add `external:` only to a repository whose every reader, apps that pin an engine included, runs
a version that knows it.

## Who it may address

Nobody declares whom a persona may address. Its session start lists the team **derived** from what
everybody else declares: every persona and channel of the workspace that admits it, each described
in its own words (`description`, a channel's `description`), with the channel route in for a
persona that admits nobody directly. A target appears in a persona's directory because the target
says so in its own `addressable` — one place, written by the one who is addressed.

One thing the list leaves out: **the channels the persona is a member of.** Offering a reviewer the
`review` round it sits in is help in no reading.

The list is guidance a persona reads, not a limit: deriving a limit from a droppable identity would
be worse than useless (see above).

**`address_book` is retired.** The older form let a persona carry a list of whom it addresses and
why. It was a second place the route was written down, it could disagree with the target's own
`addressable`, and its `why` lines went stale when a target's job changed. A declaration that still
carries it loads; the key is ignored, `nxs prime` says so, and `nxs personas migrate` leaves it out.
`address_book: []` — "this persona commissions nobody" — has no successor either: a persona sees
the targets that admit it.

## What it says it needs

```yaml
nxs:
  requires: [board, memory, mail]
```

`nxs.requires` names the capabilities a persona needs, in free words. **In this release it is read
and shown, and nothing more:** no vocabulary checks the names, and nothing binds one to a tool, a
subscription or an account. `nxs prime` says so once when any persona names one, and
`nxc list --json` carries the list. Binding a capability per environment — "mail" meaning Outlook on
one machine and Gmail on another — is a later release; do not build on it yet.

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
nxs:
  working_tree: exclusive        # shared (default) | exclusive
```

`exclusive` says this persona's sessions need sole use of the repository's working copy and build
directory. A second chain that would collide waits instead. It is declared rather than inferred
because a persona may be named anything and the runtime cannot guess from a task that a checkout is
about to be branched. The full mechanics — what a claim covers, what releases it, and what it does
not protect you from — are in [limits-and-safety](nxc-limits-and-safety).

## Where it runs

```yaml
nxs:
  machine: studio   # a machine id or name from `nxs sync machines`; default: where the chat starts
```

A chat with this persona runs on exactly **one** machine. Every other machine that syncs the
workspace sees the chat and starts nothing. The machine is decided when the chat starts, in this
order: `nxc send --to <persona> --machine <m>` for that one chat, then this `nxs.machine`, then the
machine that starts the chat. The answer is written into the chat itself, so every machine reads the
same one, and a later `nxc reply --thread <id> --machine <m>` hands the chat to another machine.

When the machine a chat would run on is **not online**, nothing is sent and you are asked instead:
the refusal lists the machines that are online, and naming one with `--machine` is the answer. The
designated machine's service picks the chat up within about half a minute of it being written, but
only from a machine whose key it trusts (`nxs sync trust add`): an order from anywhere else stays
visible in the thread and starts nothing.

All of this applies to a workspace that syncs with other machines (`nxs sync bind`). One that syncs
nowhere has a single machine, so nothing is designated there and a chat starts where it is written.

`nxs.machine` is read only when a person starts a chat. A chat that a running persona starts runs on
that persona's machine, because the answer has to come back to the session that asked. A persona
commissioned as a channel's member runs wherever the channel was started.

## A worked example

```markdown
---
name: coder
description: Implements a work order on a branch and merges it.
allowed-tools: Bash Read Write
nxs:
  title: Coder
  working_tree: exclusive
  permissions: acceptEdits
---
You are the coder. Implement the work order in the message you were handed: on a branch of
its own, with the project's own gates green before you merge it.

An answer from you means the branch is merged and those gates were green on the tree that was
merged. If you cannot get there — something is missing, or a decision came up that is not
yours to make — say what you are missing in your thread rather than answering.
```

**Three things in that body are worth copying into your own, and one absence carries all three.**
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

Trustworthy documentation says which fields are inert. Three keys of the older form never had an
effect, and are ignored with a warning from `nxs prime`; `nxs personas migrate` leaves them out:

- **`session: fresh | continue`** — inert because **the persona is not what decides**. `nxc
  send --to <persona>` mints a fresh session; a `nxc reply --thread` into that persona's own thread
  resumes the session it already has, with everything it already knows; and inside a channel that
  declares `steps:`, the step being entered decides for itself with its own `resume:` (`nxc guide
  channels`).
- **`sub_agents: true | false`**.
- **`reports_to: <handle>`**.

**`nxs.claude_md: override` is the one to be careful with**, because it is only HALF unread. The
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

Both of those watch **the persona's own file**, and nothing else in the folder. A channel file
that was rolled back the same way — different members, a different `working_tree:`, a different
`timeout:` — steers your agents just as much and is **not** reported by either. So the advice above
is not "the tooling has your back": it is version control that has your back, and this is a second
pair of eyes on the half of the folder it can see.

## A published skill is a persona

A skill you found — a folder with a `SKILL.md`, perhaps `references/` and `scripts/` beside it —
is a persona the moment it lies in `.nxs-personas/`: copy the folder in, unchanged, and the next
`nxc list` shows it. Nothing has to be rewritten and no command has to run. Putting it there is the
decision; `nxc` reads no other skill location (`.claude/skills/` included) by itself.

- **Without `nxs:` it runs with every default:** addressable by anybody inside this workspace and
  by nobody outside it, at the default stage, with the tools its `allowed-tools` names, or the full
  default set when it names none.
- **Fields nexus-flow does not read are left alone.** Claude Code and other runtimes add fields of
  their own to the frontmatter (`when_to_use`, `hooks`, …); they are not an error. `license`,
  `compatibility` and `metadata`, the specification's own, are passed through: not read, kept by
  `nxs personas migrate`, and shown in `nxc list --json` under `declaration`, beside the file, its
  form and its folder. To a human a persona is a persona — `nxc list` does not show the form.
- **A nexus-flow key at the top level is not read.** `addressable: none` belongs under `nxs:`;
  written at the top it would be ignored, so `nxs prime` warns about it. A key under `nxs:` that
  names no field is warned about too.

## One definition for every repository

A persona you want in every repository — the same `pm` in each — does not have to be copied into
each one. Beside a workspace's own `.nxs-personas/` there is a **user-level folder**,
`~/.nexusflow/personas/`, laid out the same way: one `<name>/SKILL.md` per persona and one
`channels/<name>.yaml` per channel (or the older `<handle>.yaml` and `channels.yaml`). Every
workspace on the machine reads it.

- **Merged per name.** A persona or channel the repository declares itself **hides** the user-level
  one of the same name; everything else is added. A channel comes along with the personas it
  presupposes, and its members resolve against the merged team — a user-level `planning` channel
  naming `coder` reaches each repository's own `coder`.
- **Identity stays per repository.** One definition `pm` is a participant of its own in every
  repository it runs in, with that repository's board, memory and working copy.
- **Where it came from is shown.** `nxc list` marks each entry from the user-level folder and says
  which entries the repository hides; `nxc list --json` carries `"origin": "user"` on such an entry
  and `user_path`, `from_user` and `shadowed` under `declarations`. `nxs prime` says the same under
  "Declarations". A hiding is reported there and nowhere else — not when an operation opens, not in
  the session it starts. A repository that keeps its own copy runs that copy, with its own hurdles,
  members and access rules; the user-level folder cannot forbid it.
- **How the folder is filled is yours to decide** — a git clone, a symbolic link. The folder is the
  contract, not its history.

**The price, said out loud:** a declaration outside the repository is not versioned with it, and a
clone on a machine without the folder has no `pm`. Everything said above about version control holds
for this folder too — keep it in a repository of its own.

Three things hold exactly as they do for the repository's own folder:

- **It is frozen per operation.** What an operation runs under is the merged catalogue as it stood
  when the operation opened — hurdles (`preconditions:`) from the user-level folder included. An
  edit there reaches the next operation, never a running one.
- **A relative path follows the form.** In a `SKILL.md` it means the persona's own folder, so a
  user-level skill finds its `references/` in every repository it runs in — that is the Agent
  Skills rule, and the session is told where the folder is ("Declared in"). In the older YAML form
  it means the repository the persona runs in: a prompt that says "read `knowledge/x.md`, relative
  to the root of this workspace" is read by a session whose working directory is the repository, and
  a hurdle runs there too. A YAML persona from the user-level folder is told where its declaration
  lives as well, so its instructions can point there.
- **An embedding app reads the same folder.** It does not choose another one, and cannot: the
  personas it starts run `nxc`, which reads this folder, and two answers to "who exists" would split
  an app from its own sessions.

**An `external` from the user-level folder opens no repository by itself.** A user-level persona
that admits callers from outside is admissible in every repository that runs it, but only from the
workspaces that repository trusts by name in its own trust list (`nxs sync trust add --workspace`).
A repository that trusts nobody admits nobody from outside.

A development build reads a user-level folder only under a named service instance —
`~/.nexusflow-<name>/personas/` when `NXS_SERVICE_INSTANCE` names one. A build without one (CI, a
shell without `direnv`) reads none, so a build under test never picks up the personas your installed
suite runs with.

## The older YAML form

Before the skill form, a persona was one `<handle>.yaml`. It is still read, in the repository's
folder and in the user-level folder alike, and it maps field for field:

| YAML form | Skill form |
| --- | --- |
| `handle` | `name` |
| `job_description` | `description` |
| `system_prompt` | the body of `SKILL.md` |
| `tools` | `allowed-tools`, with the same three states |
| `job_title` | `nxs.title` |
| `expected_output`, `stage`, `model`, `addressable`, `prime`, `claude_md`, `base_prompt`, `permissions`, `working_tree`, `machine` | `nxs.<the same name>` |
| `address_book`, `session`, `sub_agents`, `reports_to` | left out — ignored, with a warning |

**One name, one declaration.** A `pm.yaml` beside a `pm/SKILL.md` is refused when the folder is
read, and the error names both files: two declarations of one name must not let one win silently.

**`nxs personas migrate`** rewrites a folder from the older form into the new one: every
`<handle>.yaml` into `<handle>/SKILL.md`, the list in `channels.yaml` into one
`channels/<name>.yaml` per channel, and it removes the old files. It keeps your comments with the
keys they stand above, moves the notes heading `channels.yaml` into `channels/README.md`, and
reports per file what it left out. Every rewritten file is read back before anything is written and
must give the same declaration; a key it does not know stops the run, naming the file, before a
single file changes. `--dry-run` shows the plan and writes nothing, `--user` migrates the user-level
folder, and a second run finds nothing to do.

An app that embeds an engine older than this one does not see a skill folder at all — migrate a
repository an app reads only once the app runs this version.

## When a declaration is wrong

A malformed file is a loud `validation` error naming the path — never a silently skipped persona.
A `SKILL.md` without a frontmatter, or without a `name`, is one; a sub-folder without a `SKILL.md`
is not a declaration at all and is left alone, so a team can keep its `knowledge/` there. A
missing folder is *not* an error: a workspace with no declarations resolves cleanly and simply has
nobody to address, and `nxc list` and `nxs prime` say so in as many words.

Referential problems — a channel naming a persona that does not exist, a persona declaring itself
reachable through a channel it is not a member of — are surfaced in `nxs prime`, and only in an
interactive context. A spawned persona is not shown its author's mistakes; a human at a keyboard is.

**Quality warnings arrive in the same place, under the same rule.** A declaration that copies text
the engine supplies anyway, whose `description` cannot tell a caller when to call it, that still
carries `address_book` or an inert key, whose name breaks the Agent Skills rule, or that names
capabilities under `nxs.requires` that nothing binds yet, is not broken — so it is a *warning*: it
is listed for the human, it excludes nothing, and the declaration loads and runs exactly as it would
otherwise. What is checked, what deliberately is not, and the four error classes no check can decide
are in [writing-declarations](nxc-writing-declarations).

## Next

- [writing-declarations](nxc-writing-declarations) — how to fill these fields so the answers are
  usable: seven measured error classes, and a checklist.
- [channels](nxc-channels) — putting several personas to work together.
- [commands](nxc-commands) — addressing what you just declared.
- [limits-and-safety](nxc-limits-and-safety) — the caps around a running persona.
