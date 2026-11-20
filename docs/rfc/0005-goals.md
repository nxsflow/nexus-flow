# RFC 0005: Goals in NoCentralDB

Status: Decided (2026-11-20)

This RFC designs the `goals` module of `factory/GENESIS.md`, "Goals set the focus". It covers how a group of repositories stores the goals it focuses on in its coordination space, and the order the owner gives the goals in focus. It covers how a ticket in a repository space names the goals it serves, and how pm reads "the tickets that serve this goal" from its own local database. It sets the rule by which the goals in focus rank a repository's work, and what v1's priority field becomes once they do. It builds on RFC 0001, which places goals in the coordination space, and works with RFC 0002 (tickets) and RFC 0009 (work across repositories). Now that it is decided, a coding ticket can build the module: the `goalsGoal` collection in nexus-flow's model, the goal reference on RFC 0002's `ticketsReference`, the `nxs goal` commands, the goal filters and the focus key of `nxs ticket`, and their counterparts on the library seam.

## What it solves

The owner directs agents across several repositories. They want the agents to work on what matters now, without re-prioritising dozens of tickets whenever that changes. GENESIS.md, "Goals set the focus", asks for:

- **A goal spans repositories.** "Win 20 paying customers for nexflow.it" touches the repository that integrates the billing provider, the one that runs the waiting list, and the product itself.
- **A ticket serves several goals, or none yet.** pm ranks the work by the goals in focus, and no priority field on dozens of tickets needs updating when a goal changes.
- **Work for a goal goes first.** Agents finish what they are working on, but take on new work that serves a goal in focus before other new work.
- **Goals live in the coordination space,** and each repository's pm ranks its own tickets against them.
- **A new goal starts a re-evaluation:** a model proposes which tickets serve it, and the owner confirms.

The owner added one need when deciding this RFC: agents work on several goals in parallel when they have the capacity. Goal 1's ready work goes first. When nothing of goal 1 is free, and nothing will be soon, work goes on with goal 2 (NEX-59).

NoCentralDB shapes how. Rights apply to a whole space, a link points only into its own space (NoCentralDB RFC 0004, "Links"), and every query runs through an index the model declares (NoCentralDB RFC 0005, "Indexes"). Nothing computed may follow a reference into another space (NoCentralDB RFC 0004, "How a reference resolves"). A goal lives in the coordination space and a ticket in a repository space, so the ticket names the goal through a reference, and every rule that reads both is nexus-flow's.

**Scope.** This RFC designs the goal's storage, its commands and JSON, the reference from a ticket to a goal, and the ranking rule. The re-evaluation when a goal is added is behaviour for priority 2 (decision of 2026-11-15, "Priorities: migrate to NoCentralDB, then build the new factory, then the rest"). This RFC stores only what it needs (see "What the re-evaluation stores"), and its flow is designed later by rewriting this RFC.

Out of scope, each in its own RFC:

- The ticket collection, its `ticketsReference` collection, readiness and the `next` order this RFC adds a key to (RFC 0002).
- The coordination space's repositories and requests, and the goal a request serves (RFC 0009).
- What the daily note says about goals (RFC 0007).

NoCentralDB's own design is in nocentraldb/nocentraldb, `docs/rfc/`, cited here as "NoCentralDB RFC" by number and section. An RFC number without that prefix is one of nexus-flow's own.

## The design

### Where goals live

Goals live in the coordination space (RFC 0001, "Spaces"). Every repository of the group reads the same goals from its own copy, and every pm there may write them (RFC 0001, "Roles in a space"). The storage crate refuses to write a goal into a repository space (RFC 0001, "The model").

**A repository that belongs to no group** creates a coordination space of its own to use goals (decided by the owner, NEX-62). A solo developer runs `nxs space create-coordination` (RFC 0009, "Creating it") for a group of one repository. `nxs goal add` without a coordination space refuses with `no_coordination_space` and names that command. When other repositories join later, the goals are already where they belong, and nothing moves. The cost is a second space, and its pm is a member of both.

*Not taken:* goals in the repository space while it belongs to no group. Joining a group later would then need a migration of every goal and every reference to it. The storage crate's rule on which collection goes into which kind of space would also get an exception.

### The goal

A goal is one object in the collection `goalsGoal`:

| Field | Type | Holds |
| --- | --- | --- |
| `key` | `nc.string()` | Its readable id, such as `cb.3` (see "Goal ids") |
| `title` | `nc.string()` | The goal in one line, such as "Win 20 paying customers for nexflow.it" |
| `description` | `nc.string()` | Why it matters, Markdown; optional |
| `reachedWhen` | `nc.string()` | What "reached" means, as a check a person can make, such as "20 customers pay a monthly plan"; optional |
| `status` | `nc.oneOf(["proposed", "focus", "later", "reached", "dropped"])` | Where the goal stands (see "Status") |
| `rank` | `nc.string()` | Its place among the goals in focus; unset for a goal not in focus (see "The order of the goals in focus") |
| `statusAt` | `nc.string()` | Since when it has had this status, RFC 3339 UTC |
| `statusComment` | `nc.string()` | Why the status last changed, if a reason was given |
| `owner` | `nc.string()` | Who answers for the goal: an identity, `account:<fingerprint>` or `member:<member id>`, as RFC 0002's `assignee`; optional |
| `createdAt`, `createdBy`, `statusBy` | `nc.string()` | When and by which member it was created, and which member set the status and the rank; RFC 0001's author stop-gap, until R6 |

Every field may be unset, as every NoCentralDB field may (NoCentralDB RFC 0005, "No required fields and no defaults"). `nxs` writes `key`, `title`, `status`, `statusAt` and the author fields on every goal it creates, and reads a goal with no `status` as `later`.

**One status, not a focus flag beside a lifecycle.** "In focus or not" and "reached or not" are one field, so a goal cannot be in focus and reached at the same time, and "since when" is one timestamp. *Not taken:* a boolean `inFocus` beside a status. The two fields could contradict each other after a concurrent write, since each merges on its own (NoCentralDB RFC 0004, "Basic values").

**No ticket list on the goal.** A goal does not hold the tickets that serve it. They live in other spaces, a field holds one reference, and NoCentralDB has no field of many references (NoCentralDB RFC 0005, "References into another space"). The ticket names the goal instead (see "A ticket serves a goal").

**A goal is never deleted.** `nxs` offers no command that deletes one: a goal that no longer matters is `dropped`. References to it from tickets, and the `goal` link of RFC 0009's requests, therefore stay resolvable. A ticket never names a goal that reads `missing` because someone tidied up.

**The indexes.** Every goal read runs through one of two indexes, both with `scope: "space"` (NoCentralDB RFC 0005, "Indexes"), or reads one goal by its object id (NoCentralDB RFC 0009, "Get by id"):

- `byStatus` on `status`, `rank` and `statusAt`, with `equal: 1`. It gives the goals of one status in one lookup: the goals in focus in their order, and the others longest-standing first, since only a goal in focus has a rank.
- `byKey` on `key`, with `equal: 1`, which resolves a key.

#### Goal ids

A goal has two ids, as a ticket does (decided by the owner, NEX-64). Its object id is NoCentralDB's, and every reference to the goal stores it. Its **key** is the readable id people and agents use, in the form RFC 0002 decided for ticket keys ("Ticket ids"): the creating member's tag in the coordination space, a dot, and a number, such as `cb.3`. The key is written once, when the goal is created, and never again by `nxs`. Every member of the coordination space gets a tag there when it is added, as RFC 0009 has it for requests (RFC 0009, "Request ids"). RFC 0002's rules hold for these tags: a freed tag is reused, and two members given one tag offline mint the same keys. `nxs` then names both goals by their object ids until pm retags one of the members.

**One numbering per tag in the coordination space.** Goals and requests share the coordination space, so a member's number counts across both collections. The next number is the highest number among the goal and request keys with the member's tag in its local copy, plus one. A key then names one object in the coordination space, as RFC 0002 has a key unique within its space. A key a person types or reads in a thread is then never both a goal and a request. *Not taken:* a numbering per collection, under which the goal `cb.3` and the request `cb.3` could both exist and the text alone would not say which is meant.

`nxs` accepts a goal's key or its object id wherever it takes a goal. Two goals with one key are listed with the first characters of their object ids (`cb.3~3f9a`), and an argument that names only the key fails with `ambiguous_key` (RFC 0002, "Ticket ids").

### Status

| Status | Means |
| --- | --- |
| `proposed` | An agent suggested it; a person has not yet decided whether the group pursues it |
| `focus` | In focus: work that serves it goes first, in the goals' order |
| `later` | The group pursues it, but not now |
| `reached` | What `reachedWhen` names is true |
| `dropped` | No longer pursued |

**Only a person changes a goal's status or its place in the order** (decided by the owner, NEX-58). What is in focus, and in which order, decides what every agent of the group takes on next, so it is the owner's lever (GENESIS.md, "Goals set the focus", and principle 1). `nxs` refuses when the member it runs as is an agent or a sub-agent (NoCentralDB `MemberKind` `agent` or `subAgent`; RFC 0001, "Members"), with the error `goal_change_needs_a_person`. The refusal covers setting a status other than `proposed` and moving a goal in the order. A goal an agent adds is `proposed`. A goal a person adds is `later`, or `focus` with `--focus`. A person may move a goal from any status to any other. The cost is a step for the owner on every goal pm proposes.

As with RFC 0009's request statuses, the rule binds `nxs` and the library seam only. Rights apply to a whole space, so a writer of the coordination space that writes through another client can set any status or rank (NoCentralDB RFC 0008, "Roles and rights"). `statusBy` shows which member set them, until R6 as a field the writer writes.

*Not taken:* any member may change a goal's status. pm could then adjust the focus on its own, and an agent would decide what every other agent takes on next.

Two members who set the status at the same time write two values. The last writer wins (NoCentralDB RFC 0004, "Basic values"), and `nxs` reports the goal with `conflict: true` until the next change of its status or rank (NoCentralDB RFC 0009, "Conflicts, lost removals and link states").

### The order of the goals in focus

The goals in focus have an order the owner sets (decided by the owner, NEX-59). Goal 1's work goes before goal 2's, goal 2's before goal 3's, and so on (see "Ranking"). Each goal in focus carries its place in the order as its `rank`, a string that `nxs` compares by its bytes, lowest first. NoCentralDB compares text in an index the same way (NoCentralDB RFC 0005, "Indexes").

**One goal is written per move.** A new rank lies strictly between the ranks of the two goals it is placed between, as fractional indexing does. It is made of the characters `0` to `9` and `a` to `z`, never ends in `0`, and grows by one character only when no shorter string fits between its neighbours. Moving a goal therefore writes that goal's `rank` and no other goal's, however long the order is. `nxs` never renumbers the order.

**Setting the order.**

- `nxs goal status <goal> focus` puts a goal into focus last. With `--before <goal>` or `--after <goal>`, it goes next to a goal in focus instead.
- `nxs goal edit <goal> --first | --last | --before <goal> | --after <goal>` moves a goal that is in focus.
- A goal that leaves focus loses its rank, and comes back last unless placed.
- Only a person moves a goal (see "Status").

**Two people who reorder at once.** Each move writes one goal's rank, so every member of the coordination space computes the same order from the same ranks once it has synced. Three cases differ in what the people see:

- **They move different goals.** Both moves stand. Each goal lands between the ranks its mover saw, so the order holds both moves. It may still not be the order either person meant, for example when one put A before B and the other put B before A. `nxs goal list` shows the order as it is, and a person moves a goal again to settle it.
- **They move the same goal.** The last writer wins (NoCentralDB RFC 0004, "Basic values"). NoCentralDB keeps the losing rank as a conflict, and `nxs goal list` shows the goal with `conflict: true` until it is moved again.
- **Two goals get the same rank.** This happens when two people put two goals into the same gap while offline. The tie is broken by key, then by object id, so every member still sees one order. The next move of either goal writes a rank of its own.

No move is lost silently, and a reorder never touches more goals than the one it moves.

*Not taken:* no order among the goals in focus, where priority alone would order their work. The owner wants to say which goal's work comes first (NEX-59). Also not taken: positions `1` to `n` stored on each goal, where moving one goal rewrites every goal between its old place and its new one. Two people reordering at once would then conflict on many goals and could leave two goals at the same position. A single list of the goals in focus on one object was not taken either: concurrent moves of different goals would conflict on that one value, and the last writer's list would undo the other person's move whole.

### A ticket serves a goal

A ticket in a repository space cannot link to a goal in the coordination space (NoCentralDB RFC 0004, "Links"). It names it through RFC 0002's `ticketsReference` object of kind `serves`, whose `target` is an `nc.ref` (RFC 0002, "References out of the space"). This RFC adds `goalsGoal` to that field's targets, and the kind `proposed` to its kinds. Both are additive changes (NoCentralDB RFC 0005, "The additive-change rule"):

| Field | Type in RFC 0002 | With this RFC |
| --- | --- | --- |
| `target` | `nc.ref(["ticketsTicket"])` | `nc.ref(["ticketsTicket", "goalsGoal"])` |
| `kind` | `nc.oneOf(["serves", "cites"])` | `nc.oneOf(["serves", "cites", "proposed"])` (see "What the re-evaluation stores") |

A ticket serves as many goals as it has `serves` references to goals, or none. `label` holds the goal's key and title when the reference was written, such as `cb.3 Win 20 paying customers for nexflow.it`, for a reader who cannot resolve it (RFC 0002, "References out of the space").

**Writing one.** `nxs ticket ref <key> --serves <goal>` (RFC 0002) takes a goal's key or object id in the coordination space `nexus-flow.toml` names. It resolves the goal there and writes the reference with the goal's key and title as `label`. `--remove` deletes it. RFC 0009's `nxs request link` writes the same reference when the request names a goal and the ticket serves none (RFC 0009, "From a request to tickets"). `nxs` refuses a second `serves` reference from one ticket to the same goal. Two written concurrently on two members are both kept and read as one.

**Only the current coordination space counts.** A `serves` reference may name a goal in another coordination space, for example one written before the repository left a group (RFC 0009, "Repositories in the space"). Such a reference is shown with `current: false` and never counts for focus.

#### Reading "the tickets that serve this goal"

pm reads it locally, in one database. A member's database holds every space it belongs to (RFC 0001, "The local data on disk"; NoCentralDB RFC 0006, "Where the view lives"), so pm's holds both the coordination space and its repository space:

1. The goals in focus, in their order: `byStatus` with `status = "focus"` in the coordination space.
2. The tickets that serve one goal: RFC 0002's index `byTarget` on `ticketsReference`, in the repository space, with `target` set to the goal's space id and object id and `kind = "serves"`. NoCentralDB indexes a reference as its space id and object id, whatever its state. This lookup is therefore the same on every member of the repository space, and needs no copy of the goal (NoCentralDB RFC 0005, "References into another space").
3. The goals one ticket serves: RFC 0002's `byTicket` on `ticketsReference`, then each target read by its id in the coordination space.

No read crosses the network or follows a reference inside NoCentralDB: nexus-flow reads the goal ids in one space and looks them up in the other. A goal's own view, `nxs goal show`, puts both together, with the requests that serve the goal from RFC 0009's `byGoal` index.

**Only this repository's tickets.** A pm is a member of its own repository space only (RFC 0001, "Roles in a space"), so it sees which of its own tickets serve a goal, never another repository's. A view of a goal across every repository needs a member of every repository space. The owner's devices are such members, and manufakt.io, as a device of the owner's account, can read each space and add them up (RFC 0001, "Members"). This RFC stores no roll-up in the coordination space: a count written by each pm would be derived state that drifts from the tickets it counts (RFC 0001, "Derived state").

#### A member who reads one space but not the other

A reference resolves per reader to `ok`, `notAccessible` or `missing` (NoCentralDB RFC 0004, "How a reference resolves"). What a member sees of goals therefore depends on the spaces it holds:

| Member | Holds | Sees |
| --- | --- | --- |
| pm | Both spaces | Every goal, and for its own tickets which goals they serve, with the goal's status and place in the order. Ranks by focus. |
| The owner's devices, manufakt.io | Every space | As pm, for every repository |
| A role session (coder, legal, …), or any other member of a repository space that is not in its coordination space | The repository space only, and for a role session perhaps an ad-hoc space with copies of requests (RFC 0009, "Members and roles") | Its tickets' goal references as `not_accessible`, each with its `label`. It sees no goal status, so `focus` is `null` (see "Ranking"). |
| A developer device in the coordination space, but not in a repository's space | The coordination space only | The goals and the requests, but no ticket of that repository |
| Any member, for a goal not yet synced | Both spaces | The reference as `missing`, with its `label`, until the goal arrives |

A reference to a goal that reads `not_accessible` or `missing` never counts for focus, for that reader. Nothing derived from a reference's state is stored (NoCentralDB RFC 0005, "References into another space").

A role session never reads the coordination space: it is not a member there (RFC 0001, "Roles in a space"), and the ad-hoc space pm may give it holds copies of requests, not goals (decided by the owner, NEX-82; RFC 0009, "Members and roles"). It is therefore always in the row with `focus: null`, and its `nxs ticket next` leaves the focus key out. pm hands out work in the order of the goals, so a session needs no focus of its own. The same row holds a member a team keeps out of the coordination space.

### Ranking

GENESIS.md: "work for a goal goes first", and agents "may finish what they are working on, but they take on only the new work identified as important for the goal." The owner added that goal 1's ready work goes first, then goal 2's, so agents with capacity can work on several goals in parallel (NEX-59). This RFC adds one derived value per ticket and one key to RFC 0002's `next` order.

**`focus`**, derived on read and never stored, is for a ticket:

- **`direct`** when it has a current `serves` reference to a goal whose status is `focus`.
- **`inherited`** when it is not `direct`, but its epic (`parent`) is `direct` or `inherited`, or it is an unresolved dependency of a ticket that is `direct` or `inherited`. Dependencies are followed through `dependsOn` to any depth in the repository space (RFC 0002, "Derived state"). A goal's ticket can only move once what it depends on is done, so that work serves the goal too, without anyone writing a reference on it (decided by the owner, NEX-61).
- **`none`** otherwise.
- **`null`**, not known, for a reader that holds no copy of the coordination space `nexus-flow.toml` names (decided by the owner, NEX-63). For such a reader, ranking leaves the focus key out.

**Its goal in focus.** A ticket that is `direct` or `inherited` ranks by one goal: the first in the order among the goals in focus it serves directly or inherits from. Its JSON names that goal and the goal's position in the order, `1` for the first.

The goal statuses and ranks are read from the coordination space. The walk over `parent` and `dependsOn` runs in the repository space, as the readiness walk does. nexus-flow does it in memory, behind the storage crate's functions, never inside NoCentralDB, which forbids a computed value that follows a reference (NoCentralDB RFC 0004, "How a reference resolves"). The walk over links within the repository space is RFC 0001's derived-state stop-gap until NoCentralDB computes reach (R7). The step across the reference stays nexus-flow's for good.

*Not taken:* focus for direct references only. A goal's ticket could then wait behind its own unranked dependencies until pm wrote references on them.

**The `next` order** becomes, within RFC 0002's tiers:

1. Status: claimed work first.
2. The position of the ticket's goal in focus: goal 1's work, then goal 2's, and so on, then the tickets whose `focus` is `none`.
3. Priority, type, due date and key, as RFC 0002 orders them (RFC 0002, "What the model is generated from").

Work in progress stays first, so an agent finishes what it holds. Among the new work, goal 1's tickets come first, whether they serve it directly or stand in its way. Then come goal 2's tickets, and the tickets that serve no goal in focus come last. `next` lists only work that is ready or in progress. When nothing of goal 1 is ready, or all of it is already held, goal 2's ready work leads the list, and an agent with capacity takes it, as the owner asked (NEX-59). The order shows what is ready now. Whether to hold a lane back for goal 1's work that will be free soon is pm's judgement when it hands out work; nothing in the order holds a lane.

When a goal enters or leaves focus, or moves in the order, only the goal changes. The order of every repository's tickets follows on the next read.

**`nxs ticket list --focus`** lists only the tickets whose `focus` is `direct` or `inherited`, in the order of their goals. **`--goal <goal>`** lists only the tickets with a `serves` reference to that goal. Both read through `byTarget` as above.

**Readiness does not change.** A goal is not a dependency. Whether a ticket is ready is computed as RFC 0002 computes it, and focus only orders the ready tickets.

### What v1's priority becomes

RFC 0002 keeps `priority`, the ordinal 0 to 4, on every ticket, carried over from v1 (RFC 0002, "What the model is generated from"; NEX-29). It stays as the key after focus (decided by the owner, NEX-60). It orders the tickets of one goal in focus, and the tickets that serve no goal in focus. A `P0` bug that serves no goal still comes after every new ticket that serves a goal in focus. Among goal 1's tickets, `P0` comes first. The field stays a ticket's own judgement of urgency, and is never set to express a goal.

*Not taken:* deprecating `priority` (NoCentralDB RFC 0005, "Modifiers"). Goals rank by what the group pursues, not by how urgent one ticket is. A bug that breaks the build is urgent whatever the goals are, and v1's workspaces carry the field into v2 (RFC 0008).

### What the re-evaluation stores

When a goal is added, a model proposes which tickets serve it, and the owner confirms (GENESIS.md). That flow is priority 2. What it needs stored is the proposal, and it is stored where the confirmed answer will live: a `ticketsReference` of the new kind **`proposed`**, with the goal as its target, written by pm in its repository space. Confirming changes `kind` to `serves`; declining deletes the object. A `proposed` reference never counts for focus. RFC 0002's `references` list shows it with its kind, so a reader can tell it from a confirmed one.

Proposals live in the repository space, not the coordination space, because the ticket is there. pm writes them where it may write, and the owner's devices, members of every space, confirm them where the ticket is. GENESIS.md also asks for proposals that move a ticket from one goal to another. Such a proposal is a `proposed` reference to the new goal next to the `serves` reference to the old one. The commands, the model's prompt and how the owner confirms are designed with the flow.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner. The command names are `nxs goal add|list|show|edit|status` (decided by the owner, NEX-64).

- **Commands:**
  - `nxs goal add --title <text> [--description <text> | --description-file <path>] [--reached-when <text>] [--owner <identity>] [--focus [--before <goal> | --after <goal>]]`.
  - `nxs goal list [--status <status>]... [--all]`: without flags, the goals that are `focus` (in their order), `proposed` or `later`; `--all` adds `reached` and `dropped`.
  - `nxs goal show <goal>`.
  - `nxs goal edit <goal> [--title <text>] [--description <text> | --description-file <path>] [--reached-when <text>] [--owner <identity>] [--first | --last | --before <goal> | --after <goal>]`.
  - `nxs goal status <goal> <proposed | focus | later | reached | dropped> [--before <goal> | --after <goal>] [--comment <text>]`.
  - Wherever a command takes `<goal>`, it takes the goal's key or its object id.
  - RFC 0002's `nxs ticket ref <key> --serves <target>` takes a goal. `nxs ticket list` gains `--focus` and `--goal <goal>`.
- **JSON output** (every timestamp RFC 3339 UTC, every identity as RFC 0002 writes it):
  - A goal, in `nxs goal list` and as the head of `nxs goal show`, has these keys:
    - `id`, `key`, `title`, `reached_when`, `status`.
    - `position`: its place in the order, `1` for the first, or `null` when it is not in focus.
    - `status_at`, `status_comment`, `status_by`, `owner`, `created_at`, `created_by`, `conflict`.
    - `tickets`: the number of this repository's tickets with a current `serves` reference to it, or `null` without a repository space.
  - `nxs goal show` adds:
    - `description`.
    - `tickets` as a list instead of a number, each with RFC 0002's `id`, `key`, `title`, `status`, `ready`, `done` and `on_main`.
    - `requests`, each as RFC 0009's `nxs request list` prints one.
  - A ticket's JSON (RFC 0002) gains `focus` (`direct`, `inherited`, `none` or `null`) and `focus_goal`. `focus_goal` is the goal it ranks by, with `id`, `key`, `title` and `position`, or `null`.
  - Each entry of a ticket's `references` whose target is a goal gains two keys. `goal_status` is the goal's status, or `null` for a reader who cannot resolve it. `current` says whether its space is the coordination space `nexus-flow.toml` names. Its `kind` may be `proposed`.
  - The errors `no_coordination_space`, `goal_not_found`, `goal_change_needs_a_person` and `goal_already_served`, and RFC 0002's `ambiguous_key` for a goal key that names two goals.
- **Library seam:** `Engine::goal_add`, `Engine::goals`, `Engine::goal`, `Engine::goal_edit` and `Engine::goal_status`, with the types `Goal`, `GoalStatus`, `GoalPlace` and `Focus`. RFC 0002's ticket reads on the seam take the same `focus` and `goal` filters and return `focus` and `focus_goal`. Each `nxs goal` verb has its counterpart here (the decision on CLI and library parity stays in force).
- **Stored formats:**
  - The collection `goalsGoal`: its fields, its status values, the rank format and the indexes `byStatus` and `byKey`.
  - The goal key in RFC 0002's form `<tag>.<number>`, numbered per tag across the goals and requests of the coordination space.
  - `goalsGoal` as a target of RFC 0002's `ticketsReference.target`, and the kind `proposed` on `ticketsReference.kind`.

### Changes to the decided RFCs

RFC 0002 left the goal reference's target and the ranking rule to this RFC. This RFC's design and the owner's answers change five places in RFCs that are already decided. Each is rewritten in a follow-up of its own, as a decided RFC changes (`docs/rfc/README.md`, "How a decided RFC changes"); this RFC states the rule they follow.

- **RFC 0001, "Derived state":** focus joins the derived state. The walk over epics and dependencies runs in the repository space, and the status and rank of each goal are read through the ticket's reference in the member's own copy of the coordination space (NEX-61, NEX-63).
- **RFC 0002, "What the model is generated from", "Derived state" and "Left to priority 2":** the `next` order gains the position of the ticket's goal in focus as its second key, after status and before priority (NEX-59, NEX-60). "Ranking by goals is priority 2" points to this RFC's rule instead. The proposal of which tickets serve a new goal points to "What the re-evaluation stores".
- **RFC 0002, "References out of the space":** `target` gains `goalsGoal`, rewriting "names `ticketsTicket` only today", and `kind` gains `proposed`. `nxs ticket ref --serves` takes a goal's key, and `label` holds the goal's key and title.
- **RFC 0002, "Commands, JSON and the seam":** `nxs ticket list` gains `--focus` and `--goal <goal>`. A ticket's JSON gains `focus` and `focus_goal`, and a reference to a goal gains `goal_status` and `current`.
- **RFC 0002, "Ticket ids":** in the coordination space, a member's numbers count across goals and requests together, so a key names one object there (NEX-64). RFC 0009 brings the tags into the coordination space for requests (NEX-49).

### What nexus-flow needs from NoCentralDB

**Met:**

- **A reference into another space, and an index on it.** `nc.ref` carries the target's space id and object id, and resolves per reader to `ok`, `notAccessible` or `missing`. An index on it finds every reference to one target in one lookup, the same on every member (NoCentralDB RFC 0005, "References into another space"; RFC 0004, "References").
- **Adding a target collection to a reference.** This is accepted as an additive change, and so is a new option of `nc.oneOf` (NoCentralDB RFC 0005, "References into another space", "The additive-change rule").
- **Text compared by its bytes in an index.** The goals in focus come out of `byStatus` in their rank order (NoCentralDB RFC 0005, "Indexes").
- **One database for both spaces.** A member's database holds every space it belongs to (NoCentralDB RFC 0006, "Where the view lives"), so pm reads goals and tickets locally, together.
- **A member knows its own kind.** An SDK database is opened as a device, an agent or a sub-agent (NoCentralDB RFC 0009, "Sub-agents and ad-hoc spaces"; RFC 0010, "Accounts"). That is what `goal_change_needs_a_person` checks.

**Requests:** none beyond RFC 0001's. The author fields wait for R6, and the walk over `parent` and `dependsOn` waits for R7, as RFC 0001 and RFC 0002 request. The step from a ticket to its goal's status is not a request: NoCentralDB decided that nothing computed follows a reference (NoCentralDB RFC 0004, "How a reference resolves"), and nexus-flow does that step itself.

### A worked example

The owner's four repositories, nexus-flow, central-infra, app-foundations and nexflowit-app, share a coordination space (RFC 0009, "A worked example"). The owner's device holds the tag `cb` there, and nexus-flow's pm holds `p3`.

1. **A goal.** On a device, the owner runs `nxs goal add --title "Win 20 paying customers for nexflow.it" --reached-when "20 customers pay a monthly plan" --focus`. The goal gets the key `cb.1`, and its status is `focus` from now, first in the order.
2. **pm proposes a second.** nexus-flow's pm sees that three of its tickets would make the CLI usable by a second team. It runs `nxs goal add --title "A second team runs its factory on nexus-flow"`. The goal gets the key `p3.1`, and it is `proposed`, since pm's member is an agent. The owner reads it in `nxs goal list` and runs `nxs goal status p3.1 focus`, which puts it second.
3. **Tickets serve them.** nexflowit-app's pm has a ticket for the checkout page and runs `nxs ticket ref p7.12 --serves cb.1`. central-infra's pm accepts the webhook request that names `cb.1`, and links its two tickets with `nxs request link`. That writes the same reference on both (RFC 0009, "From a request to tickets"). nexus-flow's pm writes `--serves p3.1` on its three tickets.
4. **Work moves.** central-infra's `nxs ticket next` lists the database migration one webhook ticket depends on as `focus: "inherited"`, with `focus_goal` `cb.1` at position 1. It lists it before a `P1` refactoring that serves no goal, and the pm hands out the migration first.
5. **Goals in parallel.** In nexus-flow, nothing serves `cb.1`, so `next` starts with the tickets of `p3.1`, and its pm hands them out. Later, the only ready ticket for `cb.1` in central-infra is already in progress, so its spare lane takes the next ticket of the order, which serves no goal there.
6. **A coder reads its ticket.** The coder on the migration is not a member of the coordination space (RFC 0009, NEX-82). It sees the reference as `not_accessible`, with the label "cb.1 Win 20 paying customers for nexflow.it", and `focus: null`; it works on the ticket pm handed it, in the order pm read.
7. **Two people reorder.** The owner and a second developer, both offline, each move `p3.1` before `cb.1`: one with `nxs goal edit p3.1 --first`, the other with `--before cb.1`. After sync, the last write wins and `nxs goal list` shows `p3.1` first with `conflict: true`. The owner confirms the place with one more move, which clears the conflict.
8. **The focus changes.** Twenty customers pay. The owner runs `nxs goal status cb.1 reached --comment "20 paying on 2027-03-02"`. On the next read, after sync, the tickets that served it are `focus: "none"` in every repository, and `p3.1` is first. No ticket was changed.

## What it does not do

- **Every read is as fresh as the last sync.** A goal put into focus or moved in the order ranks a repository's work only once that repository's pm has synced it. Until then pm ranks by the focus its copy holds (GENESIS.md, "Limits").
- **The focus is known only to a member of the coordination space.** A member without it sees its tickets' goals as `not_accessible`, with the label they had when they were written, and `focus: null`. Its `nxs ticket next` leaves the focus key out. That is every role session (RFC 0009, NEX-82), and a member a team keeps out of the coordination space.
- **A pm sees its own repository's tickets for a goal, not the others'.** A view of a goal across every repository needs a member of every repository space, such as the owner's devices or manufakt.io.
- **A label can be stale.** A reference's `label` holds the goal's key and title when the reference was written. A renamed goal shows its old title to a reader who cannot resolve it.
- **Two concurrent reorders can give an order neither person meant.** Every member sees the same order, and no move is lost, but moves of different goals combine. A person settles it with one more move.
- **The status and order rule is `nxs`'s, not NoCentralDB's.** Any writer of the coordination space can set any status or rank, or edit any goal, through another client. `statusBy` says who did, as a field the writer writes, until R6.
- **A goal is not a dependency.** Focus orders the ready tickets; it never makes a ticket ready or blocked.
- **The order shows what is ready now.** Whether to keep a lane free for a goal's work that will be ready soon is pm's judgement.
- **Focus is computed on every read** that ranks, from the goals in focus and the walk over epics and dependencies, in memory. R7 will move the walk inside the repository space into NoCentralDB. The step across the reference stays nexus-flow's.
- **No goals without a coordination space.** A repository in no group creates a coordination space of its own to use them.
- **A goal is never deleted through `nxs`.** A goal written by mistake is `dropped`, and stays readable.
- **Proposals are stored, not made.** The `proposed` kind exists from this RFC on; the flow that writes and confirms proposals is priority 2.

## Migration

Nothing to migrate. v1 has no goals: its board holds one repository's tickets, ordered by priority, and nothing in it names something the tickets serve. RFC 0002 and RFC 0008 carry v1's priority over, and it keeps its meaning (see "What v1's priority becomes"). A group of repositories starts on v2 with no goals. Every ticket's `focus` is `none` until a goal is put into focus and tickets name it.

## Open questions

None. The owner answered them on 2026-11-20 (NEX-58 to NEX-64), and the answers are written into the text above.

## Decisions

- **A role session never reads the coordination space** ("A member who reads one space but not the other", "A worked example", "What it does not do"). Decided by the owner on 2026-11-20 (NEX-82), with RFC 0009 (#158). This RFC had given role sessions a row of their own "once it reads the coordination space", after NEX-47; under NEX-82 a role session sees only copies of the requests its tickets wait for, in an ad-hoc space pm gives it, so it always has `focus: null`. Why: pm, a writer of the coordination space, cannot add members there, and a coder would read every request and goal of the group. *Not taken:* role sessions as readers of the coordination space.

## History

- 2026-11-17: published as a draft in #161.
- 2026-11-20: decided by the owner (NEX-58 to NEX-64), in #161.
- 2026-11-20: what a role session sees of goals corrected to RFC 0009's final design (see "Decisions"), in #173 (NEX-83).
