# nexus-flow — Product Vision

> Status: Initial vision · As of: 2026-06-07 · Ref: `beads-dashboard-av4`
>
> **Superseded in parts (2026-09-21).** This vision is kept as written in June. Where it disagrees
> with the [README](../../README.md), the README is current — in three places above all:
>
> - **Positioning.** nexus-flow is the board, the memory and the channel your agents work from, not
>   "an engine for projects and tasks — and nothing else" or "engine-first".
> - **beads.** The sync framing below ("git-style sync (`refs/dolt/data` + JSONL export)") is not
>   how beads works: it syncs with `bd dolt push` / `bd dolt pull` and merges at the cell level. The
>   README's *Prior art* section carries the checked comparison.
> - **Bodies.** The description is a last-writer-wins field like every other scalar; the
>   Yjs-compatible body CRDT (Yrs) described below is not built.
>
> This vision describes the **purpose, scope, and positioning** of nexus-flow. It is
> deliberately narrative and conceptual — not a technical spec. The detailed modeling
> (schema, types, sync protocol) follows in dedicated tickets/specs.

## In one sentence

**nexus-flow is an agent-native, offline-first engine for projects and tasks.**
It gives every AI assistant a reliable memory of *which projects and tasks exist, how they
depend on one another, and what to work on next* — locally instant, and convergently
synchronized once connected.

## The problem

AI assistants — whether coding agents (Claude Code, Copilot) or personal assistants
(Claude Cowork) — are only as good as their overview of the state of the work. They must be
able to answer in seconds: *What is there to do? What is blocked? What do I work on next?*
Today there is no shared foundation for this:

- **Issue trackers** (Jira, GitHub Issues) are server-centric, slow for agents, and not
  designed for machine usability.
- **beads** proved that agent-native tracking works — but it suffers from git-style sync
  (`refs/dolt/data` + JSONL export). That produces a whole class of bugs: **branch-switch drift**
  (closed issues reappear as open), and multi-user / multi-device / web are not cleanly
  solvable.
- **Personal to-do tools** have no dependencies, no derivation of "what's up next," and are
  not deterministically readable by an agent.

nexus-flow eliminates the sync bug class structurally (offline-first, convergent) and makes
**agent ergonomics the bar to clear** — because that is the only reason agent-native tracking
makes a difference at all.

## Positioning: engine-first (but the apps must pull)

nexus-flow is the **hero**. It is a standalone, downloadable engine with which anyone can
teach their assistant project and task management. The consuming products (`manufakt.io`,
`nexflow.it`) are **not mere demos** but independently desirable apps that happen to share the
same engine.

This follows the **open-core pattern** (Supabase / GitLab): a free, open engine as the
adoption engine, plus a hosted experience so good that you don't want to run it yourself.

## What nexus-flow is — and what it is not

The sharpest design decision is a **boundary**:

**nexus-flow is project and task management — and nothing else.**

It is explicitly **not**:

- the AI agent itself,
- brainstorming, coding, or quality gates,
- the UI/product experience.

Those things belong to the consuming apps. `manufakt.io`, for example, is a complete
coding-agent platform (built on Claude Code) with opinionated brainstorming, coding with
memory, and quality gates — within it, nexus-flow is only the **tracking layer** the agent
reasons over. It is precisely this narrow boundary that makes the engine universally reusable.

## The core (the same for all consumers)

The core is *unopinionated*: it defines a data model, derives the state of the work from it,
keeps the data local and in sync, and provides an agent CLI. What is *opinionated* (how
prioritization happens, where data comes from, what it is called and how it looks) is a plugin.

### Data model — universal slots

The core defines **semantic slots**. They apply to every variant; only the *naming* varies
(see the plugin model).

| Slot (core semantics) | Description | Plugin label examples |
|---|---|---|
| Identity | ID (internally ULID/UUID with a replica prefix → collision-free offline; short display form for the eye), type | Project/Epic · Task/Issue |
| Title | Short label | — |
| Description | Prose body | — |
| Completion criterion | when it counts as *done* | Definition of Done / Acceptance Criteria |
| Status | Lifecycle (open, in progress, closed …) | — |
| Priority | Weight / urgency | — |
| Structure | Dependencies (task↔task **and** project↔project), `belongs-to` (1:n), `contributes-to` (n:m) | — |
| Time | Due date; defer/on-hold (a period during which work need not start) | — |
| History | append-only progress/note stream (intermediate states) **+ a single closing comment** | Worklog / Comments / Updates |
| Assignment | Owner / Assignee | — |

Two structural features are deliberately stronger than in classic trackers:

- **Dependencies exist at both levels** — between tasks *and* between projects.
- **`contributes-to` (n:m)** — a task belongs to *one* project but can simultaneously
  contribute to *other* projects. This models real work, where a single step serves multiple
  goals.

### Derivation layer

`ready` / `blocked` / "what's up next" are **deterministic derivations** from the converged
state — never stored state. *Convergence ≠ validity:* after a merge the graph may be invalid
(e.g. dependency cycles from concurrent edits), so the derivation layer includes an
**invariant repair**. This layer is the heart of what makes an agent useful at all: *"show me
the next sensible task."*

### Local store, sync & agent CLI

- **Local store**: a local working copy — fast, readable and writable offline.
- **Sync**: keeps the local copy and the durable server truth convergent (see the offline
  promise).
- **Agent CLI**: the primary interface. `--json` everywhere, deterministic, a `prime`
  equivalent for building context (+ SessionStart hook), `ready` / `blocked` as core
  features, plus `create` / `show` / `update` / `claim` / `close`. The CLI is also a
  **full-fledged editor of the description (body)** — read *and* write, **offline too**
  (see the offline promise). **The bar to clear: at least as usable for agents as `bd`.**
  `bd` is the proven prior art that sets that bar; if the CLI is worse than beads, the engine
  is worthless.

## The plugin model

Plugins override **policy and presentation — never the data model**. The core stays a shared
language; plugins give it a vocabulary and a stance.

- **Vocabulary mapping** — plugins name records and fields for their purpose: the same
  "completion criterion" slot is called *Acceptance Criteria* in an issue tracker and
  *Definition of Done* in a PM tool; "type" becomes *Epic/Issue* or *Project/Task*.
- **Ranking policy** — a coding agent weights "next task" differently (blocker depth, CI
  status) than a knowledge worker (due date, energy, context-switching cost).
- **Integrations** — GitHub/Jira import vs. calendar/email/personal sources.
- **Views & presentation** — board vs. personal day planning.
- **Identity & access** — solo vs. team/multi-tenant.

### Minimal reference plugins (in the open engine)

So that nexus-flow is useful *out of the box*, the open engine repo ships two **minimal
reference plugins** — just enough that any assistant understands the engine immediately:

- an **issue-tracker plugin** → Claude Code and other coding agents see a tracker;
- a **personal-to-do plugin** → Claude Cowork and personal assistants see to-do management.

These plugins are the **bait**: they make the engine useful on its own and drive adoption. It
gets **truly good** only with the opinionated, hosted variants `manufakt.io` and `nexflow.it`
— they are the **reward**.

## Offline promise

**Fully offline-capable.** The internet is never a prerequisite for working on the structure.

- **Structure fields** (title, status, priority, type, dependencies, parent/child, dates) are
  genuinely local-first via a **relational CRDT** — status as an LWW register, dependencies as
  an OR-set (observed-remove), tombstones for deletions. Important: the substrate is
  *maintainable* and **not dependent on a dormant third-party project**. The spike decided: a
  **narrow in-house build** (SQL-native, self-owned); the maintained **Automerge stays as a
  differential-test oracle** to safeguard correctness but does not land in the binary.
  cr-sqlite proved that the approach holds and serves as a reference. Issues
  created/updated offline **converge after reconnect without a manual merge step** — no
  branch-switch drift.
- **The prose body** (description) is a **Yjs document** (TipTap on the web; the server holds
  the durable, authoritative Y.Doc, with Markdown as a materialized projection). Unlike
  originally envisioned in the epic, it is **not a purely server-mediated field**: the **CLI
  can edit the body fully — offline too**. To do this, the CLI itself participates in the body
  CRDT (an embedded Yjs-compatible runtime, e.g. *Yrs*, the Rust port), so that body edits
  created locally **converge like the structure** — concurrent changes (web, desktop, CLI;
  online and offline) merge without loss. The price: the CLI thereby runs a second (CRDT)
  engine alongside the structure path — deliberately accepted, because CLI body editing is a
  must. The CLI edits the body as **Markdown/plaintext**; the visual block editor is provided
  by the web app.

A local replica technically *is* a cache — but one with built-in convergent merge logic. The
server is the durable truth; the local copy is an offline-writable working copy that converges
without manual effort.

## The three products

| Product | Role | Relationship to nexus-flow |
|---|---|---|
| **nexus-flow** | The engine — audience: developers & agents | *is* the engine; its own landing page |
| **manufakt.io** | Complete coding-agent platform (Claude Code based): brainstorming, coding with memory, quality gates | bundles the engine **invisibly** as the tracking layer. `beads-dashboard` is `manufakt.io` in stripped-down form |
| **nexflow.it** | Project management for knowledge workers — personal **and** in a team | bundles the engine **invisibly** as the task/project layer |

Each app ships the engine (including a local version), **without the user needing to notice**,
and carries a **"powered by nexus-flow"** badge (Intel-Inside model). The engine is thereby
both *invisible* (bundled) and *credited* (co-branding). This creates a dual go-to-market:
developers discover the engine directly; end users discover it *through* the apps and the
badge.

## Open core & distribution

| Layer | What | Standing |
|---|---|---|
| Engine (core + CLI + local store) | Solo, local, your agent manages tasks/projects | **Free & open source** — the adoption engine |
| Sync server | from "local only" → "from multiple devices/places" | Open source **and** self-hostable — the hosted variant is the convenient one |
| The apps (`manufakt.io` / `nexflow.it`) | Web board, realtime, team/multi-tenant, curated integrations, the *finished experience* | **Hosted SaaS** — where pull and monetization arise |

The logic: solo + local is free and makes the engine popular. As soon as someone wants
**multi-device, a team, a web board, or realtime**, the most convenient path is the hosted
product. The agent workflow (the same CLI) stays identical everywhere — you simply "grow" into
the app without cutting the engine down.

## Order / phases

1. **Engine core + CLI against the coding-agent tracker** (`manufakt.io` / off beads). This is
   the most demanding consumer and is **dogfooded by us ourselves**: we currently use beads to
   build nexus-flow — the engine's first consumer migrates us off exactly that beads. The
   engine eats its own tail, and we feel our own gaps immediately.
2. **`nexflow.it` as the second consumer** — proves that the core is genuinely
   product-agnostic (knowledge worker instead of coding).

Content building blocks along the way (from `beads-dashboard-av4`): sync substrate spike
(ElectricSQL vs. cr-sqlite) + validate the ID strategy → data model + invariants + derivation
layer → CLI MVP with agent parity → sync protocol + server (durable store, relay, realtime
push, Yjs doc service) → web UI → migration off beads (JSONL import, round-trip) →
auth/multi-tenant.

## Non-goals (v1)

- No full feature parity with beads (Formulas / Molecules / Federation).
- No rich-text/block-editor parity in the CLI — the CLI edits the body as
  Markdown/plaintext; the visual block editor is the web app's job.
- nexus-flow is **not** the agent, not brainstorming, not quality gates, not the UI
  experience — those are the apps.

## Open decisions

- **Detailed modeling** of the data model (schema, types, exact status/priority values) —
  deliberately kept out of this vision; it belongs in the spec.
- Additional dimensions that *are candidates* but are not yet fixed in the core picture:
  labels/tags, effort/estimate values, recurring tasks.
- **Validation of the sync substrate**: ElectricSQL had API churn — test hard in the first
  spike, otherwise cr-sqlite.
- **Body CRDT in the CLI**: embedded Yrs runtime (real char-level merge, larger binary) vs. a
  simplified patch/replace-op fallback — validate in the substrate spike (Yrs maturity, binary
  size, merge correctness against the web Y.Doc).

## Relationship to `beads-dashboard-av4`

The epic `beads-dashboard-av4` originally framed the goal as *"replace beads with a product."*
This vision **reframes it**: nexus-flow is the **engine**; moving off beads is its **first
consumer**, not the end product. The architecture decisions made in the epic (split substrate
of relational CRDT + Yjs, durable server truth, hybrid JSONL export, agent ergonomics as the
bar to clear) remain valid and are translated into product language here. **One deliberate
deviation**: body editing is also possible via the CLI and offline (the CLI participates in
the body CRDT), rather than being purely server-mediated — so "fully offline-capable" is
delivered for the description too.
