# RFC 0005: Goals in NoCentralDB

Status: Draft

This RFC designs the `goals` module of `factory/GENESIS.md`, "Goals set the focus": how the goals a group of repositories focuses on are stored in its coordination space, how a ticket in a repository space names the goals it serves, how pm reads "the tickets that serve this goal" from its own local database, the rule by which goals in focus rank a repository's work, and what v1's priority field becomes once they do. It builds on RFC 0001, which places goals in the coordination space, and works with RFC 0002 (tickets) and RFC 0009 (work across repositories). Once it is decided, a coding ticket can build the module: the `goalsGoal` collection in nexus-flow's model, the goal reference on RFC 0002's `ticketsReference`, the `nxs goal` commands, the goal filters and the focus key of `nxs ticket`, and their counterparts on the library seam.

## What it solves

The owner directs agents across several repositories, and wants them to work on what matters now without re-prioritising dozens of tickets whenever that changes. GENESIS.md, "Goals set the focus", asks for:

- **A goal spans repositories.** "Win 20 paying customers for nexflow.it" touches the repository that integrates the billing provider, the one that runs the waiting list, and the product itself.
- **A ticket serves several goals, or none yet.** pm ranks the work by the goals in focus, and no priority field on dozens of tickets needs updating when a goal changes.
- **Work for a goal goes first.** Agents finish what they are working on, but take on new work that serves a goal in focus before other new work.
- **Goals live in the coordination space,** and each repository's pm ranks its own tickets against them.
- **A new goal starts a re-evaluation:** a model proposes which tickets serve it, and the owner confirms.

NoCentralDB shapes how: rights apply to a whole space, a link points only into its own space (NoCentralDB RFC 0004, "Links"), every query runs through an index the model declares (NoCentralDB RFC 0005, "Indexes"), and nothing computed may follow a reference into another space (NoCentralDB RFC 0004, "How a reference resolves"). A goal lives in the coordination space and a ticket in a repository space, so the ticket names the goal through a reference, and every rule that reads both is nexus-flow's.

**Scope.** This RFC designs the goal's storage, its commands and JSON, the reference from a ticket to a goal, and the ranking rule. The re-evaluation when a goal is added is behaviour for priority 2 (decision of 2026-11-15, "Priorities: migrate to NoCentralDB, then build the new factory, then the rest"): this RFC stores only what it needs (see "What the re-evaluation stores"), and its flow is designed later by rewriting this RFC.

Out of scope, each in its own RFC: the ticket collection, its `ticketsReference` collection, readiness and the `next` order this RFC adds a key to (RFC 0002); the coordination space's repositories, requests and the goal a request serves (RFC 0009); what the daily note says about goals (RFC 0007). NoCentralDB's own design is in nocentraldb/nocentraldb, `docs/rfc/`, cited here as "NoCentralDB RFC" by number and section; an RFC number without that prefix is one of nexus-flow's own.

## The design

### Where goals live

Goals live in the coordination space (RFC 0001, "Spaces"). Every repository of the group reads the same goals from its own copy, and every pm there may write them (RFC 0001, "Roles in a space"). The storage crate refuses to write a goal into a repository space (RFC 0001, "The model").

**A repository that belongs to no group** has no coordination space, and therefore no goals. A solo developer who wants goals creates a coordination space of one repository with `nxs space create-coordination` (RFC 0009, "Creating it"); `nxs goal add` without one refuses with `no_coordination_space` and names that command. When other repositories join later, the goals are already where they belong, and nothing moves. Keeping goals in the repository space while it has no group was not taken: joining a group later would need a migration of every goal and every reference to it. Whether that is right for a solo developer is question 5.

### The goal

A goal is one object in the collection `goalsGoal`:

| Field | Type | Holds |
| --- | --- | --- |
| `title` | `nc.string()` | The goal in one line, such as "Win 20 paying customers for nexflow.it" |
| `description` | `nc.string()` | Why it matters, Markdown; optional |
| `reachedWhen` | `nc.string()` | What "reached" means, as a check a person can make, such as "20 customers pay a monthly plan"; optional |
| `status` | `nc.oneOf(["proposed", "focus", "later", "reached", "dropped"])` | Where the goal stands (see "Status and focus") |
| `statusAt` | `nc.string()` | Since when it has had this status, RFC 3339 UTC |
| `statusComment` | `nc.string()` | Why the status last changed, if a reason was given |
| `owner` | `nc.string()` | Who answers for the goal: an identity, `account:<fingerprint>` or `member:<member id>`, as RFC 0002's `assignee`; optional |
| `createdAt`, `createdBy`, `statusBy` | `nc.string()` | When and by which member it was created, and which member set the status; RFC 0001's author stop-gap, until R6 |

Every field may be unset, as every NoCentralDB field may (NoCentralDB RFC 0005, "No required fields and no defaults"); `nxs` writes `title`, `status`, `statusAt` and the author fields on every goal it creates, and reads a goal with no `status` as `later`.

**One status, not a focus flag beside a lifecycle.** "In focus or not" and "reached or not" are one field, so a goal cannot be in focus and reached at the same time, and "since when" is one timestamp. *Not taken:* a boolean `inFocus` beside a status (two fields that can contradict each other after a concurrent write, since each merges on its own; NoCentralDB RFC 0004, "Basic values").

**No ticket list on the goal.** A goal does not hold the tickets that serve it: they live in other spaces, a field holds one reference, and NoCentralDB has no field of many references (NoCentralDB RFC 0005, "References into another space"). The ticket names the goal instead (see "A ticket serves a goal").

**A goal is never deleted.** `nxs` offers no command that deletes one: a goal that no longer matters is `dropped`. References to it from tickets and the `goal` link of RFC 0009's requests therefore stay resolvable, and a ticket never names a goal that reads `missing` because someone tidied up.

**No order among goals in focus.** Every goal in focus counts the same; within the work for goals in focus, a ticket's own priority orders (see "Ranking"). An explicit order among goals was not taken, since GENESIS.md asks only that work for a goal goes first, and an order is a second thing to keep current; question 2 asks whether the owner wants one.

**The index.** `byStatus` on `status` and `statusAt`, with `equal: 1` and `scope: "space"`, gives the goals of one status, longest-standing first, in one lookup (NoCentralDB RFC 0005, "Indexes"). Every goal read runs through it or reads one goal by its id (NoCentralDB RFC 0009, "Get by id").

**Goal ids.** A goal is named by its NoCentralDB object id, and `nxs` accepts any prefix of at least 8 hexadecimal digits that names one goal in the coordination space, as RFC 0009 does for requests (RFC 0009, "Request ids"). A readable form follows whatever RFC 0002 and RFC 0009 decide for tickets and requests (question 7).

### Status and focus

| Status | Means |
| --- | --- |
| `proposed` | An agent suggested it; a person has not yet decided whether the group pursues it |
| `focus` | In focus: work that serves it goes first |
| `later` | The group pursues it, but not now |
| `reached` | What `reachedWhen` names is true |
| `dropped` | No longer pursued |

**Only a person puts a goal into focus.** What is in focus decides what every agent of the group takes on next, so it is the owner's lever (GENESIS.md, "Goals set the focus", and principle 1). `nxs` therefore refuses to set a status other than `proposed` when the member it runs as is an agent or a sub-agent (NoCentralDB `MemberKind` `agent` or `subAgent`; RFC 0001, "Members"), with the error `goal_status_needs_a_person`. A goal an agent adds is `proposed`; a goal a person adds is `later`, or `focus` with `--focus`. A person may move a goal from any status to any other.

As with RFC 0009's request statuses, the rule binds `nxs` and the library seam only: rights apply to a whole space, so a writer of the coordination space that writes through another client can set any status (NoCentralDB RFC 0008, "Roles and rights"). `statusBy` shows which member set the status, until R6 as a field the writer writes. Whether agents may change the focus too is question 1.

Two members who set the status at the same time write two values; the last writer wins (NoCentralDB RFC 0004, "Basic values"), and `nxs` reports the goal with `conflict: true` until the next status change (NoCentralDB RFC 0009, "Conflicts, lost removals and link states").

### A ticket serves a goal

A ticket in a repository space cannot link to a goal in the coordination space (NoCentralDB RFC 0004, "Links"). It names it through RFC 0002's `ticketsReference` object of kind `serves`, whose `target` is an `nc.ref` (RFC 0002, "References out of the space"). This RFC adds `goalsGoal` to that field's targets, an additive change (NoCentralDB RFC 0005, "The additive-change rule"):

| Field | Type in RFC 0002 | With this RFC |
| --- | --- | --- |
| `target` | `nc.ref(["ticketsTicket"])` | `nc.ref(["ticketsTicket", "goalsGoal"])` |
| `kind` | `nc.oneOf(["serves", "cites"])` | `nc.oneOf(["serves", "cites", "proposed"])` (see "What the re-evaluation stores") |

A ticket serves as many goals as it has `serves` references to goals, or none. `label` holds the goal's title when the reference was written, for a reader who cannot resolve it (RFC 0002, "References out of the space").

**Writing one.** `nxs ticket ref <key> --serves <goal>` (RFC 0002) accepts a goal id, or a prefix of one, in the coordination space `nexus-flow.toml` names, resolves it there, and writes the reference with the goal's title as `label`. `--remove` deletes it. RFC 0009's `nxs request link` writes the same reference when the request names a goal and the ticket serves none (RFC 0009, "From a request to tickets"). `nxs` refuses a second `serves` reference from one ticket to the same goal; two written concurrently on two members are both kept and read as one.

**Only the current coordination space counts.** A `serves` reference whose space is not the coordination space `nexus-flow.toml` names, for example one written before the repository left a group (RFC 0009, "Repositories in the space"), is shown with `current: false` and never counts for focus.

#### Reading "the tickets that serve this goal"

pm reads it locally, in one database. A member's database holds every space it belongs to (RFC 0001, "The local data on disk"; NoCentralDB RFC 0006, "Where the view lives"), so pm's holds both the coordination space and its repository space:

1. The goals in focus: `byStatus` with `status = "focus"` in the coordination space.
2. The tickets that serve one goal: RFC 0002's index `byTarget` on `ticketsReference`, with `target` set to the goal's space id and object id and `kind = "serves"`, in the repository space. NoCentralDB indexes a reference as its space id and object id, whatever its state, so this lookup is the same on every member of the repository space and needs no copy of the goal (NoCentralDB RFC 0005, "References into another space").
3. The goals one ticket serves: RFC 0002's `byTicket` on `ticketsReference`, then each target read by id in the coordination space.

Neither read crosses the network or follows a reference inside NoCentralDB: nexus-flow reads the goal ids in one space and looks them up in the other. A goal's own view, `nxs goal show`, puts both together, with the requests that serve the goal from RFC 0009's `byGoal` index.

**Only this repository's tickets.** A pm is a member of its own repository space only (RFC 0001, "Roles in a space"), so it sees which of its own tickets serve a goal, never another repository's. A view of a goal across every repository needs a member of every repository space: the owner's devices are, and manufakt.io, as a device of the owner's account, can read each space and add them up (RFC 0001, "Members"). This RFC stores no roll-up in the coordination space: a count written by each pm would be derived state that drifts from the tickets it counts (RFC 0001, "Derived state").

#### A member who reads one space but not the other

A reference resolves per reader to `ok`, `notAccessible` or `missing` (NoCentralDB RFC 0004, "How a reference resolves"), so what a member sees of goals depends on the spaces it holds:

| Member | Holds | Sees |
| --- | --- | --- |
| pm | Both spaces | Every goal, and for its own tickets which goals they serve, with the goal's status. Ranks by focus. |
| The owner's devices, manufakt.io | Every space | As pm, for every repository |
| A role session (coder, legal, …) | The repository space only | Its ticket's goal references as `not_accessible`, each with its `label`; no goal status, so `focus` is `null` (see "Ranking"). It is handed its ticket by pm, which ranked it, and needs no more. |
| A developer device in the coordination space, but not in a repository's space | The coordination space only | The goals and the requests, but no ticket of that repository |
| Any member, for a goal not yet synced | Both spaces | The reference as `missing`, with its `label`, until the goal arrives |

A reference to a goal that reads `not_accessible` or `missing` never counts for focus, for that reader. Nothing derived from a reference's state is stored (NoCentralDB RFC 0005, "References into another space").

### Ranking

GENESIS.md: "work for a goal goes first", and agents "may finish what they are working on, but they take on only the new work identified as important for the goal." This RFC adds one derived value per ticket and one key to RFC 0002's `next` order.

**`focus`**, derived on read and never stored, is for a ticket:

- **`direct`** when it has a current `serves` reference to a goal whose status is `focus`;
- **`inherited`** when it is not `direct`, but its epic (`parent`) is, or it is an unresolved dependency of a ticket that is `direct` or `inherited`, followed through `dependsOn` to any depth in the repository space (RFC 0002, "Derived state"). A goal's ticket can only move once what it depends on is done, so that work serves the goal too, without anyone writing a reference on it;
- **`none`** otherwise;
- **`null`**, not known, for a reader that holds no copy of the coordination space `nexus-flow.toml` names. For such a reader, ranking leaves the focus key out.

The goal statuses are read from the coordination space, and the walk over `parent` and `dependsOn` runs in the repository space, as the readiness walk does: nexus-flow does it in memory, behind the storage crate's functions, never inside NoCentralDB, which forbids a computed value that follows a reference (NoCentralDB RFC 0004, "How a reference resolves"). The walk over links within the repository space is RFC 0001's derived-state stop-gap until NoCentralDB computes reach (R7); the step across the reference stays nexus-flow's for good. Whether focus is inherited at all is question 4.

**The `next` order** becomes: status (claimed work first), then `focus` (`direct` and `inherited` before `none`), then priority, type, due date and key, as RFC 0002 orders them (RFC 0002, "What the model is generated from"), within RFC 0002's tiers. Work in progress stays first, so an agent finishes what it holds; among the new work, every ticket that serves a goal in focus, directly or by being in its way, comes before every ticket that does not. When a goal enters or leaves focus, only the goal's status changes, and the order of every repository's tickets follows on the next read.

**`nxs ticket list --focus`** lists only the tickets whose `focus` is `direct` or `inherited`, and **`--goal <goal>`** only the tickets with a `serves` reference to that goal; both read through `byTarget` as above.

**Readiness does not change.** A goal is not a dependency: whether a ticket is ready is computed from the repository space alone (RFC 0001, "Derived state"), and focus only orders the ready tickets.

### What v1's priority becomes

RFC 0002 keeps `priority`, the ordinal 0 to 4, on every ticket, carried over from v1 (RFC 0002, "What the model is generated from"; NEX-29). With goals ranking the work, it orders the tickets within one focus value: a `P0` bug that serves no goal still comes after every new ticket that serves a goal in focus, and among the tickets of the goals in focus, `P0` comes first. The field stays a ticket's own judgement of urgency, and is never set to express a goal.

*Not taken:* deprecating `priority` (NoCentralDB RFC 0005, "Modifiers"). Goals rank by what the group pursues, not by how urgent one ticket is; a bug that breaks the build is urgent whatever the goals are, and v1's workspaces carry the field into v2 (RFC 0008). Whether the owner wants it kept as the second key is question 3.

### What the re-evaluation stores

When a goal is added, a model proposes which tickets serve it, and the owner confirms (GENESIS.md). That flow is priority 2. What it needs stored is the proposal, and it stores it where the confirmed answer will live: as a `ticketsReference` of the new kind **`proposed`**, with the goal as its target, written by pm in its repository space. Confirming changes `kind` to `serves`; declining deletes the object. A `proposed` reference never counts for focus, and RFC 0002's `references` list shows it with its kind, so a reader can tell it from a confirmed one.

Proposals live in the repository space, not the coordination space, because the ticket is there: pm writes them where it may write, and the owner's devices, members of every space, confirm them where the ticket is. Proposing to move a ticket from one goal to another, as GENESIS.md also asks, is a `proposed` reference to the new goal next to the `serves` reference to the old one. The commands, the model's prompt and how the owner confirms are designed with the flow.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Commands:**
  - `nxs goal add --title <text> [--description <text> | --description-file <path>] [--reached-when <text>] [--owner <identity>] [--focus]`.
  - `nxs goal list [--status <status>]... [--all]`: without flags the goals that are `focus`, `proposed` or `later`; `--all` adds `reached` and `dropped`.
  - `nxs goal show <goal>`.
  - `nxs goal edit <goal> [--title <text>] [--description <text> | --description-file <path>] [--reached-when <text>] [--owner <identity>]`.
  - `nxs goal status <goal> <proposed | focus | later | reached | dropped> [--comment <text>]`.
  - RFC 0002's `nxs ticket ref <key> --serves <target>` accepts a goal id; `nxs ticket list` gains `--focus` and `--goal <goal>`.
- **JSON output** (every timestamp RFC 3339 UTC, every identity as RFC 0002 writes it):
  - A goal, in `nxs goal list` and as the head of `nxs goal show`: `id`, `title`, `reached_when`, `status`, `status_at`, `status_comment`, `status_by`, `owner`, `created_at`, `created_by`, `conflict`, and `tickets`, the number of this repository's tickets with a current `serves` reference to it (`null` without a repository space).
  - `nxs goal show` adds `description`; `tickets` as a list instead of a number, each with RFC 0002's `id`, `key`, `title`, `status`, `ready`, `done` and `on_main`; and `requests`, each as RFC 0009's `nxs request list` prints one.
  - A ticket's JSON (RFC 0002) gains `focus` (`direct`, `inherited`, `none` or `null`). Each entry of its `references` whose target is a goal gains `goal_status` (the goal's status, or `null` for a reader who cannot resolve it) and `current` (whether its space is the coordination space `nexus-flow.toml` names); `kind` may be `proposed`.
  - The errors `no_coordination_space`, `goal_not_found`, `goal_id_ambiguous` (with the matching goals), `goal_status_needs_a_person` and `goal_already_served`.
- **Library seam:** `Engine::goal_add`, `Engine::goals`, `Engine::goal`, `Engine::goal_edit`, `Engine::goal_status`, with the types `Goal`, `GoalStatus` and `Focus`; RFC 0002's ticket reads on the seam take the same `focus` and `goal` filters and return `focus`. Each `nxs goal` verb has its counterpart here (the decision on CLI and library parity stays in force).
- **Stored formats:** the collection `goalsGoal` with its fields, its status values and the index `byStatus`; `goalsGoal` as a target of RFC 0002's `ticketsReference.target`; the kind `proposed` on `ticketsReference.kind`.

### What nexus-flow needs from NoCentralDB

**Met:**

- **A reference into another space, and an index on it.** `nc.ref` carries the target's space id and object id, resolves per reader to `ok`, `notAccessible` or `missing`, and an index on it finds every reference to one target in one lookup, the same on every member (NoCentralDB RFC 0005, "References into another space"; RFC 0004, "References").
- **Adding a target collection to a reference.** Accepted as an additive change (NoCentralDB RFC 0005, "References into another space", "The additive-change rule"), and so is a new option of `nc.oneOf`.
- **One database for both spaces.** A member's database holds every space it belongs to (NoCentralDB RFC 0006, "Where the view lives"), so pm reads goals and tickets locally, together.
- **A member knows its own kind.** An SDK database is opened as a device, an agent or a sub-agent (NoCentralDB RFC 0009, "Sub-agents and ad-hoc spaces"; RFC 0010, "Accounts"), which is what `goal_status_needs_a_person` checks.

**Requests:** none beyond RFC 0001's. The author fields wait for R6, and the walk over `parent` and `dependsOn` for R7, as RFC 0001 and RFC 0002 request. The step from a ticket to its goal's status is not a request: NoCentralDB decided that nothing computed follows a reference (NoCentralDB RFC 0004, "How a reference resolves"), and nexus-flow does that step itself.

### A worked example

The owner's four repositories, nexus-flow, central-infra, app-foundations and nexflowit-app, share a coordination space (RFC 0009, "A worked example").

1. **A goal.** On a device, the owner runs `nxs goal add --title "Win 20 paying customers for nexflow.it" --reached-when "20 customers pay a monthly plan" --focus`. Its status is `focus` from now.
2. **pm proposes a second.** nexus-flow's pm sees that three of its tickets would make the CLI usable by a second team and runs `nxs goal add --title "A second team runs its factory on nexus-flow"`. The goal is `proposed`: pm's member is an agent. The owner reads it in `nxs goal list` and sets it to `later`.
3. **Tickets serve it.** nexflowit-app's pm has a ticket for the checkout page and runs `nxs ticket ref NXA-12 --serves 3f9a1c07`. central-infra's pm accepts the webhook request that names the goal and links its two tickets with `nxs request link`, which writes the same reference on both (RFC 0009, "From a request to tickets").
4. **Work moves.** central-infra's `nxs ticket next` lists its two webhook tickets as `focus: "direct"`, and the database migration one of them depends on as `inherited`, before a `P1` refactoring that serves no goal. The pm hands out the migration first.
5. **A coder reads its ticket.** The coder on the migration sees the ticket and nothing of the goal: it is not a member of the coordination space, the reference reads `not_accessible` with the label "Win 20 paying customers for nexflow.it", and its `focus` is `null`.
6. **The focus changes.** Twenty customers pay. The owner runs `nxs goal status 3f9a1c07 reached --comment "20 paying on 2027-03-02"`. On the next read, after sync, the tickets that served it are `focus: "none"` in every repository, and the refactoring moves up. No ticket was changed.

## What it does not do

- **Every read is as fresh as the last sync.** A goal put into focus ranks a repository's work only once that repository's pm has synced it; until then pm ranks by the focus its copy holds (GENESIS.md, "Limits").
- **The focus is known only to a member of the coordination space.** A role session sees its ticket's goals as `not_accessible`, with the title they had when they were written, and `focus: null`; its `nxs ticket next` leaves the focus key out. pm, which hands out the work, holds both spaces.
- **A pm sees its own repository's tickets for a goal, not the others'.** A view of a goal across every repository needs a member of every repository space, such as the owner's devices or manufakt.io.
- **A label can be stale.** A reference's `label` is the goal's title when the reference was written; a renamed goal shows its old title to a reader who cannot resolve it.
- **The status rule is `nxs`'s, not NoCentralDB's.** Any writer of the coordination space can set any status or edit any goal through another client; `statusBy` says who did, as a field the writer writes, until R6.
- **A goal is not a dependency.** Focus orders the ready tickets; it never makes a ticket ready or blocked.
- **Focus is computed on every read** that ranks, from the goals in focus and the walk over epics and dependencies, in memory, until R7 moves the walk inside the repository space into NoCentralDB. The step across the reference stays nexus-flow's.
- **No goals without a coordination space.** A repository in no group creates a coordination space of its own to use them.
- **A goal is never deleted through `nxs`.** A goal written by mistake is `dropped`, and stays readable.
- **Proposals are stored, not made.** The `proposed` kind exists from this RFC on; the flow that writes and confirms proposals is priority 2.

## Migration

Nothing to migrate. v1 has no goals: its board holds one repository's tickets, ordered by priority, and nothing in it names something the tickets serve. v1's priority is carried over by RFC 0002 and RFC 0008 and keeps its meaning (see "What v1's priority becomes"). A group of repositories starts on v2 with no goals, and every ticket's `focus` is `none` until a goal is put into focus and tickets name it.

## Open questions

1. **Who may change a goal's status.** Asked in NEX-58. (a) Only a person: `nxs` refuses any status other than `proposed` for an agent or a sub-agent, as designed. (b) Any member, with `statusBy` showing who. Option a keeps the focus, which steers every agent of the group, in the owner's hand, at the cost of a step for the owner on every goal pm proposes. Option b lets pm adjust the focus on its own, but then an agent decides what every other agent takes on next. Either binds `nxs` only, since NoCentralDB's rights apply to a whole space. **Recommendation: a.**
2. **An order among the goals in focus.** Asked in NEX-59. (a) None: every goal in focus counts the same, and priority orders within them, as designed. (b) An explicit order, a `rank` field the owner sets, so the first goal's work goes before the second's. Option a has nothing to keep current, and fits GENESIS.md's "work for a goal goes first". Option b lets the owner say which of two goals in focus wins, but the order must be kept current, and two people reordering at once conflict. **Recommendation: a; b can be added later as a field, additively.**
3. **What v1's priority becomes.** Asked in NEX-60. (a) It stays, as the key after focus, as designed. (b) It is deprecated: goals alone rank, and type, due date and key order the rest. Option a keeps urgency that no goal expresses, such as a broken build, and loses nothing of v1's data. Option b has one ranking idea instead of two, but a fix that serves no goal can then only move up by being given a goal. **Recommendation: a.**
4. **Whether focus is inherited.** Asked in NEX-61. (a) Through the epic and through every unresolved dependency, as designed. (b) Direct only: only a ticket with its own `serves` reference to a goal in focus moves up. Option a ranks first what stands in a goal's way, without pm writing a reference on each such ticket; it costs a walk on every read until R7. Option b is simpler to explain, but a goal's ticket can wait behind its own unranked dependencies until pm writes references on them. **Recommendation: a.**
5. **Goals for a repository in no group.** Asked in NEX-62. (a) It creates a coordination space of its own, as designed. (b) Goals live in the repository space while it belongs to no group. Option a keeps one place for goals, and joining a group later moves nothing; it costs a second space, and its pm becomes a member of both. Option b needs no second space, but joining a group later needs a migration of every goal and every reference to it, and the storage crate's rule on which collection goes into which kind of space gets an exception. **Recommendation: a.**
6. **What a reader without the coordination space sees.** Asked in NEX-63. (a) `focus: null`, and its `next` leaves the focus key out, as designed. (b) pm keeps a copy of the goals in focus in the repository space, one record per space, which it rewrites when the focus changes, so every member computes the same `focus`. Option a stores nothing derived; a coder's order differs from pm's, but pm hands out the work. Option b gives every member the same order, at the cost of a copy that is stale until pm runs and that a writer can change, and it would bring back a stored value that GENESIS.md wants derived. **Recommendation: a.**
7. **Goal ids and command names.** Asked in NEX-64. (a) A goal id prefix of at least 8 digits, with the readable form that RFC 0002 and RFC 0009 choose for tickets and requests once the owner decides it, and the commands as designed: `nxs goal add|list|show|edit|status`. (b) Other names, such as separate verbs for each status (`nxs goal focus`, `nxs goal drop`). Option a keeps one command per thing done and one rule for ids across modules. Option b reads closer to English in a prompt, but adds five commands for one field. **Recommendation: a.**

## History

- 2026-11-17: published as a draft in #161.
