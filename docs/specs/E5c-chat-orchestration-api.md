# E5c — Chat Orchestration API: the role runtime as a library surface (spec)

> Status: **Implemented** · Designed 2026-07-31 · Implemented 2026-08-01 · nxf `6j6v.qvfp`
> Prerequisite: the v0.33.0 role runtime (`6j6v.*`), the messaging facade + `chat::Engine` (`be9y`),
> the transcript read (`6wt2.5bym`).
> Sibling of `docs/specs/E5-embedding-host-api.md` (flow) and `docs/specs/E5m-memory-embedding-api.md`
> (memory). Those two raised an *existing* surface to a contract; this one **creates** the surface
> chat is missing, then holds it to the same discipline.
>
> **Reading this document after implementation.** The body below is the design as it was approved,
> kept in the design tense on purpose — it is the argument, and rewriting it into past tense would
> lose why each decision was made. What the code actually shipped is recorded in **`> Shipped:`**
> callouts throughout, wherever it differs from the paragraph above them, and every section is
> mapped to its implementing ticket in **§9**. One thing the design describes is **not** true of the
> code and says so where it appears.
>
> **SUPERSEDED ON THE WHOLE WORKFLOW HALF (nxf `6j6v.dvyq` §3, 2026-08-20).** §1's scope opens with
> "workflow `start`/`step done`/`tick`/`status`", and that half of the surface no longer exists —
> neither on `Engine`, nor on the CLI, nor as a `workflow_runs` record, nor as a declaration. A
> CHANNEL is the flow now: it declares its own members, their order (`flow: sequential`), its
> deadline and its consolidation, so `send --to <channel>` starts one and `reply --thread` moves it
> on. Where an operation stands is `nxc status` / `Engine::status` (nxf `6j6v.a71h`). §3.5's
> step-liveness clock went with the runs it watched, as a named loss. §4's table below carries the
> per-method record. Everything about the ROLE-TRIGGER and MESSAGING halves stands.
>
> **SUPERSEDED ON THE WRITING HALF (nxf `6j6v.ckeq`, 2026-08-21).** §4's table opens "flat on
> `Engine`, beside `send`/`reply`/`inbox`". There is no `Engine::send` and no `Engine::reply` any
> more: the writing seam carries exactly two verbs, `send_to` and `reply_thread`. Owner, verb by
> verb at the source: *"`send_to` und `reply_thread` sollen der einzige Weg der Kommunikation sein.
> Es startet immer auch die jeweilige Sitzung bzw. weckt sie wieder auf."* — so there is no way left
> to lay a message down that nothing picks up. §4.1's whole argument (that `Engine::reply` must gain
> the routing it silently dropped) SHIPPED and then travelled with the method: `reply_thread` calls
> the same `orchestration::reply`. `send`'s raw post is a NAMED LOSS with no successor — `send_to`
> takes a DECLARED target and always starts or wakes something — and `reply`'s message-addressed form
> is a named loss too (an app resolves the message's thread first, one read).
>
> The same item cut the ELEVEN caller options to three per verb: `send_to` takes `to`, `body`, `refs`
> and `reply_thread` takes `thread`, `body` and one escalation bit. `model` and `deadline` are
> DECLARED at the persona and the channel; `kind`, `priority` and `disposition` are gone as caller
> options with the owner's note *"interessante Features, die wir zu einem spaeteren Zeitpunkt bei
> Bedarf zurueckholen"*; `if_unanswered` left the seam for `surface::settle_if_unanswered`, the
> sidecar teardown's own door. `refs` is the one thing ADDED: a first message says what it is about,
> or says `--no-ref`, and omitting both warns in the `--json` envelope. The per-row record is
> `crates/chat/tests/seam_disposition.rs`.
>
> **SUPERSEDED ON THE READING HALF (nxf `6j6v.yr59`, 2026-08-21).** The same §4 opener names
> `inbox`, and the reading seam is now SEVEN verbs: `status`, `directory`, `prime_as`, ONE
> conversation reader (`thread`), `transcript_page`, `subscribe`, and `search`. Sixteen read verbs
> stood on `Engine`; ten went. Three were pure overloads (`prime` = `prime_as(c, None, now)`,
> `transcript` = `transcript_page(s, -1, None)`, and `open_with_poll_interval`, a third constructor
> for one value that is `EngineConfig::poll_interval` now). The rest were decided one by one with
> the owner: `public_channels` folded into `directory`, which names the declared personas, the
> declared channels WITH THEIR PURPOSE and the workspace's public front doors; `threads`,
> `thread_board` and `thread_quorums` folded into `status`, which grew `deadline` and the
> working-tree fields per thread and a `StatusScope::Threads` for an explicit SET (the anti-N+1
> property is measured by `crates/chat/tests/bulk_quorum.rs`, not asserted in prose); `inbox` and
> `opener_wake` folded into `prime_as`, which already carried both; `channels` and `messages` are
> NAMED LOSSES — per-channel unread counts and a channel's flat history in one call have no
> successor on the seam. `Engine::thread` gained the channel's declared `visibility`, which
> `thread_board` used to carry, so the cut does not widen what a non-requester may read. The CLI is
> untouched: every verb that reached one of these bodies still reaches it. The per-row record is
> `crates/chat/tests/seam_disposition.rs`; the SIZE of the surface is held by
> `crates/chat/tests/read_surface.rs` — which since nxf `6j6v.dh41` holds a second rule BESIDE that
> one, about the surface's shape rather than its size: every fact this seam records or reports must
> be readable back through it, and where it asks a surface to branch, answerable through it.
>
> **`search` IS THE SEVENTH, BY AN OWNER CORRECTION OF THE SAME DAY.** The item covers it nowhere —
> not the keep table, not the "what goes and why" list, not the five verbs its DoD sends to be
> decided individually — so the build decided it in the item's own terms, removed `Engine::search`
> as an eleventh named loss, and reported the gap. The owner overruled that: app-foundations'
> `ChatClient` (`packages/engine-client/src/chat-client.ts`) consumes `search` today — one of the 14
> methods of that interface that still exist on this seam, not one of the 14 that no longer do — and
> deleting a live consumed method inside the item whose purpose is to cost that consumer ONE
> migration instead of two is the break §5 warns about. The count in the item's own title is what
> went stale; the argument for the other six is untouched. One residual was named rather than
> implied: `search` was membership-scoped and applied no declared `visibility`, unlike `thread`
> beside it. **That residual is closed (nxf `6j6v.px98`, 2026-09-05):** `facade::search` applies
> the channel's declared `visibility` through the same `filter_board_messages` semantics `thread`
> uses, resolved from the same `channels.yaml` catalogue, and `nxc search` reads through the facade
> rather than straight at the store so the two surfaces cannot drift apart again. The rule the fix
> follows is the owner's of 2026-08-23: `visibility` is an ACCESS rule, not a display rule. The
> same question for the TRANSCRIPT reads is nxf `6j6v.yxsa` and is still open.
>
> **SUPERSEDED ON THE RECEIPT SHAPE AND THE ADVANCE VOCABULARY (nxf `6j6v.dvyq` §3 + `6j6v.ckeq` +
> `6j6v.rs9k`).** Added by the PR #347 review, which found this prose stale two lines from an edit
> this branch itself made — the "shape 2" case `CLAUDE.md` says the vocabulary sweep cannot see.
> Four passages below still describe a receipt and a run engine that are gone, and they are LEFT AS
> WRITTEN because each is a dated record of what shipped that day; read them against this note.
>
> `ReplyReceipt` is `{ posted, message_id, thread_id, resumed, woke, wake_skipped, completed,
> warnings }`. **There is no `advance_failed` field**, on this receipt or any other: it reported a
> failure to advance a declarative RUN, and §3's block c+d removed the run record, `workflow_tick`
> and `WorkflowTickReceipt` with it. `AdvanceFailed { run, detail }` named that run and went the
> same way. `workflow_status(run, session)`, cited nearby as the read that "deliberately takes none
> of" the caller context, no longer exists either.
>
> What answers those questions now: an ordered channel IS the run (`flow: sequential`), `tick`
> reports through `TickReceipt`, and a step that could not be started — or one whose deadline struck
> while the flow moved past it (`6j6v.rs9k`) — is reported as a `FailedConsequence` in `warnings`,
> the shape `6j6v.93zd` made carry ALL of what did not happen rather than only the first.
>
> `Engine::reply`, named in the §4.1 passage as the method that runs `orchestration::reply`, is
> itself gone (`6j6v.ckeq`): `Engine::reply_thread` is the one reply form on the seam, and it calls
> that same body with the thread as its target. The `nxc reply --json` divergence recorded in the
> parity section stands, minus its `advance_failed` half.
>
> **Follow-up round, 2026-08-02 (nxf `6j6v.5x9j` / `6j6v.d9cb` / `6j6v.ep7j`).** The two items §9.2
> listed as the honest remainder are now closed, and the worker seam gained the operational half it
> was missing: §3.3 carries the escape hatch as it actually shipped — reshaped by the Bedrock
> AgentCore probe so a REMOTE agent runtime can dock onto it — §3.5 is new (the step-liveness clock),
> and §9.2 is empty.

## 1. Goal & scope

The v0.33.0 role runtime — workflow `start`/`step done`/`tick`/`status`, role trigger/summon,
declared-channel open + fan-out, `ask` (quorum open) — is reachable **only** through the `nxc` CLI.
app-foundations links nexus-chat **as a library** (`engine-bridge`/`engine-server`, not the CLI), so
over that seam it can today read messages and quorum boards but **cannot start a workflow, trigger a
role, or open a channel**. And role/channel/workflow *definitions* are read only from the on-disk
`roles/` folder, so an app embedding the engine would have to write a definition folder into every
user's project — the agents would live in the user's repo, not in the app.

E5c closes all three:

1. **Lift the orchestration verbs onto `chat::Engine`** with the same receipt / typed-view discipline
   as the messaging facade.
2. **Accept caller-supplied definitions** in lieu of the `roles/` folder read, injected at
   workspace-open and replaceable at runtime.
3. **Plumb per-role/step/call model selection** (`fable`|`opus`|`sonnet`), which exists nowhere in
   the spawn path today — a definition naming a model is inert until it does.

**Hard scope guardrail (owner, 2026-07-30, recorded as a note on `qvfp`).** This seam delivers
**exclusively** the single-repo role runtime — roles, channels, workflows, quorum and app-supplied
definitions **within one** workspace, plus transcript read. **No** cross-repo routing, **no**
cross-workspace bridge, **no** repo-spanning WAIT resolution. Cross-repo communication is the
manufakt.io USP and stays there (`dqem.c5em`). This boundary is not negotiable and not "just for
now": what becomes surface here goes irreversibly public with the open-source launch.

## 2. Why the verbs cannot be lifted as they stand

The blocker is not visibility, it is **ambient process state**. Every orchestration verb in `cli.rs`
resolves its inputs from the environment:

| What the verb needs | How `cli.rs` gets it | Why a library cannot |
| --- | --- | --- |
| acting identity | `actor()` → `NXC_ACTOR`/`USER`/`"nxc"` | a host serves many actors from one process |
| minting workspace | `origin(db)` → `NXC_ORIGIN`, else the workspace's replica prefix | host-owned, not process-owned |
| wall clock | `resolve_now()` → `NXC_NOW`, else system clock | seam rule: the host injects `now` |
| caller session | `session()` → `NXC_SESSION` | there is no ambient session in an app |
| depth guard | `hop()` → `NXC_HOP` | per-chain state, not per-process |
| definitions | `roles_dir()` → `<workspace>/../roles` | the whole point of part 2 |
| worker | `select_worker()` → `NXC_WORKER`/`NXC_SIDECAR` | the host owns the sidecar path |

> **Three of these rows stopped being the CALLER's business on 2026-08-21 (`6j6v.07me`)**, without
> any of them going back to being ambient: the minting workspace comes off the HANDLE, the depth
> guard off the SESSION MAP, and the wall clock defaults to the system clock at the ADAPTER while the
> verbs still take it injected. The acting identity follows the session where there is one. The
> column that changes is "how a HOST supplies it", never "does the library read the environment" —
> `cli.rs` still resolves every `NXC_*` above and passes what it resolves.
>
> The minting workspace row also changed its VALUE in the same move (owner, 2026-08-21). It read
> `NXC_ORIGIN`, else the constant `"local"`; the constant is gone and the fallback is the
> workspace's own replica prefix, which is also what `Engine::origin()` answers. Both seams had to
> move together or a declared channel would carry two disjoint member sets — an app's
> `<prefix>/coder` and a terminal's `local/coder`. That is why `origin()` in `cli.rs` now takes the
> `--db` selector: it is a question about a WORKSPACE, and the six `--consumer`-defaulting verbs
> pass the one they already opened their store over.

So the move is **extraction, not duplication**: the verb bodies move into a new module that takes all
of this explicitly, and `cli.rs` becomes the adapter that fills it from the environment. One
implementation, two callers — the pattern the messaging facade already set, and the reason
`tests/parity.rs` can pin the two seams against each other at all. Copying the verbs onto `Engine`
instead would create a second implementation of the depth guard, the preflight order, the qualified-
vs-bare handle split, and the completion routing — every one of which has already cost a fix round.

Secondary benefit: `crates/chat/src/cli.rs` is 4048 lines and holds most of this logic. Extraction
moves the decision logic out and leaves clap parsing + rendering behind.

> **Shipped.** `cli.rs` is 2518 lines and `orchestration.rs` is 2401; what remains in `cli.rs` for
> every orchestration verb is clap parsing, the `CliCtx` build and rendering. The CLI copies were
> deleted, not left dead — `open_declared_channel_and_fan_out`, `resolve_declared_channel`,
> `ensure_declared_channel`, `bare_handle_of`, `select_workflow` and `dm_channel_id` are gone from
> `cli.rs` rather than duplicated (`rws4`, `wyhx`).

### 2.1 The depth guard is the one row that could not stay ambient (nxf `6j6v.m48m`)

The table above reads `hop()` → `NXC_HOP` as an input to *resolve*, like the clock or the actor. It is
not the same kind of thing. The other rows describe **who is calling**; this one is a **circuit
breaker on what the caller is allowed to do**, and every commissioned role can run a shell command
(the shipped roles declare `tools: [Bash]`; since nxf `6j6v.kffm` one that declares nothing is
granted it, because it is ordered to answer with `nxc reply`) — so the process the guard bounds owned
the number the guard read. `unset NXC_HOP` before a role's own
`nxc send` reset the chain on every hop. Extraction made that worse rather than better:
`Ctx.hop`/`Caller.hop` is a plain argument, so a library caller reaches the same effect by passing
`0` each time, with no environment involved at all. Since each hop is a real detached process plus a
paid LLM call, the guard is a cost brake, not a formality.

The counter therefore moved into the store. `session_map` carries a `depth` column, written by
whoever **spawns** a session (`trigger_role`, plus the ephemeral synthesizer's own spawn site) with
the same value it stamps into that session's `NXC_HOP`. `orchestration::resolve_hop` — what every
verb that can spawn runs first — reads the depth of the caller's OWN session back out and takes
`max(persisted, ctx.hop)`: the claimed value survives as a **floor**, so a host tracking its own
chain still has its count honoured, but nothing can use it to go down. The write is monotonic for the
same reason (`MAX`, not assignment): a shallower later caller must not hand a session a fresh budget.

Both seams keep their field — this is additive, and `Ctx`/`Caller` are unchanged. (`Caller` lost its
`hop` on 2026-08-21, `6j6v.07me`: once the persisted depth wins and a claim can only ever RAISE it, a
seam whose callers work through sessions has nothing to claim. `Ctx.hop` stays, and the CLI still
fills it from `NXC_HOP` — a spawned `nxc` process is HANDED its depth and has no other way to be
told, which is the asymmetry this whole section is about.) What is left open,
and deliberately: a caller presenting **no session at all** still resolves to its claim, because that
is exactly the shape of a genuine human kickoff. A role can therefore still discard its whole ambient
identity to look like one, at the cost of everything that identity buys it (no return address, no
resume, no ambient run for `workflow step done`). Closing that needs an unforgeable credential handed
to the spawned process rather than an environment variable — the access-control design (nxf
`6j6v.6aza`), not this guard.

#### 2.1.1 What the counter counts: the open chain, not every hop (nxf `6j6v.ka09`)

Anchoring the counter in the store fixed *whose* number it is. It left a second question unasked:
**which hops it counts.** Every trigger was one hop deeper, and `record_trigger_depth` never lowers —
so a wake that carried a RESULT back up the chain still deepened the session it returned to. For a
one-shot fan-out that is invisible. For a long-lived role it is terminal:

    user(0) → O:1 → PM:2 → worker:3
    worker answers, waking PM (hop 3) → PM = MAX(2,4) = 4
    PM answers, waking O      (hop 4) → O  = MAX(1,5) = 5
    round 2: O(5) → PM:6 → worker:7 → PM:8 → O:9

Four hops a round, and after about seven the orchestrator is over the cap on **every** verb, for
good: `MAX` does not fall, and a session handover replaces the RUNTIME session, not the internal one
the depth hangs on. That loop is not an edge case — it *is* the orchestrator design (nxf `41j0.me7d`,
found while building it there as `41j0.km2y`).

So `RoleSpawn` carries `chain: ChainMove`, and every spawn site names which way it runs:

- **`Deeper`** — the target is handed NEW work: `send --to` (a persona summon or a channel
  fan-out), a workflow step, a liveness nudge, the ephemeral synthesizer, and `role_resume` — which
  since `6j6v.dvyq` §3 no surface reaches. Lands one past the caller, monotonic as before.
- **`Unwind`** — the target is handed the RESULT of work *it* commissioned: a reply routed by its own
  return address, a completed board's wake to the requester that opened it. The link closes, so the
  target re-enters at the depth it already had. Never lower than that either — the write stays `MAX`,
  so an old message of one's own getting answered is not a way back down.

The counter therefore measures the **open** chain — commissioned and not yet answered — and
`trigger_role` derives the persisted depth and the `NXC_HOP` stamp from one value, because
`resolve_hop` takes the larger of the two and two independently computed numbers would let the higher
silently win.

**What this gives up, stated plainly.** A pure descent past the cap still trips, and so do two roles
that only ever push work at each other and never answer. What is no longer bounded is an endless
alternation of *completed* request/answer pairs. That shape is structurally identical to the
legitimate dialogue above — they differ only in whether they stop — so no depth counter could ever
have had both, and the version that had it was the reported bug. Bounding a conversation by volume is
a different instrument from bounding a chain by depth; nxf `6j6v.w1tx` carries the open question.

The guard is a **cost brake**, so it is worth naming what still holds that axis. Nothing in the
engine does. What does, in practice, is the runtime underneath: a remote one reaps a session it has
not heard from (AgentCore at 15 minutes idle or 8 hours total — the case `TriggerError::SessionGone`
exists for), so a runaway conversation dies with its sessions. That is a property of a particular
runtime, not a guarantee this crate makes — a `Worker` whose sessions never expire has nothing
bounding this axis, which is exactly what `6j6v.w1tx` has to decide about.

### 2.2 The workflow block follows the run, not the workspace (nxf `6j6v.1wgd`)

The same shape, one row over: `trigger_role` appends a per-role block of step reminders to the
composed system prompt, and it resolved that block from a per-WORKSPACE value — the legacy
`roles/workflow.yaml`, and after §3.2's catalogue landed, whatever `Definitions::prompt_workflow`
nominates. Named workflows (nxf `6j6v.d543`) made that wrong rather than merely incomplete: roles are
reused across workflows, so a role fired under `hotfix` was handed `build-and-ship`'s steps in the
same prompt as `hotfix`'s own kickoff message. An LLM role acts on the whole prompt — it can wait for
a step that will never fire, or read its `--outcome` off the wrong transition table. Host-supplied
definitions sharpen it further: nothing is nominated at all unless the caller says so, so a fix that
still resolved a *file* would do nothing for an embedding app.

`TriggerWorkflow` names where the block comes from, per trigger:

- **`Run(run_id)`** — the workflow-driven sites (`fire_role_step`, reached from `workflow start`'s
  step[0] and every `advance_and_fire` step; `fire_channel_step`'s fan-out). By run **id** rather
  than by `WorkflowDecl`, so the lookup lives in one place and no call site can hand over a workflow
  that is not the run's.
- **`Ambient`** — caller-initiated triggers (`send --to`, `reply`'s return-address
  resume, a completion wake): the run bound to the session being **triggered**, else the run bound to
  the **caller's** own session, else the nominated team workflow. A role summoned by a role that is
  mid-run is being pulled into that run's work. `Ambient` cannot stand in for `Run`, because at a
  step's fresh mint the run→session binding is emitted *after* the trigger.

A run whose declared workflow is no longer in the catalogue — an app can swap it at runtime, a
folder-backed workspace can lose a file — composes **no** block rather than falling back to the
nominated one. An absent block is a gap; the wrong block is the defect this closes.

The channel fan-out reaches this through a private `channel_open_in` rather than a `workflow` field
on `ChannelOpenRequest`: that struct is app-facing and constructed by literal on both seams, so
growing it would break every existing caller for a value no app can supply.

## 3. Architecture

### 3.1 New module `crates/chat/src/orchestration.rs`

Holds the extracted verb bodies. Every one takes an explicit context:

```rust
/// Everything the orchestration verbs need that the CLI reads from the environment.
/// The seam adds no new semantics; the host injects now/actor/origin/session/hop.
pub struct Ctx<'a> {
    pub now: &'a str,
    pub origin: &'a str,
    pub actor: &'a str,
    /// The ambient caller session, stamped into `refs.session_id` when the caller
    /// didn't set one. `None` (not a placeholder) when there is no real session.
    pub session: Option<&'a str>,
    /// Current (pre-increment) depth-guard counter; `trigger` advances it for the next hop.
    pub hop: u32,
    pub defs: &'a Definitions,
    pub worker: &'a dyn Worker,
    /// Stamped into the spawned role's `NXC_DB` so its own in-session `nxc` calls resolve.
    pub db_path: &'a str,
    /// The project's `CLAUDE.md`, if the role's `claude_md` policy wants it. Host-supplied:
    /// the CLI reads `<workspace>/../CLAUDE.md`, an app supplies its own or `None`.
    pub project_claude_md: Option<&'a str>,
}
```

`Ctx` carries no `Path`, no `Option<&str> db`, and reads no environment variable. Everything that
today calls `roles_dir(db)?` mid-verb resolves against `ctx.defs` instead — which also removes the
repeated filesystem walks a single `workflow step done` performs today.

> **Shipped as tabled, and with a companion in the same module**: `Caller { now, origin, actor,
> session, hop }` — the five values a HOST owns, which is what the `Engine` seam takes as its one
> ambient parameter (§4). `Caller::into_ctx(defs, worker, db_path, project_claude_md)` completes it
> with the four an ADAPTER resolves, so the split between "the caller's half" and "the adapter's
> half" of a `Ctx` is written down once, next to `Ctx` itself. `cli.rs::CliCtx` is unchanged: it owns
> its values (it resolves them from the environment into `String`s) and fills a `Ctx` directly.
>
> **CUT TO `Caller { session, actor, now }` on 2026-08-21 (`6j6v.07me`).** `Ctx` above is untouched —
> it is still the nine values a verb needs, `hop` included — but four of the five a CALLER handed
> over turned out to be derivable once `send --to`/`reply --thread` became the only interaction
> verbs. `origin` comes from the handle (`Engine::origin`; one handle is one workspace, and it is
> what access rights will hang off, `6j6v.6aza`); `hop` is claimed by nobody, because `resolve_hop`
> has read the authoritative depth out of `session_map` since `6j6v.m48m` and a claim could only ever
> raise it; `actor` is read back from that same session map, since identity comes from PROVENANCE
> rather than assertion (design 2026-08-05 §4.1) and is required only where there is NO session; and
> `now` became an override with the system clock as its default. What stays is `session` — the
> SENDER's, stamped as the return address (`refs.session_id`) a `reply --thread` wakes. The mapping
> is still written once: `Caller::into_ctx(ambient, defs, worker, timer, db_path, project_claude_md)`,
> where `Ambient { now, origin, actor }` is what the adapter resolved. `cli.rs` is again unchanged
> and still fills a `Ctx` from `NXC_*`, which is what keeps the goldens deterministic.

### 3.2 Definitions

```rust
pub struct Definitions { /* roles, channels, workflows */ }

impl Definitions {
    /// What the CLI does today: scan `<dir>/*.yaml` + `channels.yaml` + `workflow.yaml`
    /// + `workflows/*.yaml`.
    pub fn from_roles_dir(dir: &Path) -> Result<Definitions>;
    /// Host-supplied. Validates before returning (§3.2.1).
    pub fn new(roles: Vec<RoleDecl>, channels: Vec<ChannelDecl>, workflows: Vec<WorkflowDecl>)
        -> Result<Definitions>;

    pub fn roles(&self) -> &[RoleDecl];
    pub fn channels(&self) -> &[ChannelDecl];
    pub fn workflows(&self) -> &[WorkflowDecl];

    /// Replaces `cli.rs::resolve_role` — same error kinds (`validation` for a malformed
    /// handle, `not_found` for a well-formed handle naming no declared role).
    pub fn role(&self, handle: &str) -> Result<&RoleDecl>;
    pub fn channel(&self, name: &str) -> Option<&ChannelDecl>;
    /// Replaces `cli.rs::select_workflow` verbatim, including the 0/1/N + `--name` rules.
    pub fn workflow(&self, name: Option<&str>) -> Result<&WorkflowDecl>;
}
```

> **Shipped** (`crates/chat/src/definitions.rs`, nxf `h2fr` + `p3vn`) with three additions the
> design did not anticipate:
>
> - **`declared_channel(&self, name) -> Result<Option<&ChannelDecl>>`** beside `channel()`. Both
>   exist because the existing code genuinely wants both: `workflow tick` *reports* on a board that
>   already exists (`channel`), every verb about to *act* re-checks fail-closed
>   (`declared_channel`, which applies `validate_channel_for_use`).
> - **`prompt_workflow()` / `with_prompt_workflow(name)`** — the notion of a *team* workflow, the
>   one whose per-role block is folded into a triggered role's system prompt. Needed because the
>   naive translation would have changed behaviour: `trigger_role` has always composed its block
>   from the legacy `roles/workflow.yaml` alone, never the named `workflows/*.yaml` set, so folding
>   in all of them would silently give a role appearing in two workflows two blocks.
>   `from_roles_dir` nominates the legacy file; a caller-supplied catalogue nominates its own or
>   none, and "none" produces a prompt byte-identical to one composed with no workflows at all.
> - **`role()` resolves by the declared `handle` field**, where `cli.rs::resolve_role` read
>   `<roles>/<handle>.yaml` by *filename*. This is a deliberate behaviour change: on a mismatch the
>   old path bound the session to the requested handle while the spawned session acted as
>   `decl.handle`. Two files declaring one handle are now a loud load error instead of a silent
>   winner at trigger time.

#### 3.2.1 Validation, and where it stays

`Definitions::new` runs the **structural** checks that used to be implicit in the filesystem layout,
and fails loudly at injection rather than at first use:

- role handle format — non-empty, no `/`, no `\`, no `..` (today `valid_role_handle`; the
  path-traversal reason disappears with host-supplied definitions, but the handle still becomes half
  of a qualified `origin/handle` chat identity, so the `/` rejection stays load-bearing);
- reserved handles `__synth__` / `__delivered__` rejected (`channel::SYNTHESIS_HANDLE` /
  `DELIVERED_HANDLE`);
- no duplicate role handles, channel names, or workflow names — ambiguity the directory layout
  previously made impossible.

**Referential integrity does not move.** `channel::validate_channels` (a channel naming a role that
isn't declared) stays advisory in `nxs prime`, and `validate_channel_for_use` stays fail-closed at
the point of use. Making it a hard error at injection would break the deliberate `prime` behaviour
of *reporting* a broken team roster instead of refusing to start.

#### 3.2.2 Injection and replacement

```rust
pub struct EngineConfig {
    pub definitions: DefinitionSource,   // FromRolesDir | Supplied(Definitions)
    pub worker: WorkerConfig,            // Disabled | Dry | Sidecar { sidecar, cwd }
}

impl Engine {
    pub fn open(db: Option<&str>, start: &Path) -> Result<Engine>;             // unchanged
    pub fn open_with(db: Option<&str>, start: &Path, cfg: EngineConfig) -> Result<Engine>;
    pub fn set_definitions(&self, defs: Definitions) -> Result<()>;
    pub fn definitions(&self) -> Result<Definitions>;
}
```

> **SUPERSEDED ON THE INJECTION PATH (nxf `6j6v.dvyq` step 6, 2026-08-17).** §3.2.2 documents an API
> that **no longer exists.** `DefinitionSource` (and with it `Supplied`), `EngineConfig::definitions`
> and `Engine::set_definitions` are all REMOVED. `EngineConfig` now carries `worker` and `timer` and
> nothing else, and declarations come from the workspace's `.nxs-personas/` folder for an embedding
> app exactly as for the CLI. The reason is an owner decision of 2026-08-14, obtained specifically
> for this removal: personas and channels are not codified in the app, and even a later in-app role
> editor writes into that folder rather than into an app-owned database — so the injection path had
> no use case left. app-foundations named its six call sites and released it; the teardown there is
> their work.
>
> **`Engine::definitions()` — the READ — STAYS**, and is held there by a gate rather than by care
> (`crates/chat/tests/seam_disposition.rs` names it with its reason): the app still reads the
> catalogue to render roles and channels, it merely no longer supplies it. Grabbing read and write
> together during a cleanup is the exact failure §5 of `6j6v.dvyq` warns about.
>
> The property `set_definitions` existed for is unchanged and now costs nothing to state: an edit is
> visible to the NEXT verb on the handle and on every existing clone of it, with no reopen and no
> torn-down `subscribe` receiver — because nothing is cached. The catalogue is resolved per call,
> BEFORE the store lock is taken. The `RwLock` and the two-lock discipline described in the
> "Correction" note below are therefore gone with it; what replaces them is a filesystem read that
> must still happen outside the store lock, pinned by
> `embed_orchestration.rs::a_folder_edit_races_an_in_flight_verb_without_deadlocking_or_losing_a_write`.
>
> Everything below in §3.2.2 that names `DefinitionSource`, `Supplied`, `FromRolesDir` or
> `set_definitions` — including the code block above and the "Correction" note — is HISTORY, kept as
> the record of why the seam was shaped that way. Read it as superseded, not as the contract.

> **Superseded on the LOCATION (nxf `6j6v.dvyq` step 4, 2026-08-17).** The declaration folder is
> `<workspace-root>/.nxs-personas/`, not `<workspace-root>/roles/`. Every `roles/` in this section
> should be read as that path. A legacy `roles/` folder is still READ when `.nxs-personas/` carries
> nothing, and the fact that it was is REPORTED on the catalogue itself
> (`Definitions::source() -> Option<&DeclarationSource>`, carrying the path a declaration belongs at,
> the source kind, any legacy folder found, and the count). Nothing writes or moves a user's folder:
> an application that has only opened someone else's project must not silently rewrite it.
>
> `Definitions::from_roles_dir` is now `Definitions::from_dir` (one named folder) beside
> `Definitions::resolve` (a workspace root, with the resolution above).

> **Correction (the design wrote `definitions() -> Definitions`).** The shipped signature is
> `-> Result<Definitions>` and the `Result` is load-bearing, not defensive noise: with
> `DefinitionSource::FromRolesDir` the accessor *loads* the catalogue off disk on the call, and a
> malformed or unreadable `roles/` folder really can fail. Returning a bare `Definitions` would have
> forced either an empty catalogue or a panic on that path. The spec was wrong; the code is right.
>
> **Shipped, and worth stating because it is not obvious from the signature:**
> `DefinitionSource::FromRolesDir` is a *source*, not a snapshot — it is re-read on every verb that
> needs declarations, exactly as a fresh `nxc` process re-reads the folder per invocation. A
> long-lived handle over a folder-declared workspace therefore sees an edit on disk without being
> reopened. `project_claude_md` is read per call for the same reason. Definitions live behind their
> own `RwLock` and **neither lock is ever held while the other is acquired**: every verb snapshots
> the catalogue, releases, then takes the store lock — which rules out a deadlock, rules out a
> `set_definitions` writer stuck behind a long store write, and gives each verb *one* coherent
> catalogue even if `set_definitions` lands mid-call.
>
> `WorkerConfig::Disabled` is realised as a `DisabledWorker` that refuses at **trigger** time rather
> than as an eager failure at `open_with`. Failing the open would break the existing read-only
> embedder; failing at `Ctx` construction would break `reply`/`tick`, which need no worker.
> Deferring it means a spawning verb returns the named `validation` error, while a `reply` that
> cannot wake — its direct 1:1 resume or a completing board's requester — still posts and reports
> `woke: None` plus a `wake_skipped` naming the refusal: it surfaces inside `Worker::trigger`, where
> the wake's "skip, don't fail" contract is implemented (§4.1).

`set_definitions` exists because the realistic desktop case is a user editing an agent in
manufakt.io: the next trigger must see the change without the app tearing down its handle, its
`subscribe` receiver, and its UI state. The handle keeps definitions behind the same lock discipline
as the store, so it stays `Clone + Send + Sync`.

`Engine::open` stays exactly as it is — `DefinitionSource::FromRolesDir` and
`WorkerConfig::Disabled`. Existing embedders keep working; a `Disabled` worker means the reads and
the messaging writes behave identically, and any verb that would spawn a session returns a
`validation` error naming the missing configuration rather than silently doing nothing. Orchestration
is an explicit opt-in, not something a caller gets by accident.

### 3.3 Worker

`Worker` becomes a genuinely long-lived seam. Today `select_worker(cwd, env)` builds a fresh worker
per trigger because the per-hop env (`NXC_ORIGIN`/`NXC_DB`/`NXC_HOP`) is baked into the struct. That
env moves onto the request:

```rust
pub trait Worker: Send + Sync {          // + Send + Sync so Engine stays Sync
    fn trigger(&self, req: TriggerRequest) -> Result<()>;
}

pub struct TriggerRequest {
    pub internal_session: String,
    pub resume_real: Option<String>,
    pub message: String,
    pub role: RoleSpec,
    pub env: Vec<(String, String)>,      // NEW — was baked into SidecarWorker
}
```

`SidecarWorker { sidecar, cwd }` is then constructible once and held for the handle's lifetime.
`WorkerConfig::from_env(cwd)` reproduces today's `NXC_WORKER`/`NXC_SIDECAR` reading for the CLI, so
the env dependency survives in exactly one place, at the CLI adapter. A host that wants to own
process spawning entirely (its own logs, its own kill switch) implements `Worker` itself — the
escape hatch costs nothing because the trait already exists.

> **Shipped as `WorkerConfig::from_ambient(cwd, ambient)`**, not `from_env(cwd)`: the ambient lookup
> is *injected* (`impl Fn(&str) -> Option<String>`) rather than read via `std::env::var`, so the
> selection rules are pure and unit-testable without mutating process-global env, which parallel
> tests in one binary would otherwise race on. `from_ambient` deliberately never yields `Disabled` —
> "no worker" is a library-side choice, not something an unset variable should silently produce —
> and `Disabled.build()` is a named `validation` error rather than a silent no-op.
>
> **Shipped as `WorkerConfig::Custom(Arc<dyn Worker>)`** (nxf `6j6v.5x9j`, 2026-08-02) — but
> **not** in the shape the paragraph above imagines. The blocker was never the derives: `Debug` and
> `PartialEq` are hand-written now (`Custom(..)`; `Arc::ptr_eq`, which is reflexive, so `Eq` stays
> honest), and `WorkerConfig` gained `#[non_exhaustive]` in the same cut so every later variant is
> additive. The blocker was the SHAPE of the trait, and that is what a live probe answered.
>
> **The probe** (app-foundations `41j0.9t68`, a real run against Bedrock AgentCore, closed
> 2026-08-01) asked what a SECOND, remote agent runtime needs in order to dock onto this seam.
> Verbatim from its finding: *as "give me your `std::process::Command`" AgentCore cannot dock.*
> Three requirements, and how each landed:
>
> 1. **Async.** Provisioning and teardown are a network call, not a local process start. What did
>    not carry was the `Result<()>` RETURN — it forced the answer to be known before `trigger`
>    returned, so a remote host could only block a round-trip on whatever thread the engine happens
>    to be on (inside an async runtime `block_on` panics) or lie about an outcome it does not have.
>    So `trigger` returns `TriggerOutcome`: `Started { runtime_session }` when the id is already in
>    hand, `Accepted` when it is not. **Submission is synchronous, completion is out of band** —
>    which is what `SidecarWorker` has always done (spawn detached, let the session bind itself later
>    through `nxc session bind`). A remote host hands its provisioning call to its own executor,
>    returns `Accepted`, and calls the new `Engine::bind_runtime_session` when the call resolves.
>    Nothing blocks and no `.await` appears on the path.
>
>    An `async fn trigger` was NOT taken. It would make every orchestration verb async and through
>    them every `Engine` verb — the whole surface app-foundations links — for a seam that is
>    fire-and-forget by contract anyway (§5.1). That is a coordinated break across three projects,
>    not this ticket's to take unilaterally.
> 2. **A repeatable error class.** Cloud sessions die SILENTLY (AgentCore: 15 minutes idle, 8 hours
>    maximum), so "resume what is no longer there" is routine and calls for a different response than
>    any other failure. `Worker::trigger`'s error side is now `TriggerError`
>    (`#[non_exhaustive]`): `SessionGone { session, detail }` and `Failed(NxfError)`. It converts
>    into the shared envelope as `not_found`, so the many call sites with nothing to decide still use
>    a plain `?`; the ones that DO decide match on it. All four wake sites classify it as the new
>    `WakeSkipReason::SessionGone` rather than `TriggerFailed`, and §3.5's liveness escalation
>    branches on it directly.
> 3. **An opaque session id instead of a process handle.** `TriggerOutcome::Started` carries the
>    runtime's own identifier verbatim; `orchestration::trigger_and_bind` — the one place both spawn
>    paths funnel through — binds it to the internal session and never parses it. There is no pid and
>    no termination signal anywhere on the seam.
>
> **Teardown is deliberately absent.** The probe names provisioning *and* teardown as network calls,
> and there is no `reap`/`stop` method because the engine has no call site that would use one:
> nothing in the runtime ends a session, sessions end themselves. A trait method with no caller would
> be a guess at a shape, which is exactly what this ticket was told not to make.
>
> The engine keeps shipping a default arm (`Sidecar`) so it is usable on its own; apps bring their
> own through `Custom`. That stays inside the IP boundary: the engine defines **what** a role is and
> **when** it runs, never **who** executes it or where.
>
> **The seam runs inside the handle's lock, so the contract is enforced, not merely written down**
> (PR #269 review, Code Quality #1 / Integrity #1). `Engine` holds one mutex over its store for a
> whole orchestration verb and the trigger happens inside it — fine for the workers this crate
> ships, and a very different proposition once the code in there is a third party's. A `Custom`
> worker that blocked on its provisioning call would have frozen every verb on every clone of the
> handle, forever, with nothing to observe it; one that called `bind_runtime_session` from inside
> `trigger` would have self-deadlocked on a non-reentrant mutex.
>
> `Worker` therefore states two rules — return promptly, and never re-enter the `Engine` from inside
> `trigger` (return `Started` instead; that is what the variant is for) — and `WorkerConfig::build`
> wraps a `Custom` worker so a breach is BOUNDED: it runs on its own thread and the engine stops
> waiting after `CUSTOM_WORKER_BOUND` (30s, a breach detector rather than a budget — a conforming
> worker returns in microseconds). The mutex is released, the deadlock becomes a delay, a panic in
> host code stays in host code instead of poisoning the mutex on its way out, and the timeout error
> says plainly that whether the session started is unknown. This is `timer.rs`'s `wait_bounded`
> argument at the other external seam, and it is here for the same reason that one exists.

The `env_clear()` allowlist in `SidecarWorker::trigger` (`PATH`, `HOME`, `USER`, `NXC_SIDECAR`,
`NXC_WORKER`) is unchanged BY THIS DOCUMENT. It is a security decision with a written rationale, and
nothing here widens it.

> **WIDENED ONCE SINCE, BY ONE KEY (nxf `6j6v.zq0t`, 2026-08-22).** `USER` was added, and the list
> above already names it. Until then this sentence enumerated four keys and every spawned persona
> session on macOS died at authentication: the Agent SDK's ambient credential is PLATFORM-DEPENDENT
> and lives in the login keyring there, not under `HOME`, and reaching the keyring resolves through
> `USER`. The rationale for the widening lives where the allowlist does (`worker.rs`), including the
> `env -i` bisection that pinned it to that one variable. Recorded here rather than silently
> corrected because "unchanged … nothing here widens it" is exactly the kind of standing claim a
> reader trusts without re-deriving — the failure mode nxf `6j6v.mg5b`, in this same change set, is
> about.

### 3.4 Model selection

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Model { Fable, Opus, Sonnet }

impl Model {
    /// The Claude Agent SDK `options.model` string.
    pub fn sdk_id(self) -> &'static str { /* claude-fable-5 | claude-opus-5 | claude-sonnet-5 */ }
}
```

- `RoleDecl.model: Option<Model>` and `WorkflowStep.model: Option<Model>`, both `#[serde(default)]`
  and skipped when `None`, so every existing YAML file round-trips byte-identically.
- **Precedence: call › step › role.** A per-call override on the request types lets an app offer a
  model picker for one task without rewriting the definition; a step override lets a workflow run a
  cheap reviewer and an expensive implementer; the role's own value is the floor.
- `RoleSpec.model: Option<Model>` carries the resolved choice to the worker, and the spec JSON gains
  `"model": "<sdk id>"` — **absent when `None`**, mirroring the `tools` discipline exactly (an
  omitted key leaves `options.model` unset and the SDK's own default applies; the `Option` is what
  makes "no choice declared" and "a choice" distinguishable, the same bug `tools` already fixed).
- `agent-sidecar/src/main.mjs`: `if (spec.model) options.model = spec.model;`
  > **Shipped as `resolveModelOption(spec.model)`** in `agent-sidecar/src/spec-helpers.mjs`,
  > mirroring `resolveToolsOption` beside it — a pure helper, so it is covered by the `node --test`
  > suite that deliberately runs from a bare checkout with no SDK installed. Absent or `null` (what
  > the Rust side writes for a role that declared no model) leaves `options.model` unset; a
  > malformed value degrades to the default rather than failing the session with an opaque model
  > error.
- The alias→id mapping lives in the engine, not the sidecar: `Model::sdk_id` is one table in one
  place, and the sidecar stays a dumb executor of the spec.
- `#[non_exhaustive]` so adding a model later is an additive minor, not a break for downstream
  `match`.

> **Shipped** as `crate::role::Model` (beside `RoleDecl`, which is where a declaration's types
> live), re-exported through `orchestration`. The precedence rule is one function,
> `orchestration::resolve_model(call, step, role) -> call.or(step).or(role)`, so it cannot drift
> apart across call sites. The ephemeral synthesizer passes `model: None`: it has no declaration
> file and therefore no declared model, so it runs on the SDK default like any other role that
> states no preference.

~~`nxc send --model <fable|opus|sonnet>` is added on the CLI too.~~ **REMOVED 2026-08-15
(`6j6v.dvyq` §3, owner: "vielleicht nehmen wir es spaeter wieder rein"; pulled forward by
`6j6v.e9qj` under the standing owner directive of 2026-08-14).** The flag was the per-call level of
`resolve_model`'s call > step > role precedence, and it was added because it was the only way to
exercise the spawn path against a chosen model without an app in the loop. `6j6v.e9qj` moves "which
model" onto the DECLARATION — a channel's `summary_model:` beside a role's `model:`/`stage:` — so a
per-call override and a declared value became two answers to one question, and the board had already
chosen the declaration.

**The SEAM is unchanged** (`6j6v.dvyq` §5): `Commission` (named `RoleTriggerRequest` until
`6j6v.ntp9`)/`RoleResumeRequest`/`ChannelOpenRequest`/`SendToRequest` all still carry `model`, and
`resolve_model`'s three levels are untouched. What is gone is the way an AGENT typed it; an
embedding app still chooses per call.

### 3.5 The step-liveness clock (nxf `6j6v.d9cb`, added 2026-08-02)

Not part of the original design — added in the follow-up round because it is the OPERATIONAL half of
§3.3's requirement, and because the v3 self-build run made it concrete. A triggered step's session
went idle and silently stalled the whole workflow three separate times: each an SDK session blocked
forever on a backgrounded gate command (`cargo test --all`) that had died with no completion signal,
each un-stuck by a human sending "restart it / resume in the foreground" by hand. A stalled session
cannot un-stall itself; only something outside it can poke it. `nxc` is the orchestrator of ephemeral
role sessions now, so it is that something.

**Declaration** — two optional fields on `WorkflowStep`, both `#[serde(default, skip_serializing_if)]`
so every existing workflow file round-trips byte-identically:

```yaml
- id: implement
  target: role:coder
  instructions: …
  liveness: 20m      # how long this step's session may go QUIET before it is woken
  max_nudges: 2      # unanswered pokes before the step is escalated (default 2, hard cap 5)
```

`liveness` uses the same `<n><unit>`-or-RFC3339 grammar as `--deadline` and a channel's `timeout`
(one parser, `facade::resolve_deadline_spec`, parameterised only by the argument name it reports;
`facade::resolve_instant` is the wrapper over it that keeps only the resolved instant, which is what
`liveness` and `--deadline` need — nxf 6j6v.nf38 added the other half, the declared WINDOW, for the
per-member channel timeout). Absent
is the default and arms nothing at all: a step whose target legitimately thinks for an hour must not
be poked by a guess. It is only meaningful on a role target — a channel step's board is already
watched by the channel's own `timeout` + `workflow tick --thread`, and a fan-out has no single
session to watch — which `validate_workflow` gains a fail-closed check for, alongside the grammar.

**State** — `workflow_step_liveness`, one row per RUN (a run has exactly one `current_step`), a
**plain device-local table** for the same reason `session_map` and `agent_transcript` are ones: it
describes a live session on THIS machine, a sync peer could do nothing with it and would race this
replica to poke a session it cannot reach. The run's own progress stays in the shared op log.

The nudge is **claimed** rather than merely recorded (PR #269 review, Code Quality #6): a
compare-and-swap on the window's own deadline, taken BEFORE the trigger. Two checks running at once
— a scheduled one-shot job firing while an app's own timer runs the verb by hand — would otherwise
both read the same quiet window and both poke, spending two paid triggers and two of the budget on
one stall. The loser reports the ordinary `not_due` no-op, having triggered nothing.

**Both declaration paths validate it.** `validate_workflow` is reached only by `nxc workflow
validate` and `prime`, i.e. only for declarations that came off disk; a host app supplies its own
through `Definitions::new`, which never reached that check, so a malformed `liveness` used to arm
nothing at all and say so only through a breadcrumb no app reads (PR #269 review, Integrity #2).
The two intrinsic checks — does the window parse, is it on a role target — are now their own
function (`validate_step_liveness`) that both paths run, and they sit on the right side of
`Definitions::new`'s documented line: neither looks outside the step, so neither is the referential
integrity that deliberately stays advisory.

**What counts as alive: the transcript high-water mark** (`MAX(seq)` over the session's
`agent_transcript` rows), compared against the mark taken when the window was armed. That is the
literal signal from the build run — "the process was alive but its transcript was frozen for ~40
min" — and it needs no cooperation from the role and no trustworthy clock inside the session.

**The clock is the timer this codebase already has**: a one-shot `nxc workflow liveness --run <id>`
beside `tick`'s `nxc workflow tick --thread <id>`. Still no daemon; each check that finds the step
alive but unfinished arms the next one itself. Keying on the run rather than a thread is the mirror
image of `tick`'s choice, not a deviation from it: a liveness window belongs to a run's current step,
which is precisely the context a board lacks.

**And the timer became a value, like the worker** (PR #269 review, Code Quality #3). `orchestration`
used to call `timer::select_timer()` from inside these verbs, which read `NXC_TIMER` from process env
— against §3's own "the seam reads no environment variable" rule — and left an embedding app unable
to decline an `at` subprocess it pays for, in its own process, from inside the handle's locked
section, for up to eight seconds. `TimerConfig { Disabled, Dry, At }` mirrors `WorkerConfig` exactly,
including `from_ambient` for the CLI adapter, and `Ctx` carries a `&dyn Timer` beside its `&dyn
Worker`. `Engine`'s default is `Disabled` — and unlike a disabled *worker* that is not a refusal:
both verbs a job would have run are idempotent and callable by hand, so declining the automation
breaks nothing, and an app that wants OS-level jobs asks for `At` outright.

**The verb** — `orchestration::workflow_liveness`, on both seams (`nxc workflow liveness [--run]`,
`Engine::workflow_liveness`), reporting a `LivenessVerdict`: `not_armed`, `not_running`, `not_due`,
`progressed`, `nudged`, `nudge_failed`, `escalated`. Idempotent in the way that matters — a check
that finds nothing armed, nothing due, or a run that already moved on changes nothing and says so.

- **Nudge**: resume the step's session with a synthesized message that says both halves of what the
  human said by hand — resume, and re-run gating commands in the FOREGROUND rather than backgrounded.
- **Bound**: `nudges` counts CONSECUTIVE unanswered pokes (a session that answers one gets its full
  budget back — a slow session is not a dead one). A nudge the worker could not DELIVER still counts,
  or an unreachable worker would loop forever instead of reaching the escalation.
- **Escalate**: out of budget — or, straight away, when §3.3's `TriggerError::SessionGone` says the
  runtime session is not there to poke — route through a declared `when: stalled` transition if the
  workflow has one, else fail the run. Never `default`: `default` means "this step finished", and a
  stalled step did not.

Consumers stay oblivious, the same principle a channel's `timeout` follows: the engine owns the
clock, the nudge and the budget; a workflow author declares at most a threshold and a budget, and a
role never learns any of this exists — a nudge arrives as an ordinary resume message.

## 4. The Engine surface

Flat on `Engine`, beside `send`/`reply`/`inbox` — one handle for app-foundations to reach through
the bridge, and it mirrors how the messaging half already reads. (`send` and `reply` themselves were
removed on 2026-08-21 by nxf `6j6v.ckeq`; see the supersession note at the head of this document.)

| Verb | Shape | Notes |
| --- | --- | --- |
| ~~`ask`~~ | `AskRequest → AskReceipt` | **REMOVED from `Engine` 2026-08-19 (`6j6v.dvyq` §3)**, with the CLI verb and with `orchestration::ask` — the router that chose between a declared channel and a raw one. With every channel declared there is nothing to route between. `facade::ask` (the WRITE) stays and is what the fan-out is built out of. |
| ~~`channel_open`~~ | `ChannelOpenRequest → AskReceipt` | **REMOVED from `Engine` 2026-08-19 (`6j6v.dvyq` §3):** it was a second door onto one mechanism. `Engine::send_to` reaches it when its target names a declared channel, and `orchestration::open_declared_channel_and_fan_out` — the body — is untouched. `ChannelOpenRequest` also lost its `expect` field, the per-call override that only `ask --expect` could set. |
| ~~`role_trigger`~~ | `Commission → TriggerReceipt` | **REMOVED from `Engine` 2026-08-19 (`6j6v.dvyq` §3)**, with its CLI half `nxc send --role`. It COLLAPSES into `Engine::send_to`, whose persona branch runs the same body — `orchestration::coordinator_commission`, named `role_trigger_with` until `6j6v.ntp9` — which since that item also opens the thread and tells the persona which one to answer into, without which a reply has no address (`reply --thread` being the one reply form left). Those two steps were `send_to`'s own until `6j6v.ntp9` moved them under the coordinator; the body is otherwise untouched. |
| ~~`role_resume`~~ | `RoleResumeRequest → TriggerReceipt` | **REMOVED from `Engine` 2026-08-19 (`6j6v.dvyq` §3)**, with its CLI half `nxc send --session`, and — alone among the removals — **with no successor**. §3 writes "`send --session` → `reply --thread`", but a resume COMMISSIONS a live session (`Deeper`, registering no obligation) while a reply ANSWERS one (`Unwind`, through the thread's return address), so a caller holding a session id and no thread has no path left. Recorded as a named LOSS by owner decision; the residual case — delivering into a session mid-turn — is `6j6v.xr3z`'s `reply --thread <id> --force`. `orchestration::role_resume` stays as a body no surface drives. |
| ~~`workflow_start`~~ | `WorkflowStartRequest → WorkflowStartReceipt` | **REMOVED from `Engine` 2026-08-20 (`6j6v.dvyq` §3)**, with its CLI half and with the `workflow_runs` record itself. A declared channel carries its own members, order, deadline and consolidation, so `Engine::send_to` on a channel target IS starting a run of it. |
| ~~`workflow_step_done`~~ | `WorkflowStepDoneRequest → WorkflowAdvanceReceipt` | **REMOVED 2026-08-20 (`6j6v.dvyq` §3).** A step does not announce its own completion: the channel's supervisor decides what comes next when the member's `reply` lands, so `Engine::reply_thread` is what moves a flow on. The `--outcome` token it carried has no successor and needs none — `MessageKind::Escalation` is the one bit that was ever load-bearing, and it is DECLARED. |
| ~~`workflow_tick`~~ | `WorkflowTickRequest → WorkflowTickReceipt` | **REMOVED from `Engine` 2026-08-20 (`6j6v.dvyq` §3)** by §5's closed list. The VERB survives as `nxc tick --thread`, hidden from `--help`: the one-shot `at` job a declared `timeout` schedules is what invokes it, and an embedding app does not drive a board's deadline by hand. Recorded as a `KnownGap` in `verb_seam.rs`. |
| ~~`workflow_status`~~ | `run: Option<&str>, session: Option<&str> → WorkflowRunView` | **REMOVED 2026-08-20 (`6j6v.dvyq` §3), and it is the one removal §5's closed list did NOT sanction** — decided by the owner on that day, on a construction rather than a preference: with `workflow_start` gone from both surfaces nothing can write a `workflow_runs` row again, so this read renders a table that can never be filled. `Engine::status` is where an operation stands. |
| ~~`workflow_list`~~ | `WorkflowListRequest → Vec<WorkflowRunSummaryView>` | **REMOVED 2026-08-20**, same decision and same reason. It was the read that needed no run id and no session, which is exactly what `Engine::status` with no scope is. |
| ~~`workflow_events`~~ / ~~`subscribe_workflow`~~ | `after: i64 → (Vec<WorkflowEvent>, i64)` / `→ Receiver<WorkflowEvent>` | **REMOVED 2026-08-20 (`6j6v.dvyq` acceptance point 4)**, together with `workflow_event_cursor`. Discharged by removal rather than by the rebuild the acceptance sketches, on two findings: no consumer (manufakt.io has no call site; app-foundations carries only pass-through) and a replacement already on the seam (`Engine::subscribe` for the tick, `Engine::status` for the read). The residual — a coalesced tick does not say WHICH thread moved — is named in `seam_disposition.rs`. |
| `prime` → `prime_as` | `consumer: &str, persona: Option<&str>, now: &str → PrimeReport` | the SessionStart bootstrap (added by nxf `6j6v.r5a2`, epic `6j6v.fjrc`) — see below. **`prime` itself was REMOVED on 2026-08-21 (`6j6v.yr59`)** as the pure overload it was: its body was `prime_as(consumer, None, now)`, and `None` is the human at the keyboard. `prime_as` also absorbed `inbox` and `opener_wake`, both of which it already carried. |
| ~~`set_definitions`~~ / `definitions` | — | §3.2.2 |
| | | ~~`set_definitions`~~ was REMOVED on 2026-08-17 (`6j6v.dvyq` step 6) with `DefinitionSource`/`EngineConfig::definitions`; declarations come from `.nxs-personas/` for an app as for the CLI. `definitions` — the READ — stays, and a gate holds it there (`seam_disposition.rs`). |

Receipts are `serde::Serialize` structs with a declared field order, like `SendReceipt`/`AskReceipt`
— the JSON contract is the field order, and the CLI's `--json` output is a render of the same struct.

> **Shipped exactly as tabled, with the ambient values as ONE named-field parameter.** Every verb
> except `workflow_status` reads `fn verb(&self, caller: Caller<'_>, req: …)`, where
> `orchestration::Caller { now, origin, actor, session, hop }` is what a library caller owns and the
> CLI reads from `NXC_*`. (`Caller` carries `{ session, actor, now }` since `6j6v.07me` — see the
> note in §3.1 for which of the five left and where each is answered instead. The argument below is
> untouched by that and is why the two that remain are still NAMED: `session` and `actor` are both
> `Option<&str>`, so a positional pair would swap in silence exactly as `origin`/`actor` did.) `workflow_status(run, session)` deliberately takes none of it: it is a pure
> read that needs no clock, no worker and no catalogue. `reply` (§4.1) takes the same `Caller` and
> returns `ReplyReceipt { resumed, woke, wake_skipped, completed, advance_failed, warnings }`.
>
> **The five values were positional first, and that is the thing the owner changed** (decision of
> 2026-08-01, before the surface had a single consumer). They shipped as
> `fn verb(&self, now: &str, origin: &str, actor: &str, session: Option<&str>, hop: u32, req: …)` —
> an order fixed enough that it had to be written down in a comment, and four `&str`-shaped values
> side by side, so a transposition still compiled and produced a message attributed to the wrong
> actor or minted under the wrong `origin`. `origin` is precisely what the auth package will hang
> access rights off, so that is not a cosmetic defect. `Caller` is deliberately the *caller half of
> `Ctx`*, borrowed exactly as `Ctx` is (nothing is cloned on the way through, at the cost of one
> borrow per field for a caller holding owned `String`s across a bridge) and `Copy` for the host that
> varies one field: `Caller { session: Some(&s), ..base }`. It lives beside `Ctx` in
> `orchestration.rs`, and `Caller::into_ctx(defs, worker, db_path, project_claude_md)` is the single
> place the caller's half and the adapter's half are joined — so `Engine` builds its `Ctx` FROM the
> caller object instead of restating a five-field mapping per verb. Behaviour is untouched, which is
> what the parity differential (§5) proves on both seams.
>
> `WorkflowAdvanceReceipt.next_step` shipped as a `String`, not the `Option` an earlier draft
> implied: a terminal advance still *names* a step (the `done`/`fail` sentinel), so there is no
> absent case to model.
>
> **The "CLI `--json` is a render of the same struct" clause only became true during this epic.**
> `cli.rs` rebuilt `workflow start`/`step done`/`tick`/`status` into `serde_json::json!` objects — a
> `BTreeMap` without `preserve_order` — and so emitted alphabetically-sorted keys while the library
> seam serialized the shared type in declared order. The parity differential (`8cs7`) found it. It
> is fixed at the root: all four verbs now serialize the shared type. Same keys, same values, same
> determinism — only the key *order* changed, which is a user-visible output change and is in the
> changelog as one.

### 4.0 `prime` — the SessionStart bootstrap (nxf `6j6v.r5a2`, added 2026-08-03)

`nxc prime` was the last verb that assembled anything in `cli.rs`, and the worst offender: it read
`store.inbox`/`store.opener_wake` **past this facade**, split the catch-up by disposition itself,
called `validate_declared_team` itself, and decided with `is_spawned_context()` whether the roster's
errors were shown. None of it was reachable from the library, so app-foundations rebuilt the whole
assembly in TypeScript (Foundation v0.35.0) — the drift the seam invariant exists to prevent, shipped.

`facade::prime(store, consumer, now, roles_dir) → PrimeReport` now carries the block as data — the
context-recovery hint, the coordination rule, the command reference, the catch-up split by
disposition, the requester wake, the declared-team roster with its errors — and renders itself
(`render_markdown` for the SessionStart block, `to_value` for `--json`). Every part comes from this
layer's own seams (`inbox`, `opener_wake`, `validate_declared_team`); the direct store reads are gone.

**`is_spawned_context` stays a rendering decision.** Both render methods take
`declaration_errors: bool` rather than resolving it: an embedding host has no terminal notion of
"interactive", and a facade that decided this itself would silently serve an app less than it serves
the CLI. The CLI answers it from its own env, as it always did.

**The Markdown `render_markdown` produces is a contract**: it IS the block `nxs prime` fans out to
and a host injects verbatim. `crates/chat/tests/prime_golden.rs` pins it section by section, and the
parity differential binds it to the CLI's output.

### 4.1 `reply` gains its real behaviour (a fix, not just a move)

**`Engine::reply` is incomplete today and this is the ticket that has to say so.** The CLI's `reply`
does three things beyond posting the message: it wakes the requester role when a quorum board
completes, it routes declared-channel completion (`on_channel_complete` — synthesizer or pass-through
delivery), and through that it can advance a workflow run. `Engine::reply` calls `facade::reply` and
does none of it. An app that posts a reply through the handle therefore leaves boards that never
complete and workflow runs that never advance — silently, with a successful receipt.

`reply` moves into `orchestration` with the other verbs, and `Engine::reply` gains the full
behaviour. This is a **behaviour change for existing embedders** and gets its own regression test
(reply through the handle completes a board and wakes the opener, byte-identically to the CLI).

`Engine::send` stays the raw post it is: its `SendRequest` takes an explicit channel id, so there is
no declared-channel ambiguity to resolve — `channel_open` is the declared path. Documented on both.

> **BOTH METHODS ARE GONE (nxf `6j6v.ckeq`, 2026-08-21).** The fix this section argues for shipped
> and then moved: `Engine::reply_thread` runs the same `orchestration::reply`, so the requester wake
> and the `on_complete` routing are exactly where this section put them. `Engine::send`'s raw post
> went with the owner's rule that every write must start or wake a session — a message nothing picks
> up is the one thing the surface no longer offers. `facade::send`/`facade::reply` and their request
> types are untouched: they are the WRITES the whole crate is built out of. See the head note.

> **Shipped** (`7dw6` moved the chain, `d49b` put it on the handle). `Engine::reply` now runs
> `orchestration::reply` — the requester wake, the declared channel's `on_complete` routing
> (synthesizer spawn / pass-through delivery), and the workflow advance that routing can drive — and
> returns `ReplyReceipt { resumed, woke, wake_skipped, completed, advance_failed, warnings }` so the caller can
> see which of those happened instead of guessing. Regression-tested in
> `crates/chat/tests/embed_orchestration.rs`; reverting `reply` to the old body fails three of those
> tests.
>
> **The completion contract, after the owner's decision on nxf `6j6v.bxdd` (2026-08-01).**
> *"Skip, don't fail"* now holds on **both** completion paths: a wake target that no longer resolves,
> a worker that refuses the spawn, a workflow that fails to advance are each a stderr breadcrumb and
> an `Ok`, never a failed `reply`. It used to hold only on the **declared-channel** path (where
> `wake_role_requester` catches) — `route_completion`'s non-declared fallback arm propagated its
> trigger failure with `?`, so a completing reply into a plain channel whose opener could not be
> spawned returned `Err` with the reply *already persisted*: an error for a message that was in fact
> written. That was pre-existing and identical on both seams (`cli.rs::reply` propagated at exactly
> this point too), which is why the extraction carried it verbatim rather than quietly "correcting"
> it, and why two independent implementation passes (`7dw6`, `d49b`) each rediscovered it.
>
> The owner rejected both extremes — an `Err` lies about a persisted message, a bare `Ok` swallows
> the fact that the next role was never woken — so **"skip" now means *visibly* skipped**: every wake
> that is attempted and does not land is reported as `ReplyReceipt.wake_skipped`
> (`WakeSkipped { session, reason: unresolved | trigger_failed, detail? }`), leaving `woke: None` on
> its own to mean "there was nothing to wake". Both seams render it — it is the CLI's
> `nxc reply --json` `wake_skipped` key as well — because the change removes the only
> machine-readable failure signal that path had (a nonzero exit), and a stderr breadcrumb alone
> serves the human at the terminal, not an app whose UI never sees this process's stderr.
>
> **And the same one level up: the direct 1:1 resume** (nxf `6j6v.0akf`, owner decision 2026-08-01).
> `bxdd` decided the *completion* wake and deliberately left `reply`'s step (3) — the direct
> return-address resume, by far the most commonly hit of the four wake sites — still propagating its
> trigger failure with `?`, since changing it is its own user-visible change to `nxc reply`. It was
> then decided in its own right, before the release rather than after: nothing on this seam has
> shipped, so the same change later would be a behaviour break on a delivered surface. Step (3) now
> catches **both** of its failure points exactly as the completion wakes do — a role that is no
> longer declared (or a session-map lookup that fails) is `unresolved`, a spawn the worker refuses is
> `trigger_failed` — reporting the same `WakeSkipped` in `ReplyReceipt.wake_skipped` and in
> `nxc reply --json`.
>
> The ordinary case is untouched and must stay distinguishable: a target with no return address, or
> one that is not a registered role session, is a plain human/ad-hoc post that was never owed a wake,
> and it stays `resumed: false`, `woke: None`, `wake_skipped` absent. That distinction is the whole
> justification for the change — `resumed: false` + `woke: None` is ambiguous on its own, so without
> the finding, returning `Ok` here would be a *silent* failure instead of a *visibly skipped* one.
>
> **`nxc reply` therefore exits 0 where it used to exit nonzero on that path**, which is in the
> changelog as the user-visible change it is. No compatibility switch: a toggle preserving the old
> error path would leave four sites with two possible contracts, and one uniform contract across all
> four is the entire purpose. There is now no exception left to document.
>
> **Carried to the other seam of the same routing, and to its second finding** (branch review of
> PR #265, Integrity & Robustness #1 and #2). `workflow_tick` runs the *same*
> `route_declared_channel_completion` as `reply` and threw its results away: a tick reported
> `acted: true` while the requester wake had silently failed, and the only trace was the stderr
> breadcrumb an embedding app never sees and an unattended timer's terminal has nobody reading.
> `WorkflowTickReceipt` therefore carries `woke` and `wake_skipped` beside `delivered` — `woke` too,
> not only the failure half, because the pair is what makes either readable (`woke: None` alone
> cannot say whether nothing was owed a wake or a wake was lost), because a tick is precisely the
> recovery path a caller reaches for when it suspects a hand-off did not land, and because it costs
> nothing: the value was already computed and discarded, and both seams serialize this receipt whole.
>
> The **workflow advance** that routing can drive had the same defect one link further down and now
> has the same answer: `AdvanceFailed { run, detail }`, reported as `advance_failed` on **both**
> `ReplyReceipt` and `WorkflowTickReceipt` (and on both seams' `--json`). It stays best-effort — the
> completion is delivered by the time it runs — but a failure there leaves the run `Running` at a
> step whose board is complete and whose idempotency marker is set, so nothing will ever route it
> again; a stalled run has no other symptom at all. No `reason` enum beside it, deliberately: a
> skipped wake has two causes that call for different responses, while an advance failure always
> means the one thing and its causes are open-ended.

### 4.2 The runs become enumerable, and their progress an event (nxf `6j6v.59rw`, added 2026-08-04)

Everything above reads ONE run, and every one of those reads resolves it the same two ways: an
explicit `--run`, or the caller's own ambient session read backwards to the run it is bound to. A
reader therefore had to already hold a run id, or BE the role the run had bound. The question *"which
work orders are running here?"* could not be put at all — which is why a derived picture across
several workspaces was impossible, and why each repo kept a hand-written status file beside the runs
that could go stale against them. **The workflow *is* the status; it just did not expose it.**

**`workflow list` is the read that needs no prior knowledge.** `WorkflowListRequest { status,
milestone, ticket, limit }` — all optional, so an empty request is the whole workspace — returns
`Vec<WorkflowRunSummaryView>`, newest first. The summary carries every scalar `WorkflowRunView` does
plus the ticket set, and deliberately NOT `step_outcomes`/`role_sessions`: a list is read to pick a
run out of it, and every run's whole advance history is exactly the payload that would make the
cross-workspace sweep expensive. `--ticket` is the reverse of the lookup `6j6v.jd37` created — from a
board item back to the work orders on it.

*Ordering is `run_id` descending, and that is a time order rather than a lexical accident*: a run id
is a ULID whose leading bits are its mint timestamp, so lexical order is mint order — including for a
run synced in from another site, which sorts by when THAT site started it. It needs no new column and
no migration, where a folded `started_at` would need both and would still be a wall clock a peer is
free to lie about.

**The event stream is the op log, projected.** `subscribe_workflow() → Receiver<WorkflowEvent>`
yields one event per run started and per step advanced — `{cursor, kind, run_id, workflow, status,
current_step, step?, outcome?, work_order?, milestone?, tickets, author, at}` — whoever wrote it:
this handle, another process, or a sync pull. `workflow_events(after) → (events, cursor)` is the same
read addressed by cursor, for an app catching up after a restart; one call returns at most
`MAX_EVENTS_PER_READ` events, so a first-time reader on a long history pages rather than
materializing the whole backlog.

The source is `ops` itself, filtered to the `workflow` domain, with `ops.rowid` as the cursor — the
local append order the substrate's own `view_watermarks.folded_through` already tracks. So there is
no new journal, no new schema and no second copy of anything. That the log is the source is what
makes the stream a true delta: a synced peer's advances produce events exactly like local ones, and
every completion is its own event with its own `step`/`outcome` — two advances between two ticks are
two events, not one collapsed "it is at step 4 now".

**The op decides WHICH events exist; the fold decides what each one says the run IS.** Conflating the
two was a real defect (PR #284 review, Code Quality #1). `fold_advance` is a keep-if-beats LWW
register, so an advance arriving out of causal order — which a sync pull genuinely delivers, a
relay's server-assigned `seq` not being `(lamport, site)` — loses, and its `next_step` is discarded.
An event deriving `current_step`/`status` from that payload would report a phase the run never
reached and silently contradict `workflow status` for the same run. So the projection reads the run's
state from the folded row and takes only `step`/`outcome` from the op — facts about the completion
itself, folded grow-only into `workflow_run_outcomes` regardless of who won the register. The same
rule answers the premature case for free: an op whose run has not folded here (an advance ahead of
its own start, which the reducer store-don't-folds) yields no event at all, because the substrate
holds no run for it to describe.

`session_bind`, `run_tickets_add` and `channel_thread_bound` are workflow ops too and are
deliberately NOT events: they are not phase. A role bound to a session is the machinery of running a
step, and a widened ticket set is a change of scope. The cursor still steps over them, so a skipped
op is read once and never again.

**Why a real event rather than the smaller "list plus the existing change hint".** The ticket named
the smaller variant as the default to be argued away from, and the argument is the tick itself:
`Engine::subscribe` is a coalesced, payload-free "someone wrote, re-read" hint over the WHOLE
workspace, and in a chat workspace the overwhelming majority of writes are messages. A reader
watching runs would therefore re-enumerate every run in every workspace on every message, and still
could not tell whether anything about a run had changed at all. The tick remains what says *when* to
look; the cursor is what decides *what* is new.

**The event stream is a library-seam surface only, and that is a decision, not an omission.** A CLI
process is short-lived, so a `nxc workflow events` verb would be a poll by construction — the exact
thing this half exists to remove. The long-lived reader is the embedding app, which is where the
subscription lives. The enumeration, whose reader is equally an agent at a terminal and an app, is on
both seams.

## 5. Seam invariant & acceptance

The invariant is the one the messaging facade already carries: **no new semantics — identical
derivations, and rejection behaviour is part of the contract.** Extraction gives most of it by
construction; the differential proves it.

`tests/parity.rs` gains an orchestration differential, in the shape the messaging one already has:
two workspaces seeded identically, the same declared roles/channels/workflow, the same script
(`workflow start` → `step done` → a channel step's completion) driven once through `nxc` and once
through `Engine`, then compared. Both sides run the **dry worker** (`NXC_WORKER=dry` for the CLI,
`WorkerConfig::Dry { log }` for the handle) so no node process is needed, and the recorded trigger
log gives a comparable trigger stream. Compared: the op logs byte-for-byte modulo the per-op random `op_id`
(`(lamport, site)`, target, field, op_type, value, author, wall_clock), the post-write reads, and the
recorded trigger stream. `now`/`origin`/`actor`/`session` are pinned on both sides, as they already
are.

> **Shipped** (`8cs7`, `crates/chat/tests/parity.rs`), as described, plus `hop` and
> `NXF_DETERMINISTIC_IDS=1` pinned on both sides and a **sibling rejection differential**: an
> undeclared role, an over-cap hop, an unknown `--name`, and `step done` with neither a run nor an
> ambient session are identical `(kind, msg)` pairs on both seams *and* persist nothing on either.
> The differential is mutation-checked (four deliberate perturbations, each seen red, each
> reverted), because a differential that passes first try proves nothing.
>
> **Three divergences are documented rather than asserted**, with the reasoning in the test's own
> section header — an "identical" claim with silent carve-outs would be worse than none:
> the workspace `db_path` (two workspaces by construction, and it only reaches a spawned session's
> `NXC_DB` stamp); `nxc reply --json`'s shape, which omits `ReplyReceipt`'s `woke`/`completed` — but
> not its `wake_skipped`/`advance_failed`, which both seams render (§4.1) — the library seam's answer
> to a question the CLI never had to ask, so the shared fields are compared and the other two
> asserted directly; and
> `DryWorker` reading `NXC_DRY_LOG` straight from process env with no injected seam, unlike
> `WorkerConfig::from_ambient` — serialized under a lock rather than worked around, since a
> log-path field on `WorkerConfig::Dry` is a public worker-seam change.
>
> **That third one is gone** (nxf 6j6v.570x): the lock was not enough, because it only bound the
> tests that SET the variable — a test that wanted no dry log took none and still inherited a
> neighbour's already-deleted `TempDir` path, which reddened `quality-gates` on an unrelated PR. The
> public worker-seam change was made: `WorkerConfig::Dry { log }` carries the path, the CLI resolves
> `NXC_DRY_LOG` once in `from_ambient_or_installed`, and the handle is TOLD where to record exactly
> as it is told which timer to use. Two divergences remain, not three.
>
> A fourth was **retired** by nxf 6j6v.m48m rather than justified again: the depth-guard message used
> to name `NXC_HOP`, a variable a library caller has no equivalent of — oddly, but identically on
> both sides. The counter is no longer primarily that variable on either seam (see §2.1), so the
> message names the hop itself and there is nothing left to carve out.
>
> **The differential's own blind spot, closed by nxf 6j6v.r5a2**: it can only compare verbs that
> exist on both seams, so a verb living on ONE seam passes it silently. Strong against divergence,
> defenceless against omission — which is how `prime` (§4.0) stayed CLI-only long enough for
> app-foundations to rebuild it in another language. `prime` now has a differential case of its own,
> covering both the `--json` projection and the rendered SessionStart block, and a new CLI verb that
> assembles anything belongs in the facade with a case beside it.
>
> **The blind spot itself is now gated** (nxf `6j6v.vtvs`): `crates/chat/tests/verb_seam.rs` walks
> the `nxc` clap tree (hidden verbs included) against the public symbols of `engine.rs` + `facade.rs`
> and fails on any verb with no counterpart and no recorded reason — see E5 §6.1 for the waiver kinds.
> Its first run is the inventory, and chat is the module it had something to say about: the messaging
> verbs and the whole workflow-run surface are on the seam, while three clusters still act straight on
> the store from `cli.rs` — agent profiles (`6j6v.1xdd`), the channel lifecycle (`6j6v.hcq1`) and the
> transcript WRITE (`6j6v.c6e8`; the read is on the seam). Each is filed as a board item rather than
> absorbed into an exception, and the gate stops accepting the waiver the moment the lift lands.
>
> **That inventory of three is out of date** (nxf `6j6v.dvyq` §3): agent profiles went with their
> VERBS rather than being lifted — a team is DECLARED, so there was never anything to lift — and
> their waivers went with them; `6j6v.1xdd` is closed. The channel lifecycle (`6j6v.hcq1`) is decided
> the same way and waits on the removal of raw channels. The transcript WRITE is the one that stands,
> and its waiver now names its addressee: app-foundations, `crates/agent-runtime`. Which verb is
> removed and what becomes of its seam counterpart is itself gated now — see
> `crates/chat/tests/seam_disposition.rs`.
>
> **And the transcript WRITE no longer stands either** (nxf `6j6v.c6e8`, 2026-09-01):
> `facade::transcript_append` → `Engine::transcript_append` is the lift the waiver was holding a
> place for, and `nxc transcript append` goes through it — the CLI keeps only its stdin framing and
> its per-line error coordinate, and the sidecar's contract (flag name, JSON-lines framing, `--json`
> record) is untouched. `seq` assignment stays in the store, so a host's batches and the sidecar's
> continue ONE history for a session rather than colliding. The addressee had the gap MEASURED
> rather than argued: `41j0.9brv` found a bound session reading `real=…  (0 entries)` — the binding
> had worked and nobody could write. That empties chat's `KnownGap` column of everything except
> `tick`, whose seam twin was removed on purpose (§3 of `6j6v.dvyq`), so no omission from this
> inventory is outstanding.

Beyond parity, the acceptance is behavioural and has to be run for real, not asserted in a test:

- an `Engine` **whose team is declared in the workspace's own `.nxs-personas/` folder** starts a
  workflow, fans a declared channel out, and drives a run to completion (this line read
  "host-supplied definitions and no `roles/` folder on disk at all" until `6j6v.dvyq` step 6 removed
  the injection path — the acceptance is unchanged, what it is driven through is not);
- an EDIT to that folder mid-flight changes which prompt the next trigger composes;
- a role declaring `model: opus` reaches a live SDK session whose `options.model` is
  `claude-opus-5` — verified against the sidecar spec file and a real run, since this is the part
  with no prior art anywhere in the path.

> **Shipped: the first two, as automated tests rather than a manual run** —
> `crates/chat/tests/embed_orchestration.rs` drives a workspace through `workflow start` →
> `step done` → channel fan-out → `reply` → `done` over a dry worker, and pins a folder edit
> changing the next trigger's composed prompt.
>
> **The three tests this paragraph used to name were renamed by `6j6v.dvyq` step 6**, when their
> subject moved from a setter to the folder:
> `set_definitions_is_visible_to_the_next_verb_on_this_handle_and_an_existing_clone` →
> `a_declaration_written_to_the_folder_is_visible_to_the_next_verb_and_to_an_existing_clone`;
> `set_definitions_races_an_in_flight_verb_without_deadlocking_or_losing_a_write` →
> `a_folder_edit_races_an_in_flight_verb_without_deadlocking_or_losing_a_write`;
> `set_definitions_changes_which_prompt_the_next_trigger_composes` →
> `editing_the_declaration_folder_changes_which_prompt_the_next_trigger_composes`.
>
> **Outstanding: the third, in its "and a real run" half.** The spec-file half is covered —
> `send_role_model_beats_the_roles_own_declaration_in_the_spec_json` reads the *real* spec JSON the
> `SidecarWorker` writes and asserts `"model": "claude-opus-5"` against a role declaring `sonnet`,
> and the sidecar's `resolveModelOption` is covered by the `node --test` suite. But no live SDK
> session has been run with a declared model: the `#[ignore]`d live smokes (`smoke_v3.rs`,
> `smoke_transcript.rs`) declare no `model`, so nothing in this repo demonstrates end to end that
> `options.model` actually took effect against the real SDK. The path is verified up to the last
> handoff and asserted no further. Recording it here rather than calling the acceptance done.

## 6. Explicitly not in scope

- **Cross-repo routing, cross-workspace bridging, repo-spanning WAIT resolution** — §1, non-negotiable.
- **Transcript authorization** (`yxsa`). The transcript read — `Engine::transcript_page` since nxf
  6j6v.yr59 — remains ungated and keeps its existing "an app serving more than one user must
  authorize the caller itself" warning. Adjacent, watched, not fixed here.
- **`member_session: resume`** was listed here as a rejection that stays. It does not: nxf
  6j6v.fepb retired the whole `member_session` field, because the two levels (`6j6v.pf6j`) answer
  its question by construction — a supervisor addresses each member as a persona on that member's
  own thread, and a reply into that thread resumes the session through the return address. The
  `6j6v.bp4k` the message pointed at was closed and was never about this field.
- **A `nexus-chat-facade` crate.** Chat's store, reducer, model, and CLI already live in one crate;
  the facade and engine are modules within it, exactly as memory's E5m argued for itself.
- **A SemVer gate.** The `facade-semver` CI gate covers `crates/facade` (flow). Chat's surface is
  documented here and stabilised by test, not by `cargo-semver-checks` — extending the gate to a
  second crate is its own decision.

  > **What that costs, stated plainly** (PR #269 review, Code Quality #2 / Integrity #3). No gate
  > means a break to chat's public API is caught by nobody: not by `facade-semver`, which does not
  > look at this crate, and not by the tests, which are compiled against the new signature. It
  > surfaces in app-foundations' next build, as a compile error — loud, but at the wrong time and in
  > someone else's repo.
  >
  > So **a break here is a manual coordination point**, and the only mechanism is the changelog: a
  > fragment with `type: changed` naming what moved, so a consumer reading the release notes learns
  > it before `cargo build` does. The follow-up round did exactly that for
  > `Worker::trigger`/`WorkerConfig` (`changes/worker-seam-break.md`). Anyone changing a `pub` item
  > in `crates/chat` owes the same.
  >
  > `#[non_exhaustive]` is the other half, and the cheaper one: `Model`, `WakeSkipReason`,
  > `TriggerError`, `WorkerConfig` and `TimerConfig` all carry it now, so a LATER variant on any of
  > them is additive by construction rather than by anyone remembering.

## 7. Coordination (to align with, not to build here)

- **`bd6g`** — the third channel kind `public`. The definition shape should not foreclose it; a new
  `Visibility`/kind variant must remain additive.
- **app-foundations `41j0.yycn`** — reconcile the role format with the M2 agent declaration. Both
  sit low in the backlog and block nothing; the point is that `Definitions::new` is the seam where
  that reconciliation will land, so its constructor takes the decl types rather than a bespoke DTO.

## 8. Delivery

This is one package, not three. All three parts cut the same seam — done separately, it would be
opened three times, and part 3 in particular would force a second pass through the very request
types part 1 defines.

The consumer downstream (`41j0.92mr`, a WAIT anchor in app-foundations) waits on a **release**, not
a merge. `nxf` does not see across repo boundaries, so that anchor appears in no blocker list here.
The work therefore ends with a release *recommendation* — version proposal plus content — and the
owner decides whether and when to release.

## 9. Mapping to implementation tickets

All under epic nxf `6j6v.qvfp`, on branch `feat/role-runtime-library-surface` (construction pattern:
`docs/specs/nexus-chat-M2.md` §11). Ordering was not arbitrary: model plumbing landed first because
it touches `RoleSpec`, which every later task threads through; `Definitions` preceded the extraction
because every extracted verb resolves declarations through it; the `Engine` surface came last of the
code because it is a pure composition of what the extraction produced, and the parity differential
could only run once both seams existed.

### 9.1 Delivered

| Ticket | Delivers | This spec |
|---|---|---|
| **`8fw8`** | `role::Model { Fable, Opus, Sonnet }` + `sdk_id()` (`#[non_exhaustive]`, the alias→SDK-id table in the engine); `RoleDecl.model` and `WorkflowStep.model` as `Option<Model>` with `skip_serializing_if`, so existing YAML round-trips byte-identically. Declaration side only — inert until `a5na`. | §3.4 |
| **`a5na`** | `Worker` as a long-lived seam: the per-hop env stamps move onto `TriggerRequest.env`, `SidecarWorker { sidecar, cwd }` becomes constructible once, `Worker: Send + Sync`; `WorkerConfig { Disabled, Dry, Sidecar }` + `from_ambient`/`build`; `RoleSpec.model` reaches the spec JSON as the resolved SDK id, absent when undeclared. | §3.3, §3.4 |
| **`jv8p`** | The sidecar honours `spec.model`: `resolveModelOption` in `spec-helpers.mjs`, absent/`null` → SDK default, malformed → default rather than an opaque session error. Closes the last link of the spawn path. | §3.4 |
| **`h2fr`** | `Definitions` — the declaration catalogue both seams resolve through: `from_roles_dir` / `new`, construction-time validation (handle form, reserved `__synth__`/`__delivered__`, uniqueness), every lookup a verb needs; referential integrity deliberately left advisory in `prime` + fail-closed at use. Carries the `role()`-resolves-by-declared-handle behaviour change. | §3.2, §3.2.1 |
| **`p3vn`** | `orchestration::Ctx` and the trigger/compose path: `trigger_role`, `resolve_model` (call › step › role, in exactly one place), `trigger_env` (the hop increment, in exactly one place); `Definitions`' team-workflow notion (`prompt_workflow` / `with_prompt_workflow`); `cli.rs::trigger_role` becomes the adapter. | §3.1, §3.4 |
| **`rws4`** | `role_trigger`, `role_resume`, `channel_open`, `ask` extracted behind `Ctx`, with the CLI copies deleted rather than left dead; `fire_channel_step` rewired in the same cut; `CliCtx` as the CLI adapter, with a **lazily** resolved worker so a plain `nxc send` never starts requiring `NXC_SIDECAR`. | §4 (`ask`, `channel_open`, `role_trigger`, `role_resume`) |
| **`7dw6`** | `reply` extracted **including** the requester wake, the declared-channel completion routing (`on_channel_complete`, synthesizer / pass-through) and the workflow-advance chain it can drive (`advance_and_fire`, `fire_role_step`, `fire_channel_step`); `ReplyReceipt` reports `resumed`/`woke`/`completed`. The behaviour fix's engine half. | §4.1 |
| **`wyhx`** | `workflow_start` / `workflow_step_done` / `workflow_tick` / `workflow_status` extracted with the two resolutions they own (`select_workflow`'s 0/1/N `--name` rules, `resolve_run_id`); `WorkflowStep.model` finally reaches the spawn path via `resolve_model` at step[0] and every later step. `cli.rs` −374 lines. | §4 (the four workflow verbs), §3.4 |
| **`d49b`** | The library surface: `EngineConfig` / ~~`DefinitionSource`~~, `Engine::open_with` / ~~`set_definitions`~~ / `definitions`, the eight verbs flat on `Engine`, and `Engine::reply` gaining the full behaviour. `Engine::open` unchanged, so orchestration is an opt-in. Plus the three decisions the design left open: `DisabledWorker` refusing at trigger time, the non-overlapping lock discipline, and `db_path`/`project_claude_md` off the handle's own resolved workspace. **`DefinitionSource` and `set_definitions` were removed again on 2026-08-17** (`6j6v.dvyq` step 6), and with them the lock discipline — the catalogue is now re-read per call from `.nxs-personas/`. | §3.2.2, §4, §4.1 |
| **`q6m2`** | ~~`nxc send --model <fable\|opus\|sonnet>`~~ — the FLAG was removed again on 2026-08-15 (`6j6v.dvyq` §3, pulled forward by `6j6v.e9qj`); the seam's `model` field and `resolve_model`'s precedence remain. | §3.4 (last paragraph) |
| **`8cs7`** | The orchestration parity differential + the rejection differential, mutation-checked — and the divergence it found: `workflow start`/`step done`/`tick`/`status` `--json` now serialize the shared type instead of a re-built `json!` object, so both seams emit the declared key order. | §5 |
| **`a9jw`** | This closing chore: the changelog fragment, this spec's status + mapping, and the release recommendation. | §8, §9 |
| **`bxdd`** | The owner's decision on the completion contract: the non-declared quorum wake catches its trigger failure like the declared one (breadcrumb + `Ok` instead of `?`), and every wake that does not land is reported as `ReplyReceipt.wake_skipped` / `nxc reply --json`'s `wake_skipped` — so "skip, don't fail" holds on both paths and "skip" means *visibly* skipped. | §4.1 |
| **`0akf`** | The same decision at the fourth and last site: `reply`'s direct 1:1 return-address resume catches its resolution AND its trigger failure instead of propagating them, reporting the same `WakeSkipped` (`unresolved` / `trigger_failed`) — while "no role session behind the address" stays the ordinary no-wake-owed case. One contract across all four wake sites, no exception left; `nxc reply` exits 0 where it used to exit nonzero. | §4.1 |

Delivered after the branch's own independent review (PR #265), under this epic and with no ticket of
its own: `WorkflowTickReceipt` now carries the completion routing's own results (`woke`,
`wake_skipped`) instead of discarding them, and a workflow advance that fails after a channel step is
reported as `advance_failed` on **both** receipts and both seams' `--json` rather than as a stderr
breadcrumb alone — §4.1's last two paragraphs.

### 9.1a Follow-up round (2026-08-02) — the remainder, closed

| Ticket | Delivers | This spec |
|---|---|---|
| **`5x9j`** | The escape hatch, in the shape the AgentCore probe demanded: `WorkerConfig::Custom(Arc<dyn Worker>)` (+ `#[non_exhaustive]`, hand-written `Debug`/`PartialEq`), `Worker::trigger -> Result<TriggerOutcome, TriggerError>` with `Started { runtime_session }` / `Accepted` and a first-class `SessionGone`, `WakeSkipReason::SessionGone` at all four wake sites, and `Engine::bind_runtime_session` as the out-of-band completion a remote host binds through. | §3.3 |
| **`d9cb`** | The step-liveness clock: `WorkflowStep.liveness` / `max_nudges`, the device-local `workflow_step_liveness` table, `timer::schedule_liveness`, `orchestration::workflow_liveness` on both seams, the nudge, the bounded escalation and its `when: stalled` route — plus the liveness half of `validate_workflow` and a liveness entry in the parity differential. | §3.5 |
| **`ep7j`** | The live smoke that closes §5's third acceptance bullet: two roles differing only in `model:`, run for real, each session's own `session_init` reporting the model it ran on. Run live 2026-08-02 — `asks_opus -> claude-opus-5`, `asks_sonnet -> claude-sonnet-5`. | §3.4, §5 |

### 9.2 Not delivered — the honest remainder

Empty as of 2026-08-02. Both entries that stood here are closed by §9.1a.

One thing is **deliberately** not built rather than merely outstanding: a `reap`/`stop` method on
`Worker`. The AgentCore probe names teardown as a network call alongside provisioning, but the engine
has no call site that would use one — nothing in the role runtime ends a session, sessions end
themselves — and a trait method with no caller is a guess at a shape. See §3.3.

Watched and deliberately untouched, as §6 said: transcript authorization (`6j6v.yxsa`) and the
third channel kind `public` (`6j6v.bd6g`). `member_session: resume` stood in this list too and no
longer does — the field was retired by nxf 6j6v.fepb rather than implemented; see §6.
