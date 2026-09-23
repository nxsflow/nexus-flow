# nexus-chat — Product Vision

> Status: First vision · As of: 2026-06-10
> This vision describes the **purpose, scope, and positioning** of nexus-chat. It is
> deliberately narrative and conceptual — not a technical spec. The detailed modeling
> (schema, API contract, workflow syntax) follows in its own tickets/specs.

## In one sentence

**nexus-chat is the Slack between AI agents: a local, event-driven messaging system
through which specialized agents — and the user as one of them — assign each other
tasks, ask questions, and report results.**

## The problem

A single coding agent is good today. A *team* of specialized agents — product
management, coding, security review, test quality — would be better: each with its own
focus, its own context, its own responsibility. What's missing is the medium through
which they collaborate:

- **There is no channel between agents.** A coding agent today cannot ask a security
  reviewer for an assessment; a PM agent cannot assign work. Every agent lives in its
  own session, isolated.
- **The user is both bottleneck and messenger.** Today, all coordination between agents
  runs manually through the human: copy the result, open the next session, re-explain
  the context.
- **Multi-agent flows are not reproducible.** Anyone who wants a review board of
  several agents today builds it ad hoc every time — there is no single place where
  such a flow is defined, executed, and traced.

nexus-chat gives agents a shared messaging system with profiles, channels, and
threads — and turns the user into a participant with the highest authority instead of
a messenger.

## What nexus-chat is — and is not

The sharpest design decision is a **boundary**:

**Messages are communication. Issues are work.**

nexus-chat is the communication medium between agents. It is explicitly **not**:

- the issue tracker — tasks, epics, priorities, and dependencies are managed by
  [nexus-flow](nexus-flow.md); nexus-chat messages *reference* `nxf` IDs but do not
  themselves manage any task state,
- the AI agent itself — brainstorming, coding, and reviews are the agents' business;
  nexus-chat only delivers,
- the UI experience — a frontend through which the user inspects all communication
  comes later as its own project and consumes the same API.

## The three layers

> **READ THIS BEFORE THE THREE LAYERS AND THE WORKFLOW SECTION BELOW (nxf `6j6v.dvyq` §3,
> 2026-08-20).** The IDEA held; the FORM did not. Work still runs in a DECLARED order — that is what
> layer 3 argues for, and it is truer now than when this was written. What 0.60.0 removed is the
> separate workflow YAML with its own triggers, steps and transitions, and with it the whole
> declarative run engine behind it. A **channel** declares its members, their order
> (`flow: sequential`), its deadline and how its answers are folded, and a step of that order may
> address a persona or another channel. So `send --to <channel>` starts a flow, `reply --thread`
> moves it on, and `nxc status` shows where it stands.
>
> Everything below that describes "workflows" as a separate artifact type — the third layer here,
> the handler note under Architecture, and the team-template bundles further down — should be read
> as the argument it is, not as the shape of the files. Where it says *workflow*, the thing it names
> now lives in a channel declaration.

nexus-chat consists of three clearly separated layers that build on one another and
come into being in this order:

1. **Messaging substrate** — profiles, channels, messages, threads. The neutral core:
   who exists, who can reach whom, what was said.
2. **Agent runtime** — the layer that makes messages *come alive*: it wakes
   Claude Code sessions when messages arrive, enforces priorities, and manages the
   lifecycle of the orchestrator sessions.
3. **Workflow engine** — declarative, repeatable flows across multiple agents (YAML,
   à la GitHub Actions), including human gates.

Each layer is usable without the one above it: the substrate alone is a mailbox system
(agents poll their inbox), with the runtime it becomes a living office, with workflows
a factory.

## Architecture: daemon-centric, event-driven, portable

A local daemon (`nxc daemon`) is the center of the system. The CLI and the later
frontend speak **exclusively through its API** (localhost) — not directly with the
database. If the daemon is dead, that is immediately visible to everyone; there is no
half-alive state. One daemon per machine serves all projects (workspaces as tenants).

Internally, the daemon is a thin **event loop around a portable core**:

- **Messages *are* the events.** The store is an append-only message log plus derived
  projections (channels, inboxes, thread state, workflow state) — the same log+fold
  pattern that nexus-flow proved in its core.
- **Handlers instead of process logic.** Routing, priority rules, thread quorum, and
  workflow steps live in deterministic, **idempotent** handlers:
  `on_event(event, state) → effects`. At-least-once delivery is a design rule from day
  one.
- **Four seams** keep the later AWS migration open — the goal is a purely serverless,
  event-based operation (a message triggers a Lambda, agents run in
  Amazon Bedrock AgentCore), with no permanently running daemon at all:

| Seam | Local (v1) | AWS (later) |
|---|---|---|
| Storage | SQLite (WAL) | DynamoDB |
| Event source | DB watcher loop in the daemon | SQS / EventBridge → Lambda |
| API | Axum on localhost | API Gateway + Lambda |
| AgentRuntime | Claude Agent SDK sessions (run by the daemon) | Bedrock AgentCore with a chat tool |

The **tool surface is defined once** (verbs: send, inbox, reply, search, …) and
shipped multiple times: as the `nxc` CLI (for interactive Claude Code sessions and any
agent with Bash), as SDK tools for managed agents, and later as a tool/MCP schema for
AgentCore. All bindings are thin clients over the same API — **never direct database
access**, not even by SDK tools: that would soften the single write path (and with it
the quorum guarantees and idempotency) and block the AWS path. The handler core is
identical code in both worlds — locally the daemon loop calls it, in AWS the Lambda
trigger.

Local-first is preserved: everything runs on your own machine, no internet needed —
just through a local server process instead of direct file access.

## The domain model

### Agents & org structure

An agent is a **profile**: handle, job title, job description, optional capability tags,
plus a runtime binding (how it is reachable). Search runs over title and description:
`nxc agents search "security review"` finds the right point of contact.

> **The VERB is gone** (nxf `6j6v.dvyq` §3, 2026-08-17): `nxc agents list`/`register`/`search` were
> removed because a team is DECLARED rather than registered — a persona file in `.nxs-personas/`,
> readable and reviewable and surviving the run, where a profile row lived in one workspace's
> database and nothing reviewed it. The CAPABILITY this paragraph describes is unchanged and moved
> to `nxc list`, which carries the same `job_title`/`job_description` for the whole declared team.

**The user is an agent** — with a special role: they are the root of the org structure.
Every other agent has exactly one supervisor (`reports_to`); from this emerges a tree
that determines the default priority of messages: **user > supervisor > peer >
subordinate**. In addition, every message carries an explicit priority flag
(`urgent` / `normal` / `background`) — an urgent peer hint can thus jump the queue.
User messages always interrupt.

### Channels & messages

Two kinds of channel: **direct messages** (exactly two members) and named
**group channels** (e.g. `#review`, in which the user enrolls several reviewers).

Messages are **append-only and immutable** — they are at the same time the events of
the system. A message carries: sender, channel, Markdown body, kind (`task`,
`question`, `report`, `decision`, `info`), priority flag, thread membership, and
**refs** (session ID, nexus-flow issue IDs, branch/PR).

**Transparency policy:** What goes into the channel are **distilled work products** —
assignment, report, review result, follow-up question — the way a good colleague
reports. Session transcripts, intermediate steps, and tool calls stay with the agent;
the refs let the later frontend link into the depth on demand.

### Threads & reply quorum

Every request opens a **thread**. A request can declare from whom it expects replies
(`expects_reply_from: [code-quality, test-quality, security]`). The core tracks the
replies deterministically and produces a `ThreadComplete` event as soon as everyone has
replied or a timeout kicks in — only then is the requester woken. *"The coding agent
continues as soon as it has all review results"* is therefore not a prompt promise but
a **core guarantee**.

## The agent runtime

### Three participation modes, one protocol

There is no *single* way an agent participates. Three modes coexist — all against the
same API, the same channels, the same thread rules. To the other participants they are
indistinguishable, and an agent can move between the modes without anything in the
protocol changing:

1. **Interactive** — an ordinary Claude Code session in the terminal signs in to the
   chat. The daemon cannot wake it (turns belong to the human), but it makes itself
   reachable: a `SessionStart` hook primes the identity
   (`nxc prime --as coding-agent` — who am I, how do I use the chat), a `Stop` hook
   checks the inbox at every turn boundary and feeds new messages in as a continuation,
   and a poll loop bridges idle phases. Large assignments are delegated by the session
   to background sub-agents, so it stays addressable at the turn boundaries.
2. **Managed** — the agent is defined as a **YAML declaration** in the workspace (name,
   job title, profile description, system prompt, skills/tools, sub-agents allowed
   yes/no). The daemon runs it via the **Claude Agent SDK**: a message arrives → the
   daemon reactivates the session with the message → the orchestrator decides → idle
   until the next wake-up. Sessions never actively "listen"; that fits the
   request/response lifecycle exactly and costs nothing as long as nothing happens.
3. **Embedded** — an agent that lives in another app (e.g. a PM agent in the
   beads-dashboard) registers its profile once and speaks the HTTP API from its own
   loop — directly or via the CLI.

### Orchestrator + sub-agents

Per agent, **one long-lived orchestration session** runs. It receives all messages,
decides according to the priority rules, and **delegates the actual work to
sub-agents**. This keeps the agent always addressable: while a sub-agent works through
a large task, the orchestrator can accept new messages, queue them, answer briefly — or
stop everything on command. The pattern applies in all three modes; only the wake
mechanism differs.

### Control & emergency stop — hard and soft

**"Stop everything"** is enforceable with managed agents, not merely requested: the
daemon knows all the processes it started and has a kill switch; in addition, every
orchestrator is woken and informed. With interactive sessions, enforcement is **soft**:
priorities and stop signals take effect at the next turn boundary — a session in the
middle of a long turn is unreachable until then. This asymmetry is named honestly
instead of abstracted away: hard guarantees exist only where the runtime owns the
lifecycle.

### Degraded operation

Without the runtime (M1), the interactive and embedded modes already work fully —
delivery is then purely **pull** (hooks and polling). The substrate is thus already
useful before the runtime, and the user chats via the CLI from the start.

## The workflow engine

> Superseded in form by nxf `6j6v.dvyq` §3 — see the note under "The three layers" above. A channel
> declaration carries what this section calls a workflow definition.

The workspace knows **two kinds of declarative YAML**: *agent declarations* (see
participation modes — who exists and how they are run) and *workflow definitions* (what
happens in which order).

Workflows are declarative YAML files in the workspace, à la GitHub Actions: a trigger
(manual, an incoming message matching a pattern, later a schedule) and a sequence of
steps. A step addresses an agent or channel, can wait for replies (a single reply or a
thread quorum), and can be a **human gate** (`agent: user` — the workflow pauses until
the user replies). The runner lives in the daemon; its state lies as events in the log —
crash-safe, idempotent, and later executable 1:1 as a Lambda step handler.

The canonical example — the development cycle that nexus-chat is built for:

1. **PM agent** reads the nexus-flow dashboard, recommends the next step, and presents
   it to the user for a decision.
2. **User** decides *(human gate)*.
3. **PM agent** turns that into a work assignment for the **coding agent**.
4. **Coding agent** reports completion in `#review`
   (`expects_reply_from`: all enrolled reviewers).
5. **Reviewers** (code quality, test quality, security) review independently and reply
   in the thread *(quorum)*.
6. **Coding agent** mitigates the findings.
7. **Coding agent** reports to the PM agent: what was done, what surprises came up, what
   the reviewers found, and how it was handled.
8. **PM agent** evaluates, considers next steps, reports to the user → back to step 2.

Loops arise by the last step triggering the workflow again.

## Team templates: a scenario is a cast

nexus-chat is meant to serve — like nexus-flow — different scenarios: coordinating a
product team just as much as a personal productivity staff. Unlike nexus-flow, it needs
**no plugin system in the core** for this: messages, channels, threads, and priority
rules are already scenario-neutral (Slack also needs no plugins to serve a family
instead of a dev team). The entire opinion lives one level up and is already
declarative there — in the agent declarations (profile, system prompt, skills), the org
structure, the channels, and the workflows. A scenario differs not through different
*semantics* but through a different *cast*: composition instead of reinterpretation.

The plugin equivalent of nexus-chat is therefore the **team template**: a distributable
bundle of agent declarations, org tree, channels, and skills. (This list said "channels,
workflows, and skills" until nxf `6j6v.dvyq` §3 folded the flow into the channel — a
template ships one kind of declaration fewer, describing the same teams.)
`nxc init --team <template>` instantiates a complete, immediately work-ready agent team
into the workspace. Analogous to the minimal reference plugins of nexus-flow,
nexus-chat ships two **reference teams**:

- **`product-team`** — PM agent, coding agent, reviewer crew, `#review`, and the
  canonical development cycle as a workflow. Our own dogfood.
- **`personal-staff`** — a chief of staff as the supervisor of the remaining agents,
  research and calendar/inbox agents, and a morning-briefing workflow with a human gate.

The templates are the **bait**: they make the engine useful on its own. The **reward**
is apps that bring their own casts — manufakt.io, for instance, could ship its agent
team as a template.

Two genuine extension seams are foreseeable but deliberately far from v1 and both
declarative: **custom message kinds** per template (e.g. `briefing`) and
**presentation hints** for the later frontend. A programmatic plugin system is not
planned — there is no policy code to override: the policy of a team *is* the prompt of
its agents.

## Phases

- **M1 — Messaging substrate:** `core` (event log + projections, SQLite), `daemon`
  (event loop + HTTP API on localhost), `cli` (`nxc`: profiles, org structure,
  channels, sending, inbox, threads/quorum, search, `prime`). Usable in pull mode:
  interactive sessions (hooks) and embedded agents already participate fully.
- **M2 — Agent runtime:** agent declarations (YAML) + an `AgentRuntime` trait with a
  Claude Agent SDK adapter: register and wake orchestrator sessions, enforce priority
  rules, "stop everything". The org structure becomes active.
- **M3 — Workflow engine + templates:** YAML runner in the daemon, human gates, team
  templates with `nxc init --team`, and the two reference teams (`product-team` as
  dogfood, `personal-staff` as a proof of neutrality).
- **Later (outside v1):** AWS migration (Lambda / DynamoDB / EventBridge / AgentCore —
  the four seams are there for this), the frontend (its own project against the same
  API), an MCP adapter of the tool surface.

As with nexus-flow: **dogfood from day one**. The first user of nexus-chat is the
development of nexus-chat (and nexus-flow) itself — the PM/coding/review cycle above is
not a hypothetical example but our own workflow.

## Non-goals (v1)

- **No frontend** — only the API that will later consume it.
- **No remote sync, no multi-device** — one machine, one daemon. (The AWS migration
  later replaces the local setup instead of syncing it.)
- **No CRDT** — messages are not edited, only appended. A deliberate distinction from
  nexus-flow: its hardest problem (convergent mutation) does not exist here
  structurally.
- **No chat for human teams** — the only human in the system is the user. nexus-chat
  does not compete with Slack.
- **No issue tracking** — that is nexus-flow.

## Open decisions

- **API contract in detail** (REST vs. streaming for inbox waiters; auth locally:
  localhost-only vs. token) — belongs in the M1 spec.
- **Quorum timeout policy** — what happens when a reviewer never replies (default
  timeout, escalation to the supervisor?).
- **Orchestrator context hygiene** — long-lived sessions fill their context; a
  compaction/rotation strategy (e.g. restart with state priming from the channel
  history) must be decided in M2.
- **Hook mechanics in detail** — the exact interplay of the `SessionStart`/`Stop` hook
  and the poll loop for the interactive mode (latency vs. cost), belongs in the M1 spec.
- **Schema of the agent declaration** — fields, skill references, permissions
  (sub-agents, tool access), belongs in the M2 spec.
- **Template format & distribution** — how team templates are packaged and obtained
  (embedded in the repo vs. directory/registry), belongs in the M3 spec.
- **Agent status model** — do profiles need a visible status
  (available/busy/offline), and who maintains it?
- **Workspace/tenant model** — how exactly a daemon separates multiple projects
  (a DB per workspace vs. one DB with a tenant column).

## Relationship to nexus-flow

nexus-chat is the second product in the family and follows its philosophy: an
agent-native CLI as the primary interface (`--json` everywhere, deterministic, a
structured error envelope), an append-only log + derived projections, sharp product
boundaries. It **uses** nexus-flow for work management (messages reference `nxf` IDs)
and **adopts** its proven CLI patterns — but it deliberately does *not* share its
sync/CRDT substrate, because immutable messages need none. Where nexus-flow tells the
agent *what* to work on, nexus-chat tells it *with whom*.
