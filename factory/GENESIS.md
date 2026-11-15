# GENESIS.md — nexus-flow

You are a senior engineer building nexus-flow: the place where coding agents, agents from other domains and the people who set the direction work together. Their work is documented in tickets, rules and decisions, and in the factory declaration that says how the work gets done. Agents use a single CLI, `nxs`, whose output is deterministic: JSON by default, Markdown with `--md`. A CLI made for people does it the other way round; this one serves agents first.

nexus-flow already works and is in daily use. We are rebuilding it as v2, which is less rigid than v1: the way of working becomes configurable, with minimal defaults, instead of fixed in the tool. v2 keeps what works and never breaks an existing workspace on the way.

Read this file before you start any ticket. Where a design document or a ticket disagrees with it, this file wins, and you flag the conflict to pm.

In this file, **the owner** always means the person who uses nexus-flow to direct their agents. Where the file speaks of nexus-flow's own maintainer, it says "I" or "me": that is Carsten.

## What we build

Agents do most of the work in a repository. They are fast and reliable when the rules and the state of the work are close to them, in files and a local database, not behind a web UI or an MCP. A team that lets agents work for days without watching needs six things written down: what the product is, which goals are in focus, what work is planned, what was decided, how work moves, and what happened today.

nexus-flow ships each of these as a module of its own. Where a module's data lives follows who must review it. What the owner reviews in a PR stays in files in git: GENESIS, and the factory's roles, merge rules, FACTORY and CONFIG. Tickets, decisions, goals and status notes live in NoCentralDB. The way of working is the one the NoCentralDB factory runs on (`factory/` in nocentraldb), made into a product. In v2, `decisions` replaces `memory` and `factory` replaces `chat`.

| Module | In v1 | What it holds | Stored in | Ships with |
| --- | --- | --- | --- | --- |
| `genesis` | new | The product: what we build, for whom, principles, no-gos, trade-offs, what goes back to the owner | Git | A GENESIS template, a guide for filling it in, and `nxs genesis create`, which starts a GENESIS from the template |
| `goals` | new | The goals in focus, across repositories, and which work serves them | NoCentralDB | A goal format and the focus rule |
| `tickets` | `flow` | Projects and tickets with dependencies, readiness derived from the graph, references to other repositories and to the goals a ticket serves | NoCentralDB | The ticket states and fields the factory needs, nothing more |
| `decisions` | `memory` | One record per decision, rules in force included | NoCentralDB | The decision format, an index, filters by ticket, topic and date |
| `factory` | `chat` | How work moves: roles, lanes, the merge gate, the gate command | Git | FACTORY.md, the pm and coder roles, a merge gate and a CONFIG a repository fills in |
| `status` | new | The daily note: what changed, what works now, what waits for the owner, what was decided | NoCentralDB | A daily-note template and a schedule |

`genesis` is mostly its template and guide; its module starts small, with `nxs genesis create`, and may grow later. `goals` and `tickets` are full modules of nexus-flow, which keeps their data itself.

**Strong opinions on what, open on how.** nexus-flow is openly opinionated: a factory needs a product description (genesis), goals, tickets, decisions, coordination across repositories, and guardrails such as the merge gate. A team that runs agents without one of them loses control or speed. How each is shaped is the team's call: how its GENESIS is structured, what a decision record looks like, which ticket states it uses and which rules its merge gate checks.

**Every default belongs to the team.** Each module ships with the least a factory needs to run. A team uses the templates as they are or adapts them, and a setting earns its place only when a real team needs it. An upgrade of nexus-flow never overwrites what a team has changed.

## How work moves

The factory is a combination of prompts and scripts: prompts where judgement is needed, scripts where a decision must come out the same every time.

### Roles and lanes

- **pm coordinates every role.** It turns intent into tickets, opens lanes, reviews what comes back and merges. The work of every role, coding or not, is documented in tickets.
- **Roles are files.** `factory` keeps each role as a file (`roles/<name>.md`), ships pm and coder as the standard, and a team adds its own, such as a legal or a marketing team.
- **Every role holds a lane.** pm opens a lane with a ticket as its trigger, for a legal agent exactly as for a coder.
- **Sub-agents are the framework's job.** v1's chat built its own orchestration: channels and step chains like build → verify → judge. v2 drops it. The agent frameworks move faster than we could, so how a session spawns and steers sub-agents is up to them. `factory` defines only roles and hand-offs: ticket → PR → review → merge.

### Decide first, then build

The goal is to get the owner's decisions before the work starts, so the work can then merge without the owner.

1. An agent that needs an owner decision asks for it in a ticket, by default; a team can change that. Work on it starts only after the decision.
2. The question stands in the ticket, in one place, not hours up a chat. `tickets` offers subscriptions to ticket comments, so a session learns when the owner decided as soon as its copy syncs, without polling (see Limits).
3. Watching a running session is the framework's job, not nexus-flow's.

### The merge gate

Whether the owner reviews a PR is decided in a semi-deterministic step:

1. Repository rules decide what they can, such as CODEOWNERS and the branch rules on the base branch.
2. For what they cannot catch, the team writes merge rules together with an agent.
3. A sub-agent reads the PR's changes and reports as JSON, per rule, whether and how the PR meets it, and whether the PR carries out the owner's decision or departs from it. This verdict per rule is a model's judgement.
4. A script reads that JSON and decides: pm merges, or the owner reviews first. Only this decision, on top of the verdicts, is deterministic.
5. The gate reports its decision as a required status check, and repository rules enforce it: a PR the gate reserves for the owner cannot merge without the owner's approving review, whatever an agent's prompt says.

The rules in a team's GENESIS on what comes back to the owner are checked the same way. The merge rules, and the role files that define exceptions to them, are under CODEOWNERS. Agents may propose a change to them, preferably after an owner decision in a ticket, and it merges only with the owner's approval. The rules, the JSON schema and the script belong to the team; nexus-flow ships a minimal set.

**Later option, not in v2's scope:** a fast decision model, such as Jev or Strands Decider, could take the sub-agent's place in step 3. Such models are fast, highly deterministic and cheap.

## What the modules keep

### Decisions and rules

The decision record follows the nocentraldb format: date and triggering ticket in the header, a fixed structure, topics, superseded-by, and filters by ticket, topic, date and index.

- **The header states the decision.** An agent that reads only the headers of the records in force already knows every rule and decision clearly. It reads the full record when it needs the reasons.
- **A rule is a decision in force.** A durable rule, such as "English for every artifact", is a decision marked as in force. v2 installs no session-start hook and writes no memory file; in v1 both caused more trouble than they were worth. Instead, a session's instructions say how to read tickets and decisions, as in nocentraldb today, and `nxs prime` prints the rules in force on demand, which keeps the pm prompt short.
- **A decision is never edited away.** A new decision supersedes it, and the old one stays readable.

**Later, not in v2's scope:** a command that evaluates decisions and rules. Some go stale without a newer decision superseding them; some turn into duplicates or small additions to each other. A model cannot tell, when it records a decision, how long it will hold. So compacting the log needs a model looking back over it. How is still open.

### Done is not yet on main

Tickets and decisions live outside git; the code they lead to lives on branches. When agents work side by side, a ticket can be done while its branch is not merged: other sessions see it closed but not its code. A working session needs to know both.

- **Two states.** "Done" means the work is finished and its result is linked. "On main" means it is merged into the team's base branch: `main` unless the team configures another one. Closing a ticket requires a linked PR or branch, or, for a role that works outside the repository, such as legal or marketing, a linked artifact. A ticket closed with an artifact has no code to merge, so it is on main once it is done.
- **"On main" is derived, never set.** `nxs` checks whether a commit on the base branch names the ticket, squash merges included. A commit may name tickets as a range: `NEX-13..NEX-23` names every ticket from NEX-13 to NEX-23. Like readiness in v1, a derived state is never stored, so it cannot drift from git; it is as fresh as the last fetch of the base branch (see Limits).
- **A PR closed without merge reopens its ticket,** and the ticket is flagged.
- **Dependent tickets may stack on a done ticket,** building on its branch, but count as unblocked only once it is on main.
- **Decisions follow the same line.** A rule is in force when it is made. A decision that describes code links its PR and is marked "not yet on main" until the PR is merged.

### Goals set the focus

`goals` holds what the team and its agents focus on now. A goal spans repositories. "Win 20 paying customers for nexflow.it" touches the repository that integrates the billing provider, the one that runs the waiting list, and the product itself and what it can do.

- **A ticket can serve several goals, or none yet.** pm ranks the work by the goals in focus. When a goal changes, no priority field on dozens of tickets needs updating.
- **Work for a goal goes first.** When a task arrives that serves a goal in focus, other work moves back. Agents may finish what they are working on, but they take on only the new work identified as important for the goal.
- **Goals live in the coordination space.** Each repository's pm sees the goal and ranks its own tickets against it.
- **A new goal starts a re-evaluation.** Clear goals often appear only after tickets exist. When a goal is added, a model proposes to the owner which tickets serve it, those without a goal and those already assigned to one alike. The owner confirms, and the tickets are assigned.

### Work across repositories

Several repositories may have to coordinate their work. They do it through requests in a shared space.

- **Each repository's tickets stay its own.** A NoCentralDB member holds admin, write or read on a space. Writing tickets would need write on the other repository's space, which is too much.
- **A coordination space joins a group of repositories.** It holds the group's goals and the requests between them. Every role that coordinates, the pm above all, is a member there and may write.
- **A request is shaped like a ticket.** It is addressed to a member, such as the pm of another repository, carries a status (open, accepted, declined, done) and has a thread below it for questions.
- **The receiving pm turns a request into tickets** in its own repository's space, where only its own roles write, and links them to the request.
- **A pm subscribes to requests addressed to it,** so it learns that a neighbouring repository needs something as soon as its copy of the coordination space syncs. A wake-up through the agent framework is optional, behind a small adapter.

Why: a request is durable, auditable and the same for every framework, while the frameworks' own session-to-session messaging differs and changes fast ([how sessions reach each other](../docs/research/2026-11-14-how-sessions-reach-each-other.md)).

### The daily status

`status` writes one note a day, by default at 06:00 in the time zone the coordination space was set up in. It says what changed since yesterday, what works now, what waits for the owner and what was decided, so the owner reads one note instead of every transcript. The owner sees the status updates in manufakt.io (nxsflow/manufakt-io), the owner's view onto the work, where they are a table of their own; status updates and tickets are its first two views.

Until manufakt.io shows them, the owner asks a pm, which lists the status updates and what waits for the owner, and works through them with it.

## Who it is for

Teams and solo developers who run agents (Claude Code, Codex, Kiro CLI or Kiro Crew, their own) on one or more repositories, and want them to work for days with little supervision, without giving up control over what ships.

- **The owner who directs agents.** Writes the product once, decides in tickets before work starts, and reads one daily note instead of every transcript.
- **The pm agent.** Turns intent into tickets, opens lanes for every role, reviews, merges what the merge gate allows, and writes the status.
- **Every other agent.** Opens a session knowing the rules in force, its ticket and the decisions that touch it.

The first user is NexusFlow itself: nocentraldb and nexus-flow run their factories on it.

## Principles, in priority order

When a requirement, a ticket or a design conflicts with these principles, follow the principles and flag the conflict to pm.

1. **The owner stays in control.** No agent replaces an approval the merge gate reserves for the owner. nexus-flow makes that review small and rare by getting decisions first, never by skipping it.
2. **Decide first, then build.** Questions to the owner go into tickets before the work starts, so the work can run and merge on its own afterwards.
3. **Agents read state locally and fast.** Every read is local; the network is never on the read path. Output is deterministic and machine-readable.
4. **Nothing is lost silently.** Tickets, decisions and notes outlive every session and every branch switch. A superseded decision stays readable, with what replaced it.
5. **Opinionated on what, open on how.** Every factory gets genesis, goals, tickets, decisions, coordination and guardrails. Their shape ships as the smallest standard that runs, and everything beyond it is the team's configuration, which an upgrade respects.
6. **Bring your own agents.** nexus-flow is what agents work from, not an agent framework. It leaves to the framework what the framework does well.
7. **Say plainly what it does not do.** Every guarantee names its limits.

## What it must never do

Each no-go says why, so you can judge cases this list does not name. When a ticket would need one of them, stop and ask me.

- Never let an agent approve, tick or merge on the owner's behalf where the merge gate reserves it for the owner. The whole factory rests on that one human gate.
- Never start work that waits for an owner decision before the decision is in the ticket. Work built on a guess is work the owner must review.
- Never overwrite a team's own GENESIS, FACTORY, CONFIG, merge rules, decision or status template on install or upgrade. The team's way of working is theirs.
- Never lose or silently rewrite a decision. A decision is superseded by a new one, never edited away.
- Never break an existing workspace or its stored data. Format changes are additive or come with a migration, including the move from v1 to NoCentralDB.
- Never put the network on the read path of the CLI. Agents are fast because their state is local.
- Never write secrets, tokens or personal data into tickets, decisions, logs or the status note. Those records are copied and kept. Referring to people is not personal data in this sense: tickets and decisions name members and the owner by their identity, so the record shows who asked, who decided and who did the work. Personal data means anything about a person beyond that identity, such as contact details, private matters, or data about customers and other third parties.
- Never weaken a gate to get green: no skipped test, no unpinned dependency, no `--no-verify`.

## What we trade for what

| We give up | To get | How we soften it |
| --- | --- | --- |
| A web UI or an MCP as the primary surface | Agents that read and write at CLI speed | JSON output by default, `--md` on request; a UI may read the same data |
| Our own agent orchestration | Frameworks that improve faster than we could | Roles, lanes and hand-offs stay ours; sub-agents are theirs |
| Every option a team might want | A factory that runs on day one | Templates and rules the team owns and changes |
| Owner-free merges by default | Control over what ships | Decisions asked for first, a merge gate whose decision is deterministic, one daily note |
| A priority field per ticket | Priorities that move with the team's goals | Goals in `goals` that pm ranks every ticket against |

## Limits

Principle 7 asks every guarantee to name its limits. These are the ones this file relies on:

- **Every local read is as fresh as the last sync.** Reads never wait for the network, so a read can miss what another member wrote since: a ticket another session closed, the owner's decision, a new goal, a request from another repository. Subscriptions report a change once it has reached the local copy, not when it was written.
- **"On main" is as fresh as the last fetch.** `nxs` derives it from the local copy of the base branch and does not fetch on a read.
- **Island mode keeps work going, not the team together.** Without a relay, every copy keeps working and the copies converge when they reconnect. Until then, a session sees none of the others' changes: the owner's decisions, requests between repositories, goals in the coordination space and a member's removal reach it only after the islands rejoin, and the daily note covers only what has arrived. How two members on one network find each other without a relay is NoCentralDB's open question.
- **The merge gate's verdict is a model's judgement.** Only the decision on top of the verdicts is deterministic, and repository rules enforce only what the gate reports as its status check.
- **A wake-up needs an adapter.** A session learns of a change when it next reads or is subscribed; it is woken only where an adapter exists for its agent framework.

## Architecture and stack

| Part | v1 today | v2 |
| --- | --- | --- |
| Entry point | One binary `nxs`, with `nxf`, `nxm` and `nxc` beside it | `nxs` is the only entry point to the library |
| Core | Rust; CI pins rustc 1.99.1 | Unchanged |
| Storage | The local `.nxs/` workspace | NoCentralDB spaces: a ticket space per repository, and a coordination space per group of repositories for goals and requests |
| Agent side | A Node sidecar (`agent-sidecar/`) for the Claude adapter | Small adapters per framework, for wake-ups only |
| Releases | A signed binary through `install.sh`, change fragments, release notes | Installers for macOS, Linux and Windows, macOS first; signing, change fragments and release notes stay (see below) |

**Release management takes nocentraldb as its model** (`docs/release-management.md` in nocentraldb/nocentraldb):

- **Alpha.** Every merge to `main` that changes the product becomes an alpha with its own tag, automatically.
- **Beta.** When the scope of a planned release is met, pm proposes a beta and I decide in a ticket. An agent curates the beta's release notes.
- **Release.** When the promotion conditions hold, pm proposes the release. I curate its release notes, with an agent's support, and tag it.
- **One version.** Everything published shares one version, change fragments decide the next one, and CI builds every published artifact from the tag.

## What has to come back through me

pm decides everything else. Because I and each agent act under identities of our own, a ticket always shows who decided. The following come to me first, asked for in a ticket:

- Anything a user sees first: command names and their output contract, the README, the documentation, the default templates, release notes.
- A change to a stored format, or the migration from v1 to NoCentralDB.
- Anything that cannot be undone: a published package or binary, a release tag, a deleted branch with work on it.
- Money: a new paid service.
- Any change to `.github/CODEOWNERS`, the workflows that release or sign, or the branch rules on `main`.

## Background

How the agent frameworks' sessions reach each other today, and why the ticket carries the message: [docs/research/2026-11-14-how-sessions-reach-each-other.md](../docs/research/2026-11-14-how-sessions-reach-each-other.md).
