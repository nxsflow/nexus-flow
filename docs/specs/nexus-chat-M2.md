# nexus-chat — M2 thread replies & completion: quorum, `ThreadComplete`, requester-wake (design spec)

> Status: design, for review. Epic `6j6v.3v9b` (nexus-chat product line; parent epic `6j6v.m4xe`).
> Builds on the M1 substrate (`docs/specs/nexus-chat-M1.md`, T1–T4, releases 0.18.0–0.22.0) and the
> M1 acceptance dogfood (`6j6v.cdy7` — the real handoff flow this milestone is designed against).
> Vision: `docs/vision/nexus-chat.md` §"Threads & reply quorum". Consumers: the app facade
> (`6j6v.be9y`, second expansion stage) + the coordination UIs (manufakt.io / nexflow.it quorum bar).

## 1. Goal & scope

M1 **stores** the thread root and `expects_reply_from`, but tracks nothing: no one knows who still
owes a reply, and the requester is never woken. M2 makes the thread contract a **core guarantee**:

> *"The coding agent continues as soon as it has ALL review results."* — a guarantee, not a prompt
> promise.

M2 turns the latent M1 thread data into a **derived quorum** and a **data-side requester-wake**,
staying daemon-less like the rest of `nxc`.

**In scope (this epic):**

1. **Quorum derivation** — from `expects_reply_from` + the thread's reply messages, deterministically
   derive who has **replied** and who is **outstanding** (pure SQL over the M1 views — no new op-kind).
2. **`ThreadComplete`** — a **derived** predicate: all expected handles have replied. Clock-free and
   deterministic. This is the wake trigger.
3. **`stale`** — an **advisory** derived flag for an opt-in per-thread `deadline` that has passed
   while replies are still outstanding. The one clock-dependent read; **non-binding** (never completes
   a thread).
4. **Requester-wake, data-side** — `nxc inbox`/`prime` surface *"threads you opened"* that are
   **complete** (results in) or **stale** (past deadline). No process wakes anyone; the requester
   learns on its next pull, exactly like `nxf next`. **Both RENDERINGS are gone since 2026-08-27
   (`6j6v.1gm9`) — see §5.2.** The derivation stands and is carried as data (`prime --json`'s
   `threads_you_opened`); what turned out to be false is the *"no process wakes anyone"* half, which
   is why the block was a duplicate rather than a service.
5. **`nxc` verbs** — `ask` (open a thread + declare expectations + post), `threads list`/`show` (the
   quorum board), re-declaring expectations (a seam write since `6j6v.dvyq` §3 removed the verb —
   §3.2, §5.1).
6. **`hj12`** — settle the self-message question (a sender sees its own posts as unread) in the same
   inbox logic M2 already touches. *(Delivered, and removed with the inbox on 2026-09-08,
   `6j6v.4d2z` — §4.5.)*
7. **The facade seam** (`be9y`, extensible) — bulk quorum reads + `ThreadComplete`/`stale` in the
   subscribe surface, for the coordination-UI quorum bar.

**Explicit non-goals (M2 — §9):** slice *implementation* (post-approval); **process-side** waking of
managed sessions (agent runtime); **cross-workspace federation** (`6j6v.kz8p`); escalation /
supervisor **routing**; orchestrator context-hygiene; custom message kinds; a per-thread read ack.

**The daemon-less line (unchanged from M1, `6j6v.fnn1`):** the engine is passive (persist + derive);
the wake is **data** (a completed thread surfaced on pull + the facade `subscribe` re-read), never a
standing process. M2 adds **derivation and verbs**, not a service.

## 2. Data model — what M1 already carries, what M2 adds

M2 introduces **one** new stored field. Everything else is derived from data M1 already stores.

### 2.1 The thread (recap + the one new field)

M1's `threads` view (M1 §3.5 / §4) already carries the immutable root (`origin`, `channel_id`,
`opener`, `created`) and the LWW register `expects_reply_from` (JSON array of qualified handles). M2
adds one **opt-in, sparse** LWW register:

| Field | Type | Role | New in M2 |
|---|---|---|---|
| `deadline` | RFC3339 timestamp, nullable | opt-in wall-clock after which an incomplete thread reads as **stale** (advisory only). `null`/absent = wait indefinitely. | **yes** |

`deadline` is stored **absolute** (resolved from `--deadline` at `ask` time), never as a duration —
so `stale` is a single comparison against the reader's clock with no per-reader duration arithmetic
(determinism; TB-M2-6).

### 2.2 What counts as a "reply"

For a thread `T` with `expects_reply_from(T)`, a handle `h ∈ expects_reply_from(T)` has **replied**
iff there exists a `message` with `thread_id = T` and `sender = h` **written after the current
declaration of `expects_reply_from(T)`** (§4.1's `W(T)`). Consequences (all deterministic, all pure
functions of the log):

> **AMENDED 2026-08-14 by `6j6v.cg8g`.** This read "iff there exists **any** message … and
> `sender = h`" — with no reference to when the expectation was declared. See §4.1 for the amended
> definition and why: the discharge belongs to the TURN, not to the thread.

- **Set membership, not count** — `h` replying twice *within one turn* still counts once.
- **`kind`-agnostic** — any message by `h` in `T` is `h`'s reply (the engine keeps `kind` opaque,
  M1 §2.1). A team convention that a reply must be `kind:report` is presentation, not engine.
- **Only expected handles count** — a message in `T` from a handle **not** in `expects_reply_from`
  never advances quorum (TB-M2-8). The opener is not auto-added to the expected set.
- **Empty/`null` expectations ⇒ not a quorum thread** — a thread with no `expects_reply_from` never
  produces a `ThreadComplete` wake (there is nothing to wait on; TB-M2-7).

## 3. CRDT / op semantics on the shared log

M2 reuses the M1 `thread` ops (`open` root, `set expects_reply_from`) unchanged and adds exactly one
foldable `set` field.

### 3.1 `thread` `set deadline` — LWW register (the one new op)

| `target_kind` | `op_type` | `target_id` | `field` | `value` |
|---|---|---|---|---|
| `thread` | `set` | `<thread ULID>` | `deadline` | RFC3339 timestamp string |

Folded as a keep-if-beats LWW register on `deadline`/`_v`/`_site` — the **same** shape and fold path
as `expects_reply_from` (M1 §3.2/§3.5), row-may-not-exist-yet upsert included. Sparse: a thread
without a `set deadline` op simply has `deadline = NULL`.

### 3.2 Re-declaring `expects_reply_from` — opening the next turn

> **AMENDED 2026-08-14 by `6j6v.cg8g` (the turn-scoped discharge).** This section originally read
> "the in-core unstick path": narrowing the set to the handles that had already answered flipped the
> board to `complete`. That followed from the old, per-THREAD `replied` (§4.1), which this amendment
> replaces. Under the per-TURN rule a re-declaration is a QUESTION, so narrowing no longer completes
> a board by itself. The paragraph below is the current rule; the old one is stated here only so a
> reader of an older implementation knows which one they are looking at.

`expects_reply_from` is an LWW register, so re-declaring it (a later `set`) is a first-class,
deterministic operation — and it is the mechanism by which a supervisor hands a role its **next
turn**: everyone the new set names owes an answer *to this declaration*, whether or not they answered
an earlier one (§4.1). **Narrowing** it drops a handle from the expected set (that handle then owes
nothing, which is how a board blocked on a non-responder is released); **widening** it adds one. In
both cases every handle that remains named is asked again as of the moment of the re-declaration.
Because quorum is **derived against the current `expects_reply_from` and its LWW position**, no
completion state can go stale or contradict the register (TB-M2-4).

*Who may amend?* **Opener-only** (mirrors M1's exactly-2-DM rule, §5.1), checked at the seam
(`facade::set_expects`) for the friendly `forbidden` message; the store op stays pure.

## 4. Derivations — the heart of M2

All derivations are **pure reads** over the M1 views (`messages`, `threads`, `membership_*`) plus
the new `threads.deadline` column. No derivation writes; none needs a process. (`read_cursors` was
the fourth view read here; it and everything that consumed it were removed on 2026-09-08,
`6j6v.4d2z` — see §4.4 and §4.5.)

### 4.1 Replied / Outstanding

For a thread `T` with `E = expects_reply_from(T)` (parsed JSON array) and `W(T)` the LWW position of
the declaration that set it — `(expects_reply_from_v, expects_reply_from_site)`, i.e. the
`(lamport, site)` of the op that wrote the register:

```
replied(T)     = { h ∈ E : ∃ m ∈ messages . m.thread_id = T ∧ m.sender = h ∧ (m.lamport, m.site) > W(T) }
outstanding(T) = E \ replied(T)
```

> **AMENDED 2026-08-14 by `6j6v.cg8g` (the turn-scoped discharge).** The original definition carried
> no `W(T)` term — *any* message from `h` in `T`, ever, discharged the expectation. That is right for
> a one-shot review board and wrong for a thread carrying several TURNS of the same role (the normal
> case once a channel supervisor drives a conversation): from the role's first answer the thread read
> as answered forever, so `reply --if-unanswered` became a permanent no-op and the working-tree lease
> read the claim area as finished while the role was still working. **The discharge belongs to the
> turn, not to the thread**: a re-declaration of `expects_reply_from` (§3.2) resets it, and only what
> was written after the current declaration counts. The message KIND is deliberately not consulted —
> an escalating answer discharges the turn exactly like a finished one.

Implemented as a bound SQL join `threads ⋈ json_each(expects_reply_from)` with a correlated `EXISTS`
per handle carrying the `(lamport, site) > W(T)` comparison — **bulk-friendly**: one query returns
the per-handle state for every thread in a set (§8, the quorum-bar read; no N+1). The comparison
rides the causal index `messages(thread_id, lamport, site, message_id)` as a range seek.

### 4.2 `ThreadComplete` — the wake trigger (clock-free, deterministic)

```
complete(T) = E ≠ ∅ ∧ outstanding(T) = ∅
```

`complete(T)` has **no time term** — it is a pure function of the log, so every reader (and every
synced replica) computes the identical value and it converges the moment the last expected reply
folds. This is the guarantee: *complete ⟺ everyone expected has replied*. Nothing else — no majority,
no timeout — ever sets it (TB-M2-1, TB-M2-2).

Because completion is derived against the **current** `E` *and the current declaration's position*
`W(T)` (§4.1), any re-declaration of `E` (§3.2) can flip `complete` `true→false` — every handle the
new declaration names owes an answer to it. Dropping a handle from `E` removes its debt; a handle
that stays named is asked again. All of it is deterministic w.r.t. the view.

> **AMENDED 2026-08-14 by `6j6v.cg8g`.** This paragraph originally said narrowing `E` can flip
> `complete` `false→true` (completing a board off answers given before the narrowing). Under the
> turn-scoped `replied` it cannot: the narrowed set is asked again, and the board completes when that
> set answers.

### 4.3 `stale` — advisory, clock-dependent, non-binding

```
stale(T, now) = deadline(T) ≠ NULL ∧ now > deadline(T) ∧ outstanding(T) ≠ ∅
```

The **only** clock-dependent read in M2 (`now` = the reader's wall-clock; it flips at the first read
after the deadline). `stale` is **advisory**: it never appears in `complete(T)`, never wakes anyone
as "done", and never mutates the thread. It is a nudge — *"this board is past its deadline and still
waiting on `outstanding(T)`"* — and what to do about it (chase, drop the reviewer via §3.2, proceed
manually) is the requester's / a handler's call. **Escalation routing is out of M2** (§9). A complete
thread is never stale (`outstanding = ∅`).

> **Superseded in part, 2026-08-16 (nxf 6j6v.pf6j / 6j6v.nf38).** The DERIVATION above is unchanged
> and still the only one. What no longer holds is *"never wakes anyone"*: a declared channel's
> supervisor treats a `stale` member as settled, so a member's deadline now releases the channel's
> consolidation and the requester IS woken by it. TB-M2-1 is untouched and is a different question —
> a deadline still never makes a thread `complete`; what discharges a timed-out channel is the
> consolidator's own reply, not the clock. On a channel MEMBER thread the deadline is also no longer
> one fixed instant: every write to that member's session transcript restarts the declared window
> (6j6v.nf38). Everywhere outside a declared channel, §4.3 reads exactly as written.

### 4.4 The requester-wake surface — a WINDOW, not an acknowledged debt

**Superseded 2026-09-08 (`6j6v.2hx9`, then `6j6v.4d2z`).** As delivered, the wake rode M1's
per-`(consumer, channel)` `read_cursor` and added no new ack kind: a thread was
**complete-and-unacked** when `complete(T)` held and `C`'s cursor in `T`'s channel was behind the
latest reply, and it cleared when `C` acked past the results. That had a *known simplification* —
the ack was channel-granular, so acking past a later message in another thread of the same channel
also cleared this thread's annotation (TB-M2-5) — and one defect that was not known: **nothing ever
acked**, so the list never cleared and a session start replayed every commission the caller had ever
finished.

What holds now, for an opener `C` and a thread `T` (`opener(T) = C`):

- **Finished inside the window**: `complete(T)` **and** the completing reply landed after `C`'s
  PREVIOUS SESSION END. No acknowledgement is involved, and none is needed — "finished" does not
  become unfinished, and the window moves on its own each time a session ends. A caller with no
  recorded session end (a human at a terminal, a persona whose sessions this device has never seen
  end) has no window at all rather than an unbounded one. The completing reply is **annotated**
  with "completes thread `T` (N/N in)" exactly as before.
- **Stale**: `stale(T, now)` surfaces `T` in `C`'s `prime` as an advisory status line — it
  self-clears when the thread completes, the deadline is lifted, or all reply.

*The cost, stated rather than discovered:* a caller that starts twice in a row sees the notice once.
That is a different promise from "until you acknowledge it", and it is the promise this read makes.

### 4.5 `hj12` — a sender's own posts are not unread to itself · REMOVED 2026-09-08 (`6j6v.4d2z`)

M1's `unread(C, ch)` (M1 §4) filtered only by cursor and so included messages `C` itself sent. M2
**excluded the caller's own posts** from the unread set:

```
unread(C, ch) = { m ∈ messages : m.channel_id = ch
                  ∧ m.message_id > coalesce(seen(C,ch), '')
                  ∧ m.sender ≠ C }
```

This was the lighter of the two options considered (the other: auto-advancing the sender's cursor on
`send`, which costs an op per send and conflates *authored* with *read*), it matched messaging-UX
convention, and it was **coherent with the M2 wake**: the opener's own request is not "unread" to
the opener, and the opener's own posts never counted toward its board's quorum anyway (TB-M2-3,
closes `6j6v.hj12`).

**The rule went with the set it was a rule about.** There is no `unread(C, ch)` any more (M1 §3.4),
so there is no place for an exclusion: what a caller reads of a channel is its whole conversation,
own posts included, which is what `nxc threads show` and `nxc status` always showed. The half of
`hj12` that outlives it is the one that was never about the inbox — an opener's own request does not
count toward its own board's quorum — and that lives in §4.1.

## 5. Verb surface (`nxc`) — additive to M1

`--json` deterministic (stable field order) everywhere, human output alongside; the CLI stays a pure
consumer of the derived views (M1 §5). New verbs and one annotation on the existing wake render.

### 5.1 Thread verbs

| Verb | Behavior |
|---|---|
| ~~`nxc ask <channel> "<body>" --expect <handle> [--expect <handle>…] [--deadline <when>] [--kind K] [--priority P] [--ref k=v]…`~~ | **REMOVED 2026-08-19 (`6j6v.dvyq` §3).** `nxc send --to <channel> "<body>"` opens the board and the channel's own declaration says who must answer, by when and what becomes of the answers — so `--expect` was a second answer to a declared question and went with the verb, while `--deadline` MOVED to `nxc send --deadline`. **That flag was itself REMOVED on 2026-08-21 (`6j6v.ckeq`):** a board's window is the CHANNEL's own `timeout:`, and a per-call override beside a declared value is two answers to one question. The parenthetical this line used to carry — "a duration has a declared form, `timeout:`; an absolute instant has none" — was never true: `timeout:` is parsed by the same `facade::resolve_deadline_spec`, which tries RFC3339 first, so an absolute instant is declarable too and nothing was lost. The WRITE is untouched: `facade::ask` still mints the thread, declares its expects, stamps the deadline and posts the request, and is what the declared-channel fan-out runs. What follows describes that write. The atomic "open a review board" verb: mint a `thread` ULID, emit `thread open` (root) + `thread set expects_reply_from` + (if `--deadline`) `thread set deadline`, and post the request `message` (thread_id set, `--kind` default `question`). Returns `{ thread_id, message_id, persisted:true, expects:[…], deadline?:… }`. `--deadline` accepts an absolute date or a duration (e.g. `24h`) resolved to absolute at mint time. |
| `nxc reply --thread <thread_id> "<body>" […]` | As in M1 — and since 2026-08-19 (`6j6v.dvyq` §3) the thread is the ONE address; the positional `<thread_id\|message_id>` is gone. A reply by a handle in `expects_reply_from` advances quorum (§4.1). |
| `nxc threads list [--consumer <handle>] [--channel <id>]` | The caller's threads (membership-scoped), each with the **bulk** quorum state: `{ thread_id, channel_id, opener, expects, replied, outstanding, complete, deadline?, stale }`. Deterministic order (channel + thread ULID). |
| `nxc threads show <thread_id> [--consumer <handle>]` | One board in full: root, per-handle `replied`/`outstanding`, `complete`, `deadline?`, `stale`, and the reply messages in order. |
| ~~`nxc threads expect <thread_id> --set <handle>[,<handle>…]`~~ | **REMOVED 2026-08-14 (`6j6v.dvyq` §3, owner: "sehe ich nicht, wofür das notwendig ist").** Re-declaring `expects_reply_from` (LWW, §3.2) is not a human-typed verb: the write lives on the library seam as `facade::set_expects` (opener-only, `not_found`/`forbidden` unchanged) and is issued by the channel supervisor to open a role's next turn. |

Failure paths are M1's contract (`not_found` unknown thread/channel, `forbidden` non-member /
non-opener amend); the store stays pure, the CLI checks for the friendly message (M1 §5.1).

### 5.2 The wake in `prime` / `inbox` (annotation, not a new verb)

> **REMOVED 2026-08-27 (`6j6v.1gm9`). The DERIVATION stays; the two renderings below are gone**, and
> so are the verbs that carried them (`nxc inbox`, `nxc read` — M1 §5.1). Measured in the workspace
> that decided it: this block was **46.487 bytes of a 63.787-byte composed session start, 74 %** —
> 24 boards, six complete, each replaying its whole conversation, so a finished commission from
> yesterday was read out in full to every session today. Above the host's SessionStart cut-off the
> yield of the entire block is nil, not merely reduced.
>
> **What made it a duplicate rather than a service** is §6's premise below, which no longer holds:
> delivery is not pull. All three paths push — `send` starts a session with the body in its prompt,
> `reply` resumes the target with the reply's body, a completed quorum wakes the opener
> (`wake_role_requester`) — and `Engine::prime_as` says of the wake that it is "the same derivation"
> it renders here. The brake, `nxc read`, measured 0 uses across 66 role sessions.
>
> **Nothing replaced it.** The one candidate — "a thread awaiting a reply from THIS session" — was
> never in this block: it is the OPENER's view, and `expects_reply_from` on every board in it names
> other handles. A HUMAN reads a conversation with `nxc threads show` / `nxc status`, untouched. The
> data-side record survives on the app seam as `prime --json`'s `threads_you_opened`.

`nxc inbox` and the `nxc prime` catch-up gain, for the caller `C`, a derived **"Threads you opened"**
block, deterministically ordered:

- **Complete (N)** — each complete-and-unacked thread (§4.4) with its collected replies; the
  completing reply is annotated `✓ completes <thread> (N/N)`. Cleared by the ordinary `nxc read`.
- **Waiting past deadline (N)** — each `stale` thread with its `outstanding` handles (advisory).

`prime` remains deterministic (channel + ULID sorted); the block is empty (and omitted) when `C` has
opened no complete/stale threads.

### 5.3 `nxc transcript` — the role session's agent transcript (post-M2, nxf epic `6wt2`)

A **message** is a role's distilled work product; the **transcript** is what the role actually did to
produce it. Each role turn runs as a Claude Agent SDK session in the Node sidecar
(`agent-sidecar/src/main.mjs`, the role runtime this section's §6 note introduces); that stream used
to be consumed for its session id alone and discarded. It is now captured, stored, and readable.
Documented here because it lands on the same `nxc` verb surface and the same facade seam (§8).

**Wire contract (sidecar → `nxc`).** The sidecar's normalizer coalesces the stream into one JSON
object per line on stdin — never raw token deltas:

```json
{ "kind": "tool_use", "at": "2026-07-26T07:42:39.123Z", "toolUseId": "toolu_abc",
  "parentToolUseId": "toolu_parent", "subagentType": "code-reviewer", "data": { } }
```

`kind` (required) is one of `session_init` | `assistant` | `thinking` | `tool_use` | `tool_result` |
`result` | `error` — a string, not an enum: the sidecar owns the vocabulary and an unknown kind from
a newer sidecar must still be STORED (it is evidence). `at` is OUR clock. `parentToolUseId` present
⇒ the entry happened inside a Task-spawned **subagent**, and names the spawning Task `tool_use`.
`data` is the kind-specific payload, **opaque** to the store. `seq` is deliberately NOT on the wire:
the store assigns it, so a `--resume` that starts a second sidecar process for the same session
continues the transcript instead of colliding with it.

**Storage.** A **device-local, plain `agent_transcript` table** (`internal_session, seq, kind,
tool_use_id, parent_tool_use_id, subagent_type, at, data`, `PRIMARY KEY(internal_session, seq)`)
next to `session_map` — **not** op-folded, for three compounding reasons: it describes an SDK session
that exists on THIS machine only (meaningless to a sync peer), there is exactly one writer per
session (no concurrent-edit conflict an LWW/OR-set reducer would resolve), and folding it would
balloon the SHARED op log with by far the highest-volume thing chat records. There is deliberately no
foreign key to `session_map`: a transcript for a session that was never bound is exactly the evidence
of what went wrong.

**Caps.** Two are applied **producer-side**, in the normalizer, where the value is seen
pre-serialization: extended thinking at 4000 chars and a tool result at 8000, each with a truncation
marker the store never re-applies and never strips. `tool_use`'s `data.input` is deliberately
**uncapped** — it is the agent's own product and the exact evidence a transcript exists to preserve
(unlike a tool RESULT, which is re-derivable by re-running the read), the cost is local disk that
never syncs, and capping it would mean the store learning an opaque payload's shape. Retention, not
truncation, is the lever if that ever bites. The CLI's *human* timeline bounds its per-entry gist for
readability; `--json` is always whole.

| Verb | Behavior |
|---|---|
| `nxc transcript append --session <internal>` | The sidecar's callback: JSON-lines on stdin, appended as ONE atomic batch with `seq` assigned store-side. `{ session, appended }`. A malformed line or an empty `kind` is a `validation` error naming the line, and rejects the whole batch — the sidecar propagates it, so a silent skip would lose transcript with no trace. |
| `nxc transcript show <internal>` | The session's transcript as a **nested** timeline: main conversation in `seq` order, each subagent's entries under the `tool_use` that spawned them. An unknown session renders empty, never an error. |

**Nesting + the orphan rule.** An entry tagged `parentToolUseId = X` nests into the `subagent` vec of
the main-conversation `tool_use` whose `toolUseId` is `X`. Nesting is exactly one level (the SDK
reports the spawning Task's id for grandchildren too, so they surface under the same parent), and
only a `tool_use` can parent — a Task's own `tool_result` comes back on the main conversation
carrying the same id. An entry whose parent is **not in this transcript** (a flush mid-Task, a lost
parent entry) is emitted at TOP level in its own `seq` position, keeping its `subagent_type`:
**nothing is dropped**. This is the gap the beads-dashboard blueprint left open — its persist path
sanitized subagent entries away, so sub-timelines vanished on reload.

**The seam.** `Engine::transcript_page(internal_session, after_seq, limit) -> TranscriptView` (§8)
is the surface app-foundations mirrors, and `nxc transcript show --json` prints exactly that value —
one rendering of one read, for the session-detail timeline an embedding app draws. Read the whole
session with `(-1, None)`: the unwindowed `Engine::transcript` was that call and was removed as the
pure overload it was (nxf 6j6v.yr59), so the unbounded read is one a caller asks for rather than one
it is handed.

The WRITE joined it at `facade::transcript_append` → `Engine::transcript_append(internal_session,
entries) -> u64` (nxf 6j6v.c6e8). It was store-direct from `cli.rs` until then, which left a host
that drives the Claude Agent SDK itself — the case `Engine::bind_runtime_session` exists for — able
to bind a session and unable to record anything in it. `nxc transcript append` now goes through the
same call, so the table above still describes the verb exactly: the stdin framing, the per-LINE
error coordinate and the `--json` record stay in the CLI, and `seq` assignment stays in the store,
which is what lets a host's batches and the sidecar's continue one history rather than collide.

## 6. Delivery model — pull-only, data-side wake, no daemon (reaffirmed)

M2 changes **what is derived**, not **how delivery works**. The wake is not pushed: a consumer learns
a thread completed only when *it* pulls (`nxc inbox`/`prime`, or the facade `subscribe` re-read on a
`Change`).

> **This paragraph is the premise `6j6v.1gm9` overturned (2026-08-27), and it is left standing
> because §5.2's removal is unreadable without it.** By the time it was tested, a completing quorum
> DID wake the opener (`wake_role_requester`, arriving with `6j6v.zenf`/role-runtime-v2 — §7 already
> records that amendment), a `send` started a session with the message in its prompt and a `reply`
> resumed one with the reply's body. Delivery to an AGENT is a push on all three paths; the pull
> surface it was designed against was used 0 times in 66 measured role sessions. What is still
> pull-only is the HUMAN surface (`nxc threads show`, `nxc status`) and the app seam's `subscribe`,
> and the daemon-less architecture is untouched. Sync carries the reply `message` ops and the `deadline` op with their `domain` preserved
(M1 §6/§7); the peer **re-derives** quorum from its own folded views — completion converges without
any completion op ever crossing the wire. The `stale` clock read is local to each reader by
construction. No standing service, no timer, no socket (M1 §6; `6j6v.fnn1` §6).

This scopes M2 itself — an ordinary (non-managed) consumer never gets pushed to. §9's own non-goal
list already carves out the one exception: "process-side waking of **managed** agent sessions" is
named there as belonging to a separate epic, **the agent runtime**. That epic has since landed
(`6j6v.zenf`/role-runtime-v2): a completing quorum now DOES spawn a live SDK session for the
thread's role-bound opener (`cli.rs`'s `reply`, the quorum PUSH-wake block) — this is the
anticipated exception arriving, not an undocumented reversal of the paragraph above, and it is
still scoped to role-bound openers only; a human/ad-hoc `ask` opener still only ever learns of
completion by pulling, exactly as this section describes.

## 7. Multi-module coexistence & wire compatibility

**flow/memory stay byte-identical** — M2 touches only chat's views/derivations. The differential
oracle and the flow/memory trycmd goldens remain unchanged and green (M1 §7).

**The one schema change is additive.** `threads` gains `deadline` + `deadline_v` + `deadline_site`
(a foundation-additive, old-binary-safe migration — platform §4.3). The **forward-compat** property
is already in the M1 fold gate: `is_foldable` admits a `thread` `set` op **only** when
`field == "expects_reply_from"` (`crates/chat/src/message_reducer.rs`), so an M1 binary that meets an
M2 `set deadline` op **store-don't-folds** it — it neither panics nor drops the op (it stays in the
`ops` log, the source of truth), and it folds once the binary is M2. Adding `deadline` to the
foldable whitelist is therefore a **sparse-key minor**: it must **bump chat's view-schema version**
so an existing workspace refolds and applies any `deadline` ops that folded through under M1 as
deferred. Pre-M2 threads (no `deadline` op) are byte-identical under M2 — `deadline` reads `NULL`,
`stale` is always false, `complete` is unchanged.

**Global ids are untouched.** M2 mints no new id form; the T4 global `message`/`thread` ids (M1 §2.2)
are unchanged — the standing prerequisite for cross-workspace federation (`6j6v.kz8p`). Quorum is
derived, so a bridged/mirrored reply from a qualified external handle folds and counts with no extra
machinery once `kz8p` lands (M2 neither needs nor blocks it).

## 8. Consumer seam — the facade (`be9y`, second expansion stage)

The coordination UIs (manufakt.io / nexflow.it, Claude "Coordination") render a **quorum bar** and
thread state through the in-process facade, not the CLI. The facade seam adds **no semantics** — it
returns the **identical** derivations as `nxc` (seam invariant, M1 `be9y` / flow `9t7`) — and is
**bulk-first**: one call returns the §4.1 per-handle state for a set of threads (the quorum bar reads
all of a project's boards at once, no N+1), and the existing `subscribe` re-reads the quorum view on
each `Change` to push `complete`/`stale` transitions to the UI. `be9y` already exposes threads
extensibly, so M2 **extends** that surface: `{ expects, replied, outstanding,
complete, deadline?, stale }` per thread, plus the opener's complete/stale wake lists (§4.4).

## 9. Deliberate non-goals (M2)

- **Slice implementation.** This spec + the T-slice decomposition (§11) is the deliverable; the
  slices are implemented after approval.
- **Process-side waking of managed sessions.** M2's wake is data (surfaced on pull + facade
  `subscribe`). Reactivating a **managed** agent session on `ThreadComplete` is the **agent runtime**
  (vision §"The agent runtime"), a separate epic.
- **Cross-workspace federation** (`6j6v.kz8p`). M2 is intra-workspace; the bridge is later and M2's
  derived quorum is federation-ready (§7) without depending on it.
- **Escalation / supervisor routing.** M2 surfaces `stale` + `outstanding`; *who* gets escalated and
  *how* is an app/handler/human decision. The in-core resolution is re-declaring `expects_reply_from`
  (§3.2).
- **Orchestrator context hygiene** (vision open point). A different M2-era concern; not this epic.
- **Custom message kinds** (vision, post-M1). *(A **per-thread read ack** stood beside it as a
  deferred follow-up, while §4.4 rode the channel cursor. It is no longer deferred: the wake needs
  no ack — see §4.4 and TB-M2-5.)*

## 10. Open points / tie-breaker log

| ID | Question | Resolution |
|---|---|---|
| TB-M2-1 | Does a timeout auto-**complete** a thread? | **RESOLVED (owner, 2026-07-11): NO.** `complete(T)` = all expected replied, full stop — clock-free and deterministic. A passed `deadline` only marks a thread **`stale`** (advisory, §4.3), never complete. **Supersedes** vision lines 141–143 ("… as soon as everyone has replied **or a timeout kicks in**") — the "or timeout" half is dropped so a requester never proceeds on an incomplete review board silently (safety over convenience). |
| TB-M2-2 | Partial / majority (M-of-N) quorum? | **RESOLVED: NO.** Strict: only all-replied completes. No configurable fraction in the core — a majority rule would be a handler/presentation policy, and could let work proceed without a specifically-required reviewer. |
| TB-M2-3 | `hj12` — own posts in own inbox? | **RESOLVED: exclude `sender == C`** from `unread(C, ch)` (§4.5). Lighter than auto-advancing the cursor, convention-matching, and coherent with the wake. Closes `6j6v.hj12`. **MOOT since 2026-09-08 (`6j6v.4d2z`): there is no inbox.** The half that outlives it — the opener's own post does not count toward its own board's quorum — is TB-M2-8. |
| TB-M2-4 | How to unstick a non-responding reviewer without escalation? | **RESOLVED: re-declare `expects_reply_from`** (LWW, §3.2) — an in-core, deterministic release. **AMENDED 2026-08-14 (`6j6v.cg8g`):** dropping the non-responder from the set is what releases the board; narrowing to the handles that already answered does NOT complete it, because a re-declaration asks everyone it names again (§4.1, the turn-scoped discharge). **AMENDED 2026-08-14 (`6j6v.dvyq` §3):** there is no `nxc threads expect` verb any more — the write lives on the seam (`facade::set_expects`) and is issued by the channel supervisor. Escalation *routing* is out of M2 (§9). |
| TB-M2-5 | A new completion-ack kind, or ride the `read_cursor`? | **RESOLVED: ride the existing per-`(consumer,channel)` `read_cursor`** (§4.4) — no new op-kind (only `deadline` is new). Channel-granular ack is the known simplification; a per-thread ack is deferred (same axis as M1's channel cursor). **OVERTAKEN 2026-09-08: neither.** The question assumed the notice is a debt to be discharged; `6j6v.2hx9` made it a WINDOW over the caller's own session boundary, which needs no ack at all, and `6j6v.4d2z` then removed the cursor. The deferred per-thread ack is not deferred any more — it is not wanted. |
| TB-M2-6 | Store `deadline` as absolute or duration? | **RESOLVED: absolute RFC3339.** `--deadline 24h` is resolved to an absolute instant at `ask` time, so `stale` is one comparison with no per-reader arithmetic (determinism). |
| TB-M2-7 | A thread with empty/`null` `expects_reply_from`? | **RESOLVED: not a quorum thread.** `complete` requires `E ≠ ∅`; such a thread never produces a wake (nothing was expected). |
| TB-M2-8 | Does the opener / a non-expected sender count toward quorum? | **RESOLVED: no.** Only messages from handles **in** `expects_reply_from` advance `replied`; the opener is not auto-added. |

## 11. Mapping to implementation tickets

Derived from this spec, created under epic `6j6v.3v9b` (M1 T1–T4 pattern). Implementation follows
**after** spec approval.

| Ticket | Delivers | This spec |
|---|---|---|
| **T1** | Reducer/views: the `deadline` LWW `set` field (fold path = `expects_reply_from`'s) + the chat view-schema bump/refold; the **derivations** `replied`/`outstanding`/`complete`/`stale` as bound bulk SQL; and the convergence/robustness tests — reorder-independence of the quorum result; a reply by a **non-expected** handle does not advance quorum; the **opener's own** post does not count; re-declaring `expects_reply_from` re-asks everyone the new set names (amended by `6j6v.cg8g`; it originally read "**narrowing** completes a board while **widening** re-opens it"); `deadline` LWW keep-if-beats + `stale` flips only after `now > deadline` with `outstanding ≠ ∅`; empty-`E` is never complete. | §2, §3, §4.1–§4.3 |
| **T2** | `nxc` verbs: `ask` (atomic open+declare+post, `--deadline` absolute-resolved), `threads list`/`show` (bulk quorum board), `threads expect` (opener-only re-declare; the VERB was removed again by `6j6v.dvyq` §3 — the seam write `facade::set_expects` remains) — all `--json` deterministic, **incl. the `not_found`/`forbidden` paths**. | §2.2, §5.1 |
| **T3** | The **requester-wake** surface: the `inbox`/`prime` "Threads you opened" block (complete-and-unacked riding the `read_cursor`; stale advisory) with the completing-reply annotation — **and `hj12`** (exclude own posts from `unread`), since it is the same inbox derivation. *(The task as it was delivered. The block is no longer rendered (`6j6v.1gm9`), the gate is the caller's session window rather than the cursor (`6j6v.2hx9`), and the inbox and `hj12` are removed (`6j6v.4d2z`) — §4.4/§4.5 carry each change.)* | §4.4, §4.5, §5.2 |
| **T4** | The **facade** seam extension (bulk quorum reads + `complete`/`stale` in `subscribe`, seam-invariant, bulk-first) + the **wire-compat / coexistence gate**: an M1 binary store-don't-folds an M2 `deadline` op (no panic/loss); pre-M2 threads byte-identical; flow & memory byte-identical (oracle + goldens green); global `message`/`thread` ids untouched. | §7, §8 |

Acceptance of the milestone end-to-end (a real multi-reviewer board: `ask` → replies accrue →
`ThreadComplete` wakes the opener; a `deadline` board goes `stale`; a re-declared board is answered
again and completes — "a narrowed board completes" before `6j6v.cg8g`) is a follow-up dogfood ticket
(pattern: M1's `6j6v.cdy7`), not part of this spec.
