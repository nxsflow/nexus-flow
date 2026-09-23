# nexus-chat — M1 messaging substrate: data model & `nxc` CLI contract (design spec)

> Status: Design · As of: 2026-07-08 · Phase: **P4** (nxs roadmap, epic `6j6v.m4xe`)
> Reference: `docs/specs/nxs-platform-foundation.md` §4 (data architecture), §5 (crate topology),
> §6 (contracts), §7 (`.nxs/` layout); `docs/specs/nxs-memory-datamodel.md` (the sibling
> kind-product this spec mirrors); `docs/vision/nexus-chat.md` (product vision — **partially
> superseded**, see §9 TB-C6); the architecture decision `6j6v.fnn1` (nxs-member + sync-as-bus,
> sync-ack/async-pull, no daemon — incl. the 2026-07-04 federation amendment).
>
> This spec defines the **message product data model** (message/channel/profile/membership/thread/
> read-cursor) and the **`nxc` M1 verb surface** on the shared nxs substrate. It is the source of
> truth for the M1 implementation tickets `T1`–`T4` (§10). The engine is **passive** (persist +
> derive); event management (subscribe + async fan-out) is an **app layer**, not part of the M1
> CLI — this spec draws that line (§6).

## 1. Goal & scope

nexus-chat is the **third product on the shared nxs foundation** — "Slack between AI agents"
(`docs/vision/nexus-chat.md`): specialized agents — and the user as one of them — assign each other
tasks, ask questions, and report results. It is **not** the issue tracker: *messages are
communication, issues are work* — a message may **reference** `nxf` ids but manages no task state.

chat is the same substrate as flow and memory in a different vocabulary (platform §1): append-only
op-log → materialized views → keep-if-beats LWW + observed-remove OR-set + grow-only lattices on
SQLite. flow calls its unit "task" (domain `task`), memory "fact" (domain `fact`); chat calls it
**"message"** (domain **`message`** — already reserved in `crates/foundation/src/model.rs` and
`reducer.rs`). The **message reducer** is the clean, independently testable data boundary (platform
§4.2/§4.4): it folds `message`-domain ops into the chat views and knows **no** flow/memory
vocabulary; the foundation knows **no** chat vocabulary.

**In this spec:**
- the message-domain data model — the six intra-domain kinds and their CRDT classes (§2, §3);
- the materialized views and their DDL (§4);
- the `nxc` verb surface: messaging verbs + the platform contract verbs (§5);
- the M1 delivery model — **pull-only**, sync-on-demand + SessionStart/Stop hooks, **no daemon**;
  the app-layer event-management line (§6);
- forward-compat for cross-workspace federation: global ids + qualified handles, achieved **without
  a foundation op-schema change** (§2.2), so the later bridge (`6j6v.kz8p`) needs no data migration;
- multi-module coexistence with flow/memory in one `.nxs/` db (§7);
- the M1 boundary: what filtered sync M1 needs vs. what is a follow-up (§9).

**Not in this spec (deferred):**
- **public channels / external consumers** and the multi-tenant identity/ACL they require → later;
- **filtered / kind-scoped sync** (per-kind + channel-membership/ACL on the wire) → its own sync
  ticket (§9 TB-C1) — M1 is intra-workspace, full-log;
- **agent runtime (managed mode)** and the **workflow engine** — both need the standing process this
  architecture deliberately avoids (§9 TB-C8) → separate epics;
- **thread quorum / `ThreadComplete` derivation** → **M2** (§9 TB-C5); M1 only *stores* thread
  membership and `expects_reply_from`;
- the **2-agent dogfood handoff** (acceptance) is a follow-up ticket (`6j6v.cdy7`), not spec content;
- the **cross-workspace bridge contract** itself → `6j6v.kz8p`;
- frontend, AWS seams.

## 2. Data model — the `message` domain

Every chat op carries `domain = "message"`. Within the domain the reducer dispatches on
`target_kind` — exactly how flow's task reducer folds `item`/`edge`/`note`/`label` under one `task`
domain (`crates/core/src/task_reducer.rs`). M1 defines **six** kinds:

| `target_kind` | CRDT class | Mutable? | Purpose |
|---|---|---|---|
| `message` | **grow-only** (one op = one message) | no (append-only, immutable) | the messages themselves — the events of the system |
| `channel` | **LWW register** per field | yes | a DM (exactly 2), a named group channel, or a `public` one (post-M1, §3.2); metadata |
| `membership` | **observed-remove OR-set** | yes (reversible) | who is subscribed to / a member of a channel |
| `profile` | **LWW register** per field | yes | an agent (or the user) — handle, role, reachability, `reports_to` |
| `thread` | grow-only root + **LWW** fields | partly | a request's reply-tracking envelope (data only in M1; quorum → M2) |
| ~~`read_cursor`~~ | ~~**grow-only max-register** per `(consumer, channel)`~~ | ~~monotone~~ | ~~a consumer's per-channel read watermark — **synced** (§3.4)~~ · **REMOVED 2026-09-08 (`6j6v.4d2z`)**, see §3.4 |

### 2.1 The message envelope

A `message` is **immutable and append-only** — it is never edited, only posted. It carries (vision
lines 126–130):

| Field | Type | Role |
|---|---|---|
| `message_id` | ULID | global id (`m-`-prefixed mint, §2.2), minted locally, sortable by time |
| `origin` | string | the minting workspace identity (§2.2) |
| `channel_id` | string | the channel it was posted to |
| `sender` | qualified handle | `<origin>/<agent>` (§2.2) |
| `kind` | enum | `task` \| `question` \| `report` \| `decision` \| `info` |
| `priority` | enum | `urgent` \| `normal` \| `background` |
| `disposition` | enum | **`in_turn`** \| **`next_session`** — when the recipient should act on it |
| `thread_id` | ULID? | the thread this message belongs to (null = not threaded) |
| `refs` | object | `{ session_id?, nxf_ids?: [..], branch?, pr? }` — pointers, not content |
| `body` | string | Markdown — the **distilled** work product (assignment/report/question), not a transcript |
| `created` | wall-clock | display timestamp (display-only, never order-bearing — platform §4.1) |

`kind`, `priority`, `disposition` are **orthogonal** axes: `kind` classifies *what* the message is,
`priority` *how loud* it is, `disposition` *when* it wants attention. The engine treats all three as
opaque enum strings — vocabulary/labels/ranking are a later presentation concern, not engine logic
(platform §4.4). Custom message kinds (vision line 254) are explicitly **post-M1**.

### 2.2 IDs & handles — global by construction, no op-schema change (forward-compat)

Forward-compat for cross-workspace federation is a **hard M1 requirement** (the `6j6v.3cz4`
forward-compat note): M1 must already mint **global** ids and **qualified** handles so the later
bridge (`6j6v.kz8p`) needs **no** data migration. Resolution:

- **`message_id` / `thread_id` = ULID** — globally unique *by construction* (128-bit, no
  coordination), and **lexicographically time-sortable** (used as the read-cursor order, §3.4). Held
  in the op's `target_id`. A ULID minted in any workspace never collides with another's, so a bridge
  can relay messages across origins without re-keying.
- **`origin`** — the minting workspace's stable identity string, **folded as a field** of every
  message/thread/channel/profile (never inferred at read time). In M1 it is the local workspace
  identity (from the replica/`config`); the org/repo-qualified form (`org/repo`) is finalized with
  the bridge (`6j6v.kz8p`). What M1 guarantees is that the **field exists and travels through sync**
  — attribution needs no migration later.
- **Qualified handles** — a profile's identity (and a message's `sender`) is a qualified handle of
  the form **`<origin>/<agent>`** (target: `org/repo/agent`). `runtime_binding ∈ { local, bridged }`
  distinguishes an agent reachable in this workspace from one reachable only via the bridge. In M1
  every handle is `local`; the `bridged` value and the third path segment are the seam the bridge
  fills, left open here on purpose.
- **Kind prefix.** Platform §4.3 reserves one ID prefix per kind (`t-`/`f-`/`m-`); chat's is **`m-`**.
  Minted message/thread ids carry it (`m-<ULID>`) as a recognizability convention for system-minted
  ids — the same framing memory uses for `f-` and flow for its replica-prefixed ids: a convention for
  *minted* ids, not a hard namespace on every domain id (memory §2.2). Structural ids that are not
  minted ULIDs — a qualified handle, a `dm:`-derived channel id — are namespaced by their own form.
  ("ULID" elsewhere in this spec is shorthand for this `m-`-prefixed mint.)
- **Trust.** In M1 `origin` and handles are **unauthenticated** product-data — any writer can stamp
  any `origin`. Binding them to an authenticated peer identity is a **required** part of the bridge
  contract (`6j6v.kz8p`), flagged here so the locked-in scheme is not mistaken for trusted (§9 TB-C4).

**This is achieved entirely at the product-data level** — `origin` and the handle format are chat
**view/payload** data, and the ULID is a `target_id` value. **No new column on the foundation `ops`
table is required** (respecting the additive, old-binary-safe migration rule, platform §4.3): the
foundation op shape (`crates/foundation/src/model.rs`) is untouched; only chat's own views carry
these fields.

## 3. CRDT / op semantics on the shared log

All chat ops are foundation `Op`s (`op_id, lamport, site, domain, target_kind, target_id, field,
op_type, value, author, wall_clock`). The message reducer is `is_foldable` iff `domain == "message"`,
the `(target_kind, op_type, field)` triple is one of the shapes below, **and the op's `value` is
well-formed for that shape** (§3.1 spells this out for the `message` envelope); any other or malformed
shape is **store-don't-fold** (platform §4.2/§7) — kept in the log, never folded, revivable via
`refold` on a later build. Because `Reducer::fold` is **infallible by signature**
(`crates/foundation/src/reducer.rs`, no `Result`), **all** validation must live in `is_foldable`: a
malformed op returns `false` and is deferred store-don't-fold-safely rather than panicking the fold —
load-bearing because M1 sync is full-log/unfiltered (§6), so a single peer's bad op must never
poison-pill every replica that folds (or later `refold`s) it. Both local append (`emit`) and remote
merge (sync `apply`) funnel through the substrate's one `ingest`, so the fold rules below hold
identically for locally-authored and sync-received ops.

### 3.1 `message` — immutable, one op per message (grow-only)

| Verb | `target_kind` | `op_type` | `target_id` | `field` | `value` |
|---|---|---|---|---|---|
| post | `message` | `post` | `<message ULID>` | `envelope` | canonical JSON of §2.1 |

The whole message envelope rides in **one** op's `value` (canonical JSON, declared field order → a
byte-stable contract, like memory's `MemoryRecord`). Because a message is immutable there is **no
LWW** — the reducer folds it as a single **grow-only insert**:

```sql
INSERT OR IGNORE INTO messages(message_id, origin, channel_id, sender, kind, priority,
                               disposition, thread_id, refs, body, created, lamport, site)
VALUES (…parsed from the envelope…);
```

Idempotency is double-guarded: the substrate already unions the log on `op_id` (a re-delivered op is
`AlreadySeen`), and `message_id` is the `messages` PK (`INSERT OR IGNORE`). The envelope JSON is
opaque to the foundation — only the message reducer parses it.

**Envelope validity is enforced in `is_foldable`, not `fold`** (§3): a `message`/`post` op is foldable
only if `value` parses as JSON carrying every NOT-NULL `messages` column (`origin`, `channel_id`,
`sender`, `kind`, `priority`, `disposition`, `body`) with each enum (`kind`/`priority`/`disposition`)
in range. A malformed or incomplete envelope is **store-don't-fold** — deferred, never folded, never
able to crash the infallible `fold` or trip a NOT-NULL/`CHECK` constraint. Unknown *extra* envelope
fields are ignored (forward-compat); only a missing required field or an out-of-range enum defers the
op.

### 3.2 `channel` and `profile` — per-field LWW registers

Mutable metadata follows flow's item pattern exactly: **one op per field**, each field an
independent keep-if-beats LWW register versioned by the winning op's `(lamport, site)`.

| Entity | `target_id` | fields (`field`, `op_type=set`) |
|---|---|---|
| `channel` | `channel_id` | `name`, `kind` (`direct`\|`group`\|`public`), `origin` |
| `profile` | qualified handle | `job_title`, `job_description`, `capability_tags`, `runtime_binding`, `reports_to`, `origin` |

- **Channel ids.** A **group** channel gets a minted **ULID** (named, `n` members). A **DM** channel
  (exactly 2 members) gets a **deterministic** id: sort the two qualified handles, join them with a
  `\x00` separator, and take `dm:` + `hex(sha256(joined))[..24]` — so both agents derive the *same* DM
  id without a rendezvous (§9 TB-C7). Both carry `kind` and `origin`.
- **`public` — a third kind, added post-M1** (nxf 6j6v.bd6g, for app-foundations spec §8 D6.2): a
  project's FRONT DOOR, id-minted like a group channel, but READABLE by anyone in the workspace
  regardless of membership — the exception `facade::require_readable` makes to plain membership for
  `messages`/`thread`/`thread_board`. It is a read opening only: writes were never membership-gated
  (§5.1), and the one read state that WAS member-scoped, the read cursor, no longer exists at all
  (`6j6v.4d2z`, §3.4). This
  does NOT reopen §9's cross-tenant "no public channels / external consumers" non-goal (line under
  §8), which is about consumers OUTSIDE the workspace and still needs filtered sync (TB-C1); a
  `public` channel is intra-workspace and rides the same full-log sync as every other kind. `kind`
  stays an OPEN string on the wire — a reader that does not know a value round-trips it.
- **Profiles.** The identity is the qualified handle (`target_id`); `capability_tags` is a JSON
  array string (searchable). The **user is an agent** and the **root** of the org tree —
  `reports_to = NULL`; every other profile has exactly one supervisor via `reports_to` (vision
  lines 111–121). The org tree is *stored* in M1; deriving a default priority order from it (user >
  supervisor > peer > subordinate, vision line 119) is **presentation**, deferred to chat's
  presentation seam — a declarative **team template**, *not* a flow-style plugin (vision: "no plugin
  system in the core"; §9 TB-C9).

The fold is the standard keep-if-beats upsert (identical shape to memory §3 / flow `fold_item_lww`):

```sql
INSERT INTO <view>(id, <field>, <field>_v, <field>_site) VALUES (?id, ?value, ?lamport, ?site)
ON CONFLICT(id) DO UPDATE SET <field>=excluded.<field>, <field>_v=excluded.<field>_v,
    <field>_site=excluded.<field>_site
WHERE (excluded.<field>_v, excluded.<field>_site) > (<field>_v, <field>_site);
```

### 3.3 `membership` — observed-remove OR-set

Channel membership (join/leave, subscribe/unsubscribe) is an **observed-remove OR-set**, identical
to flow's edge OR-set (`edge_adds`/`edge_removes`): a member is present iff there exists an `add` op
not covered by a `remove` that observed it. This makes leave/rejoin reversible and convergent.

| `target_kind` | `op_type` | `target_id` | `value` | meaning |
|---|---|---|---|---|
| `membership` | `add` | `<channel_id>\|<handle>` | — | `handle` joins `channel_id`; the **add's `op_id` is its tag** |
| `membership` | `remove` | `<channel_id>\|<handle>` | SEP-joined observed **add-tags** | observed-remove: tombstones exactly the add-tags this remove saw |

This mirrors flow's edge OR-set **verbatim** (`crates/core/src/task_reducer.rs:72–91`,
`fold_edge_add`/`fold_edge_remove`): an `add` inserts its `op_id` as a *tag* into `membership_adds`; a
`remove` carries the observed add-tags **in its `value`** and tombstones each into
`membership_removes`. A handle is present iff it has a tag not in `membership_removes` — so a
concurrent add the remover **never saw** survives (true observed-remove, not remove-wins). The
remove's `value` is what makes this work; without it the view's `tag` column (§4) is unreachable.

**DM channels and "exactly 2".** A DM is *created* with its two members and the CLI enforces the count
**on create**. This is a **CLI-level, best-effort** guarantee — **not** a convergence-enforced
data-layer invariant, and deliberately so: rejecting a third `add` *at fold time* would make the fold
**order-dependent** (breaking convergence), so the reducer stays domain-pure and folds every
well-formed membership op. The **read side is defensive**: a `kind = direct` channel whose resolved
membership is not exactly 2 is treated as a **degraded** channel — `nxc` surfaces it as such and never
assumes/indexes "exactly 2" (a named T1 read-contract test, §10). Convergence to ≠2 can only arise
from a non-`nxc` client emitting raw ops; M1 tolerates it read-defensively rather than corrupting.

### 3.4 `read_cursor` — REMOVED 2026-09-08 (`6j6v.4d2z`)

A consumer's per-channel read position was a **synced** grow-only max-register (design decision, §9
TB-C3) — `read_cursor`/`set`, `target_id` `<consumer_handle>|<channel_id>`, `field` `seen`, folded
with `WHERE excluded.seen > seen` so the watermark never regressed. The whole apparatus is gone: the
op kind, the `read_cursors` table (§4), the `inbox` derivation it gated, the `mark_read` ack that
moved it, and the per-channel unread counts derived from it.

**Why.** Nothing consumed it. The two verbs that touched it left the CLI in `6j6v.1gm9` (0 uses
across 66 measured agent sessions each), no application ever called the ack over five layers of
seam, and the last derivation that still read the cursor — the session-start notice of finished
commissions — was REPLACED rather than deleted in `6j6v.2hx9`: it is derived from the operation now
(what finished since the caller's previous session ended), which needs no acknowledgement, because
"finished" does not become unfinished. The removal was gated on that replacement standing first.

**What it means for a peer.** The op kind is not re-used. A `read_cursor` op already in a synced log
stays in the log and is simply no longer foldable — the ordinary forward-compatible "store, don't
fold" treatment of a kind no reducer claims (§7) — so an older and a newer replica still converge on
everything they both understand. An existing workspace drops the table on its next open.

**The max-register argument is kept below, because it is the reason a WATERMARK is not an LWW
register and the next monotone register in this spec will want it.** A read watermark is
**monotone** — it must never regress. An LWW register keyed on the *op's* `(lamport, site)` would
let a later op that happens to carry an *older* value win and roll the watermark **back**.
`MAX(seen)` is a join-semilattice (max is associative, commutative, idempotent), so it converges
**order-independently** and never regresses — the same convergence discipline memory applies to
`forget` (memory §3.2), reached by a different lattice. Concretely:

```
The regression case is a later op (higher Lamport) carrying an EARLIER read position:
  A: set(seen = m9)@lamport 3     A read through m9…
  B: set(seen = m7)@lamport 5     …but B, only through m7, had its clock bumped past 3 by other work.
LWW-by-op keeps the higher (lamport,site) = B@5  → seen = m7   ← REGRESSION (m7 < m9, re-shows m8,m9).
max-register keeps max(m9, m7) = m9 in every merge order          ← never regresses. ✓
```

**And the defect it carried out with it** (`6j6v.1zew`): that comparison was lexicographic over the
ULID while message order is causal `(lamport, site, message_id)`, so within one millisecond the two
disagreed — a causally later message below the watermark was invisible for good, and an
acknowledgement of it was silently refused. Nothing in the store compares `message_id` on its own
any more.

### 3.5 `thread` — grow-only root + LWW `expects_reply_from`

A thread is opened once (grow-only root, like a message) and carries one mutable field — a mix of the
§3.1 and §3.2 shapes, so it gets its own fold and test (T1, §10) rather than being an instance of
another kind.

| `target_kind` | `op_type` | `target_id` | `field` | `value` |
|---|---|---|---|---|
| `thread` | `open` | `<thread ULID>` | `root` | canonical JSON `{ origin, channel_id, opener, created }` |
| `thread` | `set` | `<thread ULID>` | `expects_reply_from` | JSON array of handles |

`open` folds the immutable root into `threads` via an **upsert** (`INSERT … ON CONFLICT(thread_id)
DO UPDATE SET origin=…, channel_id=…, opener=…, created=…`) — not a bare `INSERT OR IGNORE`, so it
fills the root columns even when a `set expects_reply_from` folded first and created the row (root
values are identical across any duplicate `open`, so the unconditional set is order-independent). `set
expects_reply_from` is a keep-if-beats LWW register (§3.2 shape) on the
`expects_reply_from`/`_v`/`_site` columns. M1 only **stores** this — deriving reply-quorum /
`ThreadComplete` from it is **M2** (§9 TB-C5).

## 4. Views

Materialized tables per `.nxs/` db, folded by the message reducer (foundation owns the `ops` log +
schema-version spine; chat owns only this DDL). `_v`/`_site` columns carry LWW metadata where the
field is an LWW register.

```sql
-- Immutable message log (grow-only). Ordered by (lamport, site) = append order; message_id is ULID.
CREATE TABLE IF NOT EXISTS messages(
    message_id  TEXT PRIMARY KEY,       -- ULID
    origin      TEXT NOT NULL,
    channel_id  TEXT NOT NULL,
    sender      TEXT NOT NULL,          -- qualified handle
    kind        TEXT NOT NULL,          -- task|question|report|decision|info
    priority    TEXT NOT NULL,          -- urgent|normal|background
    disposition TEXT NOT NULL,          -- in_turn|next_session
    thread_id   TEXT,                   -- ULID or NULL
    refs        TEXT,                   -- JSON: {session_id?, nxf_ids?, branch?, pr?}
    body        TEXT NOT NULL,          -- Markdown
    created     TEXT,                   -- wall-clock, display-only
    lamport     INTEGER NOT NULL,
    site        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_channel ON messages(channel_id, message_id);
CREATE INDEX IF NOT EXISTS messages_thread  ON messages(thread_id);

-- Channels (per-field LWW).
CREATE TABLE IF NOT EXISTS channels(
    channel_id TEXT PRIMARY KEY,
    name       TEXT, name_v INTEGER DEFAULT 0, name_site INTEGER DEFAULT 0,
    kind       TEXT, kind_v INTEGER DEFAULT 0, kind_site INTEGER DEFAULT 0,   -- direct|group|public
    origin     TEXT, origin_v INTEGER DEFAULT 0, origin_site INTEGER DEFAULT 0
);

-- Channel membership (observed-remove OR-set, mirroring flow's edge view). tag = the add op's op_id;
-- a handle is present iff it has a tag not tombstoned in membership_removes.
CREATE TABLE IF NOT EXISTS membership_adds(
    tag        TEXT PRIMARY KEY,          -- the add op's op_id
    channel_id TEXT NOT NULL, handle TEXT NOT NULL,
    lamport    INTEGER NOT NULL, site INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS membership_removes(
    tag TEXT PRIMARY KEY                  -- an observed add-tag this remove tombstoned (from the remove op's value)
);
-- resolved membership = membership_adds WHERE tag NOT IN (SELECT tag FROM membership_removes),
-- grouped by (channel_id, handle) — the OR-set read, mirroring flow's edge_adds/edge_removes.

-- Agent/user profiles (per-field LWW). Handle is the identity.
CREATE TABLE IF NOT EXISTS profiles(
    handle          TEXT PRIMARY KEY,
    job_title       TEXT, job_title_v INTEGER DEFAULT 0, job_title_site INTEGER DEFAULT 0,
    job_description TEXT, job_description_v INTEGER DEFAULT 0, job_description_site INTEGER DEFAULT 0,
    capability_tags TEXT, capability_tags_v INTEGER DEFAULT 0, capability_tags_site INTEGER DEFAULT 0,
    runtime_binding TEXT, runtime_binding_v INTEGER DEFAULT 0, runtime_binding_site INTEGER DEFAULT 0,
    reports_to      TEXT, reports_to_v INTEGER DEFAULT 0, reports_to_site INTEGER DEFAULT 0,
    origin          TEXT, origin_v INTEGER DEFAULT 0, origin_site INTEGER DEFAULT 0
);

-- Threads (root + LWW envelope). Quorum/ThreadComplete derivation is M2 (§9 TB-C5).
CREATE TABLE IF NOT EXISTS threads(
    thread_id         TEXT PRIMARY KEY,   -- ULID
    -- Root columns are NULLable so a `set expects_reply_from` that folds BEFORE its `open` still
    -- converges (order-independence, §3.5); `open` fills them.
    origin            TEXT,
    channel_id        TEXT,
    opener            TEXT,               -- qualified handle
    created           TEXT,
    expects_reply_from TEXT,              -- JSON array of handles, may be NULL
    expects_reply_from_v INTEGER DEFAULT 0, expects_reply_from_site INTEGER DEFAULT 0
);

-- `read_cursors(consumer, channel_id, seen)` — the synced per-(consumer,channel) read watermark —
-- stood here. REMOVED 2026-09-08 (`6j6v.4d2z`, §3.4); an existing workspace drops it on open.
```

**Inbox derivation — REMOVED 2026-09-08 (`6j6v.4d2z`).** It was a pure read over the views, for a
consumer handle `C`:

```
channels(C)   = { ch : membership resolves C present in ch }
unread(C, ch) = { m ∈ messages : m.channel_id = ch AND m.message_id > coalesce(seen(C,ch), '') }
inbox(C)      = ⋃ over channels(C) of unread(C, ch)
```

`nxc inbox` (§5) partitioned `inbox(C)` by `disposition`; the cursor was advanced only by an
explicit `read`/ack (§3.4), never by a read, so a crash mid-processing re-surfaced the message
(**at-least-once + idempotent handlers**, the vision's day-one rule).

> **The two verbs went first (2026-08-27, `6j6v.1gm9`), the derivation followed (2026-09-08,
> `6j6v.4d2z`).** The verbs went because the pull premise below (§7) turned out not to describe how
> an agent is actually reached: `send` starts a session with the body in its prompt, `reply` resumes
> the target with the reply's body, and a completed quorum wakes the opener. All three PUSH, and
> both verbs measured 0 uses across 66 role sessions. The DERIVATION survived them for a fortnight,
> travelling on `nxc prime --json` as `in_turn`/`next_session`/`count` with `Engine::mark_read`
> advancing the cursor for an app that never called it, and then went too. **What replaced the
> at-least-once promise is not another pull**: a message is delivered by the session it starts or
> resumes, an answer that arrives while its requester is busy is HELD and delivered once that
> session settles (`6j6v.gn8b`), and a session start names what finished while it was away
> (`6j6v.2hx9`). The retry is the wake, not the watermark.

## 5. Verb surface (`nxc`)

Binary **`nxc`** (an `argv[0]` persona of the one `nxs` multicall binary, like `nxm`); `--json`
deterministic everywhere (stable field order) + human output. The CLI is a **pure pull consumer** of
the same views the engine derives — it embeds the engine directly, identical to how an app would
(architecture decision `6j6v.fnn1` §3).

### 5.1 Messaging verbs

| Verb | Behavior |
|---|---|
| ~~`nxc send <channel> "<body>" [--kind K] [--priority P] [--disposition D] [--thread <id>] [--ref k=v]…`~~ | append a `message` op; return a **synchronous persist-ack** the instant it is durably logged (`{message_id, persisted:true}`) — no wait for delivery (decision §2a). `--kind` default `info`, `--priority` default `normal`, `--disposition` default `in_turn`. **The POSITIONAL channel was REMOVED 2026-08-19 (`6j6v.dvyq` §3)** with the raw channels: a target is named by `--to` and resolved against the declarations, so `nxc send --to <persona\|channel> "<body>"` is the shape, and there is no plain post to a channel nothing declares. `--thread` went earlier in the same §3. `--deadline` ARRIVED here from `ask` (below). |
| ~~`nxc reply <thread_id\|message_id> "<body>" […]`~~ | convenience over `send`: post into the same thread/channel, carrying `thread_id`. **The POSITIONAL target was REMOVED 2026-08-19 (`6j6v.dvyq` §3):** `nxc reply --thread <thread_id> "<body>"` is the one address a conversation has, and a message id — which meant "the conversation this message is in" — is no longer a second one. **`Engine::reply` — which still resolved either — was REMOVED on 2026-08-21 (`6j6v.ckeq`):** the seam takes a thread and nothing else, so an app holding a message id reads its thread first (`Engine::thread`/`status` both carry it — the two reads this line named until `6j6v.yr59`, `Engine::messages` and `thread_board`, went in that cut). Recorded as a named loss rather than a collapse; `orchestration::reply` still resolves both, and is what `Engine::reply_thread` runs. |
| ~~`nxc inbox [--consumer <handle>] [--all] [--channel <id>]`~~ | the unread set `inbox(C)` (§4), **interpreting the disposition**: default returns **`in_turn`** unread (act now); `--all` includes `next_session`. Pure read — does **not** advance the cursor. **REMOVED 2026-08-27 (`6j6v.1gm9`)**, together with the `prime` block it fed: an agent does not ask after its own messages, it is handed them by the session that starts or resumes it, and the verb measured 0 uses across 66 role sessions. A HUMAN reads the conversation instead (`nxc threads show`, `nxc status`). The compute half stayed for a fortnight — `facade::inbox` was what `prime` read, and the record it produced was `PrimeReport::in_turn`/`next_session` — and went with the whole apparatus on 2026-09-08 (`6j6v.4d2z`, §3.4). |
| ~~`nxc read <channel> --through <message_id> [--consumer <handle>]`~~ | advance the **synced** read cursor (emit a `read_cursor` op, §3.4). The explicit ack that makes delivery at-least-once. **REMOVED 2026-08-27 (`6j6v.1gm9`)** with `inbox`, and it was the brake that never engaged: the one thing it bounded was the "Threads you opened" block of M2 §5.2, and it too measured 0 uses across 66 role sessions. `facade::mark_read` and the `read_cursor` op-kind outlived it on the argument that an app rendering a conversation is who acks it — measured, no app ever did, and both went on 2026-09-08 (`6j6v.4d2z`, §3.4). |
| ~~`nxc channels [list] \| create <name> \| dm <handleA> <handleB> \| join <ch> <handle> \| leave <ch> <handle>`~~ | list channels; create a group channel; open a DM (exactly 2 members); membership add/remove (OR-set, §3.3). **REMOVED 2026-08-19 (`6j6v.dvyq` §3)** — the whole group. A channel is DECLARED (`.nxs-personas/channels.yaml`) and materialises on the first `send --to`; membership is that declaration's `members:`; a direct conversation opens itself on `send --to <persona>`; `list` is `nxc list`. `channels public` — the cross-project discovery read (6j6v.bd6g) — has NO CLI successor and that is a decision, not an oversight: a front door reaching this workspace by sync carries no declaration here for `list` to read, so discovery leaves the agent surface and stays on the seam. **`6j6v.yr59` (2026-08-21) then FOLDED that read into `Engine::directory`**, which carries the front doors — including synced ones no declaration here names — beside the declared personas and channels, so learning everything addressable is ONE call; `Engine::public_channels` and `Engine::channels` are both off the handle, and the per-channel UNREAD counts `channels` carried are a named loss with no successor (`crates/chat/tests/seam_disposition.rs` carries the reasons). |
| ~~`nxc agents [list] \| register <handle> [--title T] [--desc D] [--tags …] [--reports-to H] \| search "<q>"`~~ | profiles: list/register/update; **`search`** ran a case-insensitive substring match over `job_title` **and** `job_description` via a bound `LIKE ?` parameter (vision line 113). **REMOVED 2026-08-17 (`6j6v.dvyq` §3)** — a team is DECLARED (`.nxs-personas/<handle>.yaml`) and `nxc list` is the read over it, carrying both of those fields. The profile table and its store reads remain for ops already in the log. |
| `nxc search "<q>"` | case-insensitive substring over message `body` in the caller's channels, via a bound `LIKE ?` parameter (no string interpolation), deterministically ordered. |

`--consumer` defaults to the caller's own handle (resolved from `NXC_ACTOR`→`USER`, mirroring
`nxm`); a message/reply to an unknown channel, or a channel read for a non-member, is a `not_found`/
`forbidden` error (agent-ergonomic, pattern: flow's `require_item`) — the store layer stays pure (it
emits the op), the CLI checks membership for the friendly message.

### 5.2 Contract verbs (parity with `nxf`/`nxm`, platform §6)

| Verb | Role |
|---|---|
| `nxc init` | delegate the foundation to `foundation::setup()`, activate `chat` in `active_modules`, and delegate agent-file assembly + the SessionStart wiring (one hook per active module, → `nxc prime` for chat's own; a single hook → `nxs prime` until nxf n2m6 + a2a1) to the shared `nxs-init` assembler (like `nxm init`). |
| `nxc agent-manifest --json` | declare chat's agent contribution as data: `{"prime_command":"nxc prime","hook":{"event":"SessionStart","command":"nxc prime"}}` (manifest contract, platform §6.2). |
| `nxc prime` | the SessionStart context block: **pin the chat rule** (agent-to-agent coordination goes through `nxc`, not ad-hoc), name the core verbs, and render **all unread** for the session agent — **both** dispositions (`in_turn` *and* `next_session`), because a new session is the catch-up moment. Deterministic, channel+ULID-sorted output. |

Registration is `inventory::submit!{ nxs_init::ModuleInit { key:"chat", binary:"nxc",
now_env:"NXC_NOW", order:30, accepts_plugin:false, init_fn: chat_init_entry, … } }`. The **only**
manual `nxs`-side change a new product needs (foundation/agent-3 mapping): the load-bearing link
`use nexus_chat as _;` in `crates/nxs/src/main.rs` and the two routing arms (`Some("nxc")`,
`Some("chat")`) in `crates/nxs/src/cli.rs`. Everything else — chooser, prime fan-out, agent-file
assembly — flows from the `inventory` registration.

## 6. Delivery model — pull-only, sync-as-bus, no daemon

M1 delivery is **pure pull** (architecture decision `6j6v.fnn1` §6): a consumer learns of new
messages only when *it* pulls. The two pull triggers are **sync-on-demand** (`nxs sync`, the
foundation-only sync over the shared log — platform §6.3) and the **SessionStart/Stop hooks**
(`nxs prime` fans out to `nxc prime` at session start; a Stop hook may sync at session end — it
could `nxc read` too until `6j6v.1gm9` removed that verb, see §5.1). There is **no standing background service** (contrast the vision's daemon, superseded — §9
TB-C6).

- **Synchronous half:** `nxc send` appends the op and acks on persistence (§5.1). Done.
- **Asynchronous half:** the message reaches consumers via the **shared sync** — *sync is the bus*.
  A remote write arrives via sync → bumps `PRAGMA data_version` → the foundation `Watcher` (in-
  process, `crates/foundation/src/watch.rs`) fires a coalesced `Change`.
- **Latency / cost trade-off (named, per the ticket):** with no daemon, a peer's message becomes
  visible to a consumer only at the consumer's **next** pull — bounded by hook cadence (per session)
  or an explicit `nxc sync`/cron. The trade is **zero standing cost** (no process, no socket, no
  battery) against **non-instant** delivery. For M1's interactive + embedded agent modes (turn- and
  session-scoped) this is the right default; sub-second push is the app layer below.

**The app layer (described, not an M1-CLI obligation).** A long-lived app builds *event management*
on top: it holds an `Engine`, calls `Engine::subscribe()` once, and on each `Change` **re-reads the
views** and **fans out** to its per-channel subscribers. This is the "push" tier. It is an **app
concern**, not the engine and not a daemon (decision §3). Two consequences are explicitly **open**,
not M1 work: the foundation `Watcher` is **domain-blind** (whole-db `data_version`, "someone wrote")
— per-channel routing is the app layer's re-read filter today; whether the foundation needs a
kind-aware watch is deferred (§9 TB-C2). The M1 CLI never holds a watcher — it is one-shot pull.

## 7. Multi-module coexistence

`nxf`, `nxm` and `nxc` share **one** `.nxs/` workspace: `task`, `fact` and `message` ops coexist in
**one** op-log; each reducer folds **only** its domain (foreign domain → *store-don't-fold*, no
cross-contamination — platform §4.2/§7). chat opens **its** store over the same `db.sqlite`,
registers `MessageReducer` **imperatively at open** (`ChatStore::open` →
`register_reducer(Box::new(MessageReducer))`, watermark key `"chat"`, `refold_if_behind` — the
memory-store recipe verbatim), and materializes **its** views. The views are durable, so no refold
is needed on a normal reopen; `refold_if_behind("chat")` covers the case where a foundation-only
`nxs sync` advanced the shared log with `message` ops without folding chat's views.

**Behavioral invariant:** flow **and** memory stay **byte-identical** — the differential oracle and
the flow/memory trycmd goldens remain unchanged and green. `message` ops do not change flow/memory,
because their reducers never fold a `message` op and the foundation runs additive, old-binary-safe
migrations (platform §4.3). Sync carries `message` ops over the wire with their `domain` preserved
(the wire already carries `domain`, memory §6) and folds them into the peer's chat views — messages
**synchronize** across a workspace's replicas rather than fragmenting per machine.

## 8. Deliberate non-goals (M1)

- **No public channels / external consumers.** M1 channels are private (invite / repo membership).
  Public channels (anyone may subscribe — e.g. library-update announcements) need multi-tenant
  identity + ACL and come **later** (vision; decision consequences).
  **Partly delivered post-M1, and the line is worth drawing precisely** (nxf 6j6v.bd6g): a
  `public` channel KIND now exists (§3.2) — readable and addressable by anyone **in the same
  workspace**, no membership needed. What stays out is the half this non-goal is really about:
  a consumer OUTSIDE the workspace, which still needs the multi-tenant identity + ACL above and
  the filtered sync of the next bullet. Intra-workspace, no new identity system was required —
  every reader is already a peer on the same full log.
- **No filtered sync in M1.** M1 is intra-workspace: every internal repo agent shares the **full**
  log. Kind- + channel-scoped sync (needed only once external/public consumers exist) is a
  follow-up (§9 TB-C1).
- **No agent runtime, no workflow engine.** Managed-mode agent lifecycles and the reply-quorum
  workflow runner need the standing process this design avoids (§9 TB-C8).
- **No quorum derivation.** M1 stores `thread` + `expects_reply_from`; deterministic reply tracking
  and the `ThreadComplete` wake are **M2** (§9 TB-C5).
- **No presentation layer.** Org-derived priority ordering, vocabulary labels and ranking are a later
  **presentation** concern — for chat the seam is a declarative **team template**, *not* a flow-style
  plugin system (vision: "no plugin system in the core"; §9 TB-C9). The M1 engine keeps
  `kind`/`priority`/`disposition` as opaque enums.

## 9. Open points / tie-breaker log

| ID | Question | Resolution |
|---|---|---|
| TB-C1 | Does M1 need filtered (per-kind / per-channel + ACL) sync? | **RESOLVED (2026-07-08): out of M1.** M1 is intra-workspace, full-log; domain isolation at fold already prevents cross-contamination. Filtered sync = **own follow-up ticket** — it reopens platform §4.3's "selective per-kind sync is YAGNI" and needs per-kind/per-channel cursors (replacing the scalar `pushed/pulled_through`), a relay that authorizes by channel membership, and subscription params on the wire. Prerequisite for public channels / external consumers, not for M1. |
| TB-C2 | Does the foundation need a finer (kind-/channel-aware) `Watch` signal than whole-db `data_version`? | **Deferred (foundation TB-8).** M1 rides the domain-blind watcher; the app layer filters by re-reading its channel views on any `Change`. Not an M1-CLI concern (the CLI holds no watcher). Revisit with the app/push tier. |
| TB-C3 | Where does inbox read-state live — local, or in the synced substrate? | **RESOLVED (2026-07-08): synced.** `read_cursor` was a folded, syncing `message`-domain kind (§3.4) — a **grow-only max-register** per `(consumer, channel)`, so a consumer's read position converged across its devices/replicas and never regressed. Heavier than a local watermark by one target_kind, chosen for multi-device consistency. **MOOT since 2026-09-08 (`6j6v.4d2z`): there is no inbox read-state.** The question was answered on the assumption that read-state exists; delivery turned out to be a PUSH, and the answer is now "nowhere". |
| TB-C4 | Global id form for federation forward-compat? | **RESOLVED (2026-07-08): ULID + `origin` + qualified handles** (§2.2), carried as **product-data** — **no** foundation op-schema change. ULIDs are collision-free across workspaces by construction; `origin` + `<origin>/<agent>` handles give the bridge attribution with zero migration. In M1 `origin`/handles are **unauthenticated** — binding them to an authenticated peer identity is a required part of the bridge contract (`6j6v.kz8p`). |
| TB-C5 | Thread reply quorum / `ThreadComplete` derivation? | **M2.** M1 stores `thread` + `expects_reply_from` as data (§4); deterministic reply tracking and the requester-wake are the next milestone. |
| TB-C6 | The vision's daemon + "no CRDT / no shared sync / own substrate" stance. | **Superseded.** The "no CRDT / no shared sync / own substrate" passages (vision lines 281–289, 316–319) are overturned by platform D2/D3 (shared kind-tagged op-log, one shared sync). The **daemon** (vision lines 72–79) is overturned by architecture decision `6j6v.fnn1` (engine passive, event-management in the app layer, no standing service) — a later decision than the platform spec (whose §9-P4 line still mentions "daemon/event loop"). This spec follows `fnn1`. |
| TB-C7 | How do two agents address the *same* DM without a rendezvous? | **RESOLVED (2026-07-08): deterministic DM id** = `dm:` + hash of the two sorted qualified handles (§3.2). Group channels get a minted ULID. The exactly-2 rule is **CLI-best-effort, not fold-enforced** (enforcing at fold would make the fold order-dependent); the read side treats a `direct` channel with ≠2 resolved members as degraded (§3.3). |
| TB-C8 | Managed agent runtime + workflow engine (which need a standing process). | **Deferred, separate epics.** They need exactly the daemon this architecture removes; the interactive + embedded (hook/pull) modes carry M1. Cross-workspace bridging (the removed daemon's other role) is `6j6v.kz8p` (manufakt coordinator). |
| TB-C9 | Chat's presentation/vocabulary seam — a flow-style plugin? | **RESOLVED (2026-07-08): no plugin system in core.** Per the vision ("no plugin system in the core"; team templates are the "plugin equivalent"), chat's presentation/vocabulary/priority-ordering seam is a declarative **team template** (`nxc init --team`, post-M1), not a `PluginConfig`-style plugin. M1 keeps message attributes as opaque enums. Corrects earlier "chat plugin" phrasing. |
| TB-C10 | Sanitization of untrusted `body`/`refs` for the later frontend. | **Deferred — tracked here.** `body` (Markdown) and `refs` are agent/user-authored, untrusted; M1 stores them **verbatim**. Escaping/sanitization (XSS/injection at render, a ref-scheme allowlist) is a **frontend-milestone obligation**, out of M1 — logged so it is tracked, not implied by omission. |
| TB-C11 | Retention / pagination / size limits for the grow-only `messages` log and `search` (and, until `6j6v.4d2z`, `inbox`). | **Deferred (rides TB-C1).** The message log is append-only and unbounded; M1 sets no retention, no page size, no body cap. Pagination and a retention/compaction policy land with the filtered-sync/scale follow-up (TB-C1) — named so the unboundedness is a known accepted M1 cost. |

## 10. Mapping to implementation tickets

The M1 implementation tickets are **derived from** this spec (created under epic `6j6v.m4xe`,
depending on `6j6v.3cz4`):

| Ticket | Delivers | This spec |
|---|---|---|
| **T1** | `nexus-chat` crate skeleton + `MessageReducer` + the six views, store wiring (`ChatStore::open`, watermark `"chat"`, refold-when-behind), and the convergence/robustness tests: grow-only message; per-field LWW; **thread** fold (§3.5); membership **observed-remove** (a concurrent add the remover never saw survives — not remove-wins); read-cursor **max-register regression** (a higher-Lamport op carrying a lower cursor must not roll it back, §3.4); **malformed-envelope store-don't-fold** (a bad `message` op defers, never panics `fold`, §3.1); the **DM degraded-read contract** (`direct` with ≠2 members, §3.3); plus the store-don't-fold cross-domain net | §2, §3, §4, §7 |
| **T2** | `nxc` messaging verbs: `send` (persist-ack), `inbox` (disposition interpretation), `reply`, `read`/ack (synced cursor), `channels` (create/dm/join/leave), `agents` (register/search), `search` — all `--json`, deterministic, **including the `not_found`/`forbidden` failure paths** (§5.1). *(The task as it was delivered; `inbox`/`read`/`channels`/`agents` have since been removed — §5.1 carries each one's date and reason.)* | §2.1, §5.1, §6 |
| **T3** | `nxc` contract verbs `init`/`agent-manifest`/`prime` + the `ModuleInit` self-registration + the `nxs` link line & routing arms; the `prime` catch-up block | §5.2, §6 |
| **T4** | multi-module coexistence + sync round-trip (a `message` op survives the relay and folds on a peer) + the M1 gate (flow & memory stay byte-identical) | §6, §7 |

Acceptance of the milestone as a whole is the 2-agent dogfood handoff (`6j6v.cdy7`) — a follow-up
that exercises T1–T4 end to end, not part of this spec.
