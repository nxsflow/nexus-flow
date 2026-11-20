# RFC 0002: Tickets in NoCentralDB

Status: Decided (2026-11-20)

This RFC designs how the `tickets` module, v1's `flow`, keeps its data in a repository space: the collections and fields of projects and tickets, the edges between them, the references that leave the space, "done" and "on main" as `factory/GENESIS.md` defines them, the declared indexes every read goes through, how the derived states are computed, how a ticket's readable id is formed and how a mention of it links, and the `nxs` commands, JSON and `Engine` symbols that read and write it. It builds on RFC 0001, which sets the frame: one repository space per repository, one member per agent session, one model for the whole app, reads that never touch the network, and derived state computed behind the seam. Once it is decided, a coding ticket can declare the collections in `model/model.ts` and build the ticket commands on the seam, and RFC 0008 knows where every v1 field goes.

## What it solves

GENESIS.md's row for `tickets`: "projects and tickets with dependencies, readiness derived from the graph, references to other repositories and to the goals a ticket serves", shipping "the ticket states and fields the factory needs, nothing more". The owner and the agents need from it:

- **A ticket any session reads locally**, with its dependencies, its holder, its notes and its linked work, and the same answer from `nxs` and from manufakt.io (GENESIS.md, principle 3).
- **Readable ids.** A ticket is named in branches, commits, PR titles and other tickets within minutes of its creation, so its id must be short, final when the ticket is created, and a mention of it must link (owner, NEX-16).
- **"Done" is not yet "on main".** A ticket is closed with a linked PR, branch or artifact; whether it is merged into the team's base branch is derived from git, never set (GENESIS.md, "Done is not yet on main"; NEX-33).
- **Readiness from the graph**, never stored, with a dependency that is done but not yet on main still blocking.
- **What served the factory in Linear**, carried over (NEX-23, NEX-24, NEX-29): the short id, mention links, `blocks` and `related`, a close reason, the holder of a ticket, epics with children, and history with an actor.
- **Who did what.** Every change shows the role that made it and the person who commissioned that role (NEX-16; RFC 0001, "Who decided").

The owner decided before this RFC was written: one fixed ticket model for every repository, generated from the standard `issue-tracker` plugin, with what served us in Linear carried over, and no team-specific types or fields for now (NEX-23); "on main" against the team's configurable base branch, with ticket ranges in commit titles (NEX-33); closing needs a linked PR, branch or artifact (NEX-33); tickets may refer to people by their identity (NEX-33); and tickets is a full module, not an adapter to another tracker (NEX-33). On 2026-11-20 the owner answered the questions this RFC asked (NEX-54 to NEX-57, NEX-73): how a ticket's key is formed, which members get a tag and whether it is reused, what puts a ticket in the owner's queue, the resolutions of a closed ticket, and the shape of the commands. The answers are written into the text below.

**Scope.** This RFC designs what priority 1 needs: the storage of v1's tickets on NoCentralDB and the behaviour v1 has today, plus "done" and "on main", which GENESIS.md asks for and RFC 0001 already counts on. Behaviour v1 does not have, which priority 2 adds, is named here and designed later by rewriting this RFC (decision of 2026-11-15, "A module's storage in NoCentralDB is designed in the module's own RFC"): subscriptions to ticket comments, ranking by goals, and the proposal of which tickets serve a new goal (see "Left to priority 2").

Out of scope, each in its own RFC: the goals a ticket serves and the focus rule (RFC 0005), requests between repositories and the coordination space's side of them (RFC 0009), the daily note (RFC 0007), decision records (RFC 0003), and the move of v1's data (RFC 0008). A team's own ticket types and fields are not designed now (NEX-23); a later RFC can add them additively.

NoCentralDB RFCs are cited as "NoCentralDB RFC", by number and section; an RFC number without that prefix is one of nexus-flow's own, as in RFC 0001. Requirements on NoCentralDB continue RFC 0001's numbering.

## The design

### What the model is generated from

The ticket model is generated once from v1's `issue-tracker` plugin (`crates/facade/src/plugin/issue-tracker.toml`) and is the same in every repository (NEX-23). What the plugin declared becomes fixed:

- **Types:** `epic`, `bug`, `feature`, `chore`, `decision`, and one new type, `question` (see "Questions to the owner"). Only an `epic` may be a parent, and every other type has at most one `epic` parent: two levels, as the plugin's `[types.rules]` say. An epic is what Linear called a project; nexus-flow has no project collection of its own (NEX-24, NEX-29: projects were set on 30 of NOC's first 41 tickets and on none after, and epics with children took over).
- **Priority:** the plugin's labels `P0` to `P4`, stored as the ordinal 0 to 4, never as the label (decision "Priority is a plugin-named label at the surface but stored as the canonical ordinal"). An unset priority means "not set", and sorts after `P4` (NEX-29: "no priority" meant "not set", not "lowest").
- **Statuses:** v1's canonical lifecycle, `open`, `in_progress` and `closed`, with the plugin's display labels.
- **The `next` order:** status (claimed work first), priority, type (`epic`, `bug`, `question`, `decision`, `feature`, `chore`), due date, then the key.

v1's second plugin, `personal-todo`, and every plugin's custom and list fields are not part of the model; RFC 0008 says what becomes of a workspace that used them. A `decision` ticket is work that leads to a decision; the decision itself is a record of RFC 0003.

### Collections

The module owns four collections, all in the repository space (RFC 0001, "The model": the storage crate refuses to write them into a coordination space):

| Collection | One object per | Holds |
| --- | --- | --- |
| `ticketsTicket` | Ticket, epics included | Its fields, its edges to other tickets in the space, its labels |
| `ticketsNote` | Note on a ticket | The append-only comment stream, v1's notes |
| `ticketsWork` | PR, branch or artifact linked to a ticket | What closes a ticket as done, and what "on main" is derived from |
| `ticketsReference` | Reference from a ticket to an object in another space | Goals a ticket serves, tickets of other repositories it cites |

**Edges inside the space are fields of the ticket**, as NoCentralDB's own task model has them (NoCentralDB RFC 0005, "The schema in TypeScript"): `parent` is a single link, `dependsOn`, `contributesTo` and `mentions` are sets of links. A set of links merges as a set, add wins (NoCentralDB RFC 0004, "Sets"), which is exactly v1's observed-remove edge set, so the move keeps the edges' meaning. A dependency field is also what NoCentralDB's computed reach is written over (`nc.reach("dependsOn", …)`, NoCentralDB RFC 0005), so readiness can move into NoCentralDB without a migration once R7 lands. *Not taken:* an edge collection with one object per edge, which would make both directions indexable today but would have to be migrated into a links field for `nc.reach`, and would let two members create the same edge twice.

**References out of the space are objects of their own.** A ticket serves several goals, but a field holds one reference and there is no set of references (NoCentralDB RFC 0005, "References into another space"). One object per reference, with a single `nc.ref`, holds any number and can be indexed by its target, so "which tickets serve this goal" is one equality lookup. *Not taken:* a fixed number of reference fields on the ticket, and the target's id as a plain string, which NoCentralDB could not resolve.

**Notes and work links are objects of their own**, linked to their ticket, because each has its own author, time and, for work links, state.

### The ticket

`ticketsTicket`, with the v1 field each comes from. Stored names follow NoCentralDB's pattern (`^[a-z][A-Za-z0-9]{0,62}$`, NoCentralDB RFC 0005), so v1's `completion_criterion` is stored as `completionCriterion`; the JSON keeps v1's snake_case names.

| Field | Type | From v1 | Holds |
| --- | --- | --- | --- |
| `key` | `nc.string()` | the item id (`ab12.7x3k`) | The readable id, such as `ka.42`, set once when the ticket is created (see "Ticket ids") |
| `type` | `nc.oneOf([...])` | `type` | One of the six types above |
| `title` | `nc.string()` | `title` | One line |
| `description` | `nc.string()` | `description` | Why the ticket exists and its goal |
| `design` | `nc.string()` | `design` | The path to the goal; agent notes |
| `completionCriterion` | `nc.string()` | `completion_criterion` | The acceptance criteria |
| `status` | `nc.oneOf(["open", "in_progress", "closed"])` | `status` | The lifecycle |
| `resolution` | `nc.oneOf(["done", "canceled", "replaced"])` | new | Why a closed ticket is closed; unset while it is open |
| `replacedBy` | `nc.link("ticketsTicket")` | new | For `replaced`: the ticket that carries the work on |
| `closingComment` | `nc.string()` | `closing_comment` | The reason given at close |
| `closedAt` | `nc.string()` | `closed_at` | RFC 3339 instant of the last close |
| `archivedAt` | `nc.string()` | `archived` | RFC 3339 instant of archiving; unset when not archived |
| `priority` | `nc.int()` | `priority` | 0 to 4; unset means not set |
| `due` | `nc.string()` | `due` | RFC 3339 |
| `deferUntil` | `nc.string()` | `defer_until` | RFC 3339 |
| `assignee` | `nc.string()` | `assignee` | Who holds the ticket (see "Who holds a ticket") |
| `parent` | `nc.link("ticketsTicket")` | `parent` edge | The epic it belongs to |
| `dependsOn` | `nc.links("ticketsTicket")` | `dep` edges | The tickets that block it |
| `contributesTo` | `nc.links("ticketsTicket")` | `contributes_to` edges | Tickets it contributes to, without blocking |
| `mentions` | `nc.links("ticketsTicket")` | `mentions` edges | Tickets it cites or is related to (see "Mentions") |
| `labels` | `nc.set(nc.string())` | labels | The team's labels, such as `rfc` |
| `request` | `nc.ref("requestsRequest")` | new | The request in the coordination space the ticket serves or waits for, if any; designed in RFC 0009 (see "References out of the space") |
| `createdAt`, `updatedAt` | `nc.string()` | item timestamps | RFC 3339 instants of the create and of the last write, written by `nxs` |
| `createdBy` | `nc.string()` | op `author` | The member id of the member that created the ticket, set at creation and never changed (see "Who created a ticket") |
| `updatedBy` | `nc.string()` | op `author` | The member id of the last writer: RFC 0001's author stop-gap, until R6 |

A deleted ticket is a deleted NoCentralDB object (NoCentralDB RFC 0004, "Creating and deleting an object"), which replaces v1's `deleted` tombstone; links to it read **deleted**, and it can be restored. `nxs` offers no delete: closing is final in the factory's sense, and archiving hides a ticket. Deletion stays for a ticket created by mistake and for RFC 0008.

**Text fields are strings**, so two concurrent edits of one description keep the later one and the other as a readable conflict (NoCentralDB RFC 0004, "Basic values"), which is more than v1 kept. Rich text that merges character by character is reserved in NoCentralDB (`nc.richText()`) and can be added later as a new field. A description has no size limit of nexus-flow's own (NEX-29: a long report split over four comments was a friction).

**`nxs` writes every command as one NoCentralDB change** (`db.write`, NoCentralDB RFC 0009, "One write, one change"): a close sets `status`, `resolution`, `closingComment`, `closedAt`, `updatedAt` and `updatedBy` together, so no reader sees half a close.

### Indexes

Every query runs through a declared index (NoCentralDB RFC 0006, "Declared indexes"). Each index below is declared with `scope: "space"`, so a query reads one repository space even where a member holds several (NoCentralDB RFC 0009, "Queries and indexes"). A query skips deleted objects.

| Collection | Index | Fields (equality prefix in bold) | Read by |
| --- | --- | --- | --- |
| `ticketsTicket` | `byKey` | **`key`** | `show`, resolving a key in any argument, mentions, commit ranges |
| | `byStatus` | **`status`, `archivedAt`**, `key` | `list`, and the candidates of `next`, `ready`, `blocked`, `deferred`: the unarchived tickets of one status |
| | `byClosedAt` | **`status`, `archivedAt`**, `closedAt` desc | the `closed` lane, newest first |
| | `byClosedAtAll` | **`status`**, `closedAt` desc | `recap`, archived tickets included |
| | `byArchivedAt` | `archivedAt` desc | the `archived` lane; the read stops at the first unset value |
| | `byParent` | **`parent`**, `key` | an epic's children: `list --in`, the archive cascade, the epic tier of `next` |
| | `byAssignee` | **`assignee`, `status`** | what a member or person holds; the owner's queue |
| | `byType` | **`type`, `status`, `archivedAt`** | `list --type`, open questions to the owner |
| | `byRequest` | **`request`** | a repository's tickets for one request (RFC 0009) |
| | `byUpdatedAt` | `updatedAt` desc | what changed since a time, for the daily note (RFC 0007) |
| `ticketsNote` | `byTicket` | **`ticket`**, `createdAt` | a ticket's notes, oldest first |
| `ticketsWork` | `byTicket` | **`ticket`** | a ticket's work links |
| | `byBranch` | **`branch`** | the ticket of the checked-out branch |
| | `byUrl` | **`url`** | recording a PR's merge or close by its URL |
| `ticketsReference` | `byTicket` | **`ticket`**, `kind` | a ticket's references |
| | `byTarget` | **`target`**, `kind` | the tickets that serve a goal, or cite another repository's ticket |

A fixed key equals NoCentralDB's "unset" too (NoCentralDB RFC 0006, "Declared indexes": the equality prefix is written `k IS ?`), so `archivedAt` unset in an equality prefix selects the unarchived tickets.

**What no index can answer yet.** NoCentralDB cannot index a set or a field of many links (NoCentralDB RFC 0005, "Indexes"). Three reads need one: tickets carrying a label (`list --label`), the tickets that depend on, contribute to or mention a ticket (the reverse edges), and full-text search. The first two are answered from the graph the derived-state computation already loads (see "Derived state"): it reads the open and in-progress tickets through `byStatus`, and the label filter and the reverse edges are walks over what it read, closed tickets included through `byClosedAtAll` when a read asks for them. Search is answered from a full-text cache under `derived/<member id>/` (RFC 0001, "The local data on disk"), rebuilt when the database's version moves and deletable at any time. Both are stop-gaps in RFC 0001's sense (NEX-13): they live behind the storage crate's functions, and go once NoCentralDB indexes sets and many links (R14) and offers full-text search (R15). Until then a filter by label costs the tickets of the statuses it reads, not only its result.

RFC 0003 meets the same gap with join collections, one object per decision and topic, which it keeps even after R14 (RFC 0003, "The collections"). Tickets walk instead because readiness already loads every open ticket and its dependencies on each read, so the label filter and the reverse edges cost no extra read until R14, and because `dependsOn` must stay a field of links for NoCentralDB's computed reach (R7) to replace that walk without a migration.

### Ticket ids

A ticket has two ids. Its **object id** is NoCentralDB's, 128 random bits (NoCentralDB RFC 0002, "Object ids"): every link, mention and reference stores it, so a link never changes meaning. Its **key** is the readable id people and commits use: the creating member's tag, a dot, and a number that counts up per tag, such as `ka.42` (decided by the owner, NEX-54). The key is written once, when the ticket is created, and is final from then on, because branch names and commit titles carry it before any sync, and "on main" is derived from them (NEX-24: an option that renumbers fails here). `nxs` never writes the key again; NoCentralDB cannot stop a hostile member from doing so, and a key changed by hand is read as written.

There is no prefix such as `NEX-`: the owner chose against one, because it recalls Linear (NEX-54). A key is therefore unique within its repository space only, and a key cited from another repository is told apart by the reference that carries it, not by its text (see "References out of the space").

**The member tag** is two characters, lower-case ASCII letters and digits, starting with a letter, such as `ka` or `p7`. It is assigned when a member is invited: the inviter knows every tag in the space at that point and picks one that is free (NEX-54), and `nxs member add` writes it into the new member's `spaceMember` record (RFC 0001, "The role record"), as a field this RFC adds to that collection:

| Field | Type | Holds |
| --- | --- | --- |
| `tag` | `nc.string()` | The member's ticket tag; unset for a member that has none |

`nxs member add` picks a free tag unless `--tag <tag>` names one, and refuses a tag that a current member of the space holds. Every member gets a tag, a role session included, since a member that may write notes may also create tickets. A tag is free again once its member is removed, and the next member that gets it continues its numbers (decided by the owner, NEX-73). So a tag says which member created a ticket only while that member holds it; the ticket's `createdBy` says it for good (see "Who created a ticket"). The 936 tags (a letter, then a letter or a digit) bound the members of a space at one time, not over its life; `nxs member add` refuses with `no_free_tag` when every tag is held.

**The number** is the highest number among the keys with the member's own tag in its local copy, plus one. Only the member that holds a tag creates keys with it, and its own writes are always in its own copy, so its numbers never repeat while it alone holds the tag. A member that takes over a freed tag continues from the highest number its copy holds for that tag, so no key repeats (NEX-73); `nxs member add` therefore hands out a freed tag only to a member whose first sync has completed, and a new member waits for that sync before it creates a ticket. A member whose copy holds no key with its tag starts at 1.

**Two members with one tag.** Two inviters who pick a tag offline at the same time can give two members the same one, and the two then mint the same keys. A removed member's last tickets that reach the space only after its tag passed on can repeat a key in the same way. This is rare, and it is resolved when it happens (NEX-54): `nxs member list` marks both members with `tag_shared`, and pm files a ticket to give one of them a new tag with `nxs member retag <member> <tag>`. Keys already written keep their tag. Until then, and for the keys written meanwhile:

- A key is resolved through `byKey`. When two tickets hold the same key, `nxs` shows both with the first characters of their object ids, the way git abbreviates hashes (`ka.42~3f9a`), and an argument that names only `ka.42` fails with `ambiguous_key` and lists both. A mention or link written earlier keeps pointing at the object it was written for.

**A commit range**, `ka.13..ka.23`, names every ticket whose key has the same tag and a number from the first to the last, both included (NEX-33). A range never spans two tags.

`nxs ticket branch <key>` prints the branch name for a ticket: the key, a hyphen, and a slug of the title, at most 40 characters, with no person in it (NEX-24, NEX-29), for example `ka.42-tickets-in-nocentraldb`. A team may shorten or rename a branch; what matters is the key in it, and the branch a ticket's work is on is recorded as a work link.

### Mentions

`mentions` is one field for two uses that Linear had (NEX-24, NEX-29): a ticket key cited in text, and a relation added by hand (`related`, NOC's most used relation). Both are read from either end.

- **Parsed on write.** When `nxs` writes a ticket's `description`, `design`, `completionCriterion` or `closingComment`, or a note on it, it finds every token of the form `<tag>.<number>` whose tag a member of this space holds or held, standing alone as a word, resolves each through `byKey`, and adds the object ids it finds to the ticket's `mentions` in the same change. A key it cannot resolve, because that ticket has not arrived yet or never existed, is left as text. Editing the text later does not remove a mention: a mention records that the ticket was cited, as v1's did.
- **Added by hand,** with `nxs ticket update <key> --add-mention <key>`, without any text, at any time after creation, and removed the same way.
- **Read from both ends.** `show` lists what a ticket mentions and, as `mentioned_by`, the tickets that mention it; `nxs ticket mentioned-by <key>` lists the latter with `list`'s filters.

A key cited from another repository is not told apart by its text, since keys carry no prefix: if this space holds a ticket with the same key, the parser links that one. Another repository's ticket is cited through a reference of kind `cites`, which carries its space (see "References out of the space"), and its `label` names the repository, such as `nexus-flow ka.12`.

A duplicate is closed as `replaced`, with `replacedBy` naming the ticket it duplicates; `show` of that ticket lists it under `replaces`.

### Statuses, closing and reopening

A ticket is `open`, `in_progress` or `closed`, as in v1. Linear's Backlog and Todo were used inconsistently, with hops seconds apart (NEX-29), and both mean "open"; readiness, not a status, says whether a ticket can start.

- **Claim.** `nxs ticket claim <key>` sets `status` to `in_progress` and `assignee` to the member the command runs as, or to `--assignee`.
- **Close.** `nxs ticket close <key> --reason <text> [--as done|canceled|replaced] [--replaced-by <key>]` closes the ticket with a resolution, `done` by default, and the reason as its closing comment. A reason is required for every close, as in v1 (NEX-29: a cancel without a reason was drift). `done` requires at least one work link on the ticket, a PR, a branch or an artifact (NEX-33); `replaced` requires `--replaced-by`; `canceled` requires neither. The three resolutions are the owner's choice (NEX-56): `replaced` covers both a duplicate and a ticket folded into another, since both mean that the work continues in the ticket it names, and the closing comment says which.
- **Reopen.** `nxs ticket reopen <key> --reason <text>` sets `status` to `open`, clears `resolution`, `replacedBy` and `closedAt`, and adds the reason as a note. Work links stay.
- **Archive.** `nxs ticket archive <key>...` and `unarchive` keep v1's behaviour: archiving cascades down an epic's children and only when they are all closed; unarchiving cascades up to the parent.

"Done" is a closed ticket with the resolution `done`. "On main" is derived (see "Derived state").

### Work links

A `ticketsWork` object links one piece of work to its ticket, and one ticket may have several: NOC's tickets often had slices, each with its own PR (NEX-29).

| Field | Type | Holds |
| --- | --- | --- |
| `ticket` | `nc.link("ticketsTicket")` | The ticket |
| `kind` | `nc.oneOf(["pr", "branch", "artifact"])` | What is linked |
| `url` | `nc.string()` | The PR's or the artifact's URL |
| `branch` | `nc.string()` | For a PR or a branch: the branch name |
| `state` | `nc.oneOf(["open", "merged", "closed"])` | For a PR: its state as last recorded |
| `commit` | `nc.string()` | For a merged PR: the commit on the base branch, when known |
| `createdAt`, `createdBy` | `nc.string()` | When and by which member it was linked; `createdBy` is RFC 0001's author stop-gap |

`nxs ticket work <key> --pr <url> [--branch <name>] [--state open|merged|closed] [--commit <sha>]`, `--branch <name>` or `--artifact <url>` adds a work link, or updates the one with the same URL or branch. An artifact is a link to what a role that works outside the repository produced, such as a legal text or a published post (NEX-33).

**A PR's state is recorded, not fetched.** `nxs` reads no network, so it learns a PR's state from whoever sees it: pm records the merge when it merges, and a coder records the PR when it opens it. Recording a PR as `closed`, closed without merge, reopens its ticket in the same change, with a note that names the PR, when the ticket is closed as done and none of its other work links is an open or merged PR, a branch or an artifact (GENESIS.md: "A PR closed without merge reopens its ticket, and the ticket is flagged"). The flag is derived (`pr_closed_unmerged`, see "Derived state").

### Who holds a ticket

`assignee` holds an identity, as text with a prefix:

- **A session's member**, `member:<member id>`: the session that holds the ticket, such as "coder on NEX-24". Its role and its commissioner come from its `spaceMember` record and its certificate chain (RFC 0001, "Members"). Sessions are lanes and may hold several tickets in turn (NEX-29).
- **A person**, `account:<account fingerprint>`: the account a person's devices and agents are paired into (NoCentralDB RFC 0010). The owner is the person whose devices hold the role `owner` in the space's `spaceMember` records. Naming a person by their account is naming them by their identity, which tickets may do (NEX-33); nexus-flow stores nothing else about them.

`claim` writes the member the command runs as. A value without either prefix, which only the move from v1 writes, is v1's free-text assignee kept as written. The JSON shows the holder as `{"kind": "member" | "person" | "text", "id", "role"}`, with the role from `spaceMember`, `owner` for the owner's account, and `null` for text.

### Who created a ticket

A ticket records the member that created it in `createdBy`, written in the change that creates the ticket and never changed by `nxs` (decided by the owner, NEX-73). A role session needs write access to add notes, so it can create tickets too, and a prompt that forbids it is no guarantee; the creator must therefore be recorded on the ticket, and a reused tag cannot tell it.

NoCentralDB signs every change, the create included, but offers no way yet to read who signed one (RFC 0001, "Who decided"; R6). Until R6 lands, `createdBy` is what `nxs` shows, and nothing checks it against the signature. Once R6 lands, **the signer of the ticket's create counts**: `nxs` reads it, compares it with `createdBy`, and flags a ticket whose two differ with `creator_mismatch`. Unlike the other author fields, which go once R6 lands (NEX-42), `createdBy` on a ticket stays, so the creator reads from the ticket itself, in the move from v1 as well, where no NoCentralDB signature names v1's author.

### Questions to the owner

An owner question is a ticket of type `question`: one question that can be answered from a phone, with options and a recommendation (FACTORY.md, "What goes to the owner"). It is a type, not a label, because NOC's queue hung on a label that agents forgot on about fourteen of its "Owner:" tickets (NEX-29), and because only a question carries options, while an owner *task*, such as a setup step, does not. The work a question blocks depends on it through `dependsOn`, recorded when the question is filed, so readiness follows from the answer instead of from a note (NEX-29). How a question's text is laid out stays a template the team owns.

**The owner's queue** is derived: every open ticket of type `question`, and every open ticket assigned to the owner's account. `nxs ticket list --owner` reads it through `byType` and `byAssignee`. The owner chose these facts over a label, which agents forget (NEX-55).

### References out of the space

A link stays inside its space (NoCentralDB RFC 0004, "Links"), so a ticket refers to anything in another space through a `ticketsReference` object:

| Field | Type | Holds |
| --- | --- | --- |
| `ticket` | `nc.link("ticketsTicket")` | The referring ticket |
| `kind` | `nc.oneOf(["serves", "cites"])` | What the reference means |
| `target` | `nc.ref(["ticketsTicket"])` | The target's space id and object id (NoCentralDB RFC 0004, "References") |
| `label` | `nc.string()` | The target's readable name when the reference was written, such as `nocentraldb ka.12` or a goal's title |
| `createdAt`, `createdBy` | `nc.string()` | As for work links |

| Kind | Target | Means |
| --- | --- | --- |
| `serves` | A goal in the coordination space | The ticket serves this goal (GENESIS.md, "Goals set the focus"). A ticket may serve several goals, or none yet. |
| `cites` | Another repository's ticket | It is cited, without blocking: the cross-space form of a mention |

`target` names `ticketsTicket` only today, since every repository uses the same model: a ticket of another repository is a `ticketsTicket` in that repository's space. RFC 0005 adds `goalsGoal` to its targets when it declares that collection, and designs what serving a goal means for ranking; adding a target is an additive change (NoCentralDB RFC 0005, "The additive-change rule").

**A request is the ticket's `request` field, not a reference object.** RFC 0009 designs requests between repositories and adds the field and its index `byRequest` to the ticket; this RFC declares both. A ticket answers or waits for at most one request, and "which of our tickets belong to this request" is one equality lookup. Whether the ticket serves the request or waits for it follows from whether the request is addressed to this repository (RFC 0009, "From a request to tickets"). Waiting for another repository's work goes through a request, never through a reference to its ticket, since a pm is not a member of another repository's space (GENESIS.md, "Work across repositories").

**A reference resolves per reader.** It is `ok`, `notAccessible` or `missing` (NoCentralDB RFC 0005, "References into another space"), and the state differs from member to member: a coder is not a member of the coordination space (RFC 0001, "Roles in a space") and sees a goal as `notAccessible`. `label` is what such a reader shows instead of the target. Nothing derived from a reference is stored.

`nxs ticket ref <key> --serves|--cites <target> [--label <text>]` adds a reference, and `--remove` deletes it. `<target>` is `<space id>/<object id>`, or a key of a ticket in another space the member holds, which `nxs` then resolves and labels itself.

### Notes

A `ticketsNote` is v1's note: append-only text on a ticket, the channel in which the owner answers and pm restates (NEX-24).

| Field | Type | Holds |
| --- | --- | --- |
| `ticket` | `nc.link("ticketsTicket")` | The ticket |
| `body` | `nc.string()` | The text, Markdown |
| `replyTo` | `nc.link("ticketsNote")` | The note it answers, when it answers one |
| `createdAt`, `createdBy` | `nc.string()` | When and by which member; `createdBy` is RFC 0001's author stop-gap |

`nxs ticket note <key> <text | -> [--reply-to <note id>]` adds one. `nxs` never changes a note's body: a note is append-only, as in v1. Redacting a note deletes its object, which keeps its changes in the log and can be undone, as v1's tombstone could (NoCentralDB RFC 0004, "Creating and deleting an object"). Erasing what a note said, for example personal data written into it by mistake, is NoCentralDB's erasure (NoCentralDB RFC 0009, "Erasures") and is not designed here. `replyTo` was barely used in NOC (NEX-29) and costs one field.

### Derived state

Nothing below is stored (GENESIS.md, "Done is not yet on main"). The storage crate computes it on read, in memory, from the tickets and their links read through the indexes above, and from the repository's local git: RFC 0001's derived-state stop-gap, removed once NoCentralDB computes reach (R7, NEX-41).

- **`done`:** `status` is `closed` and `resolution` is `done`.
- **`on_main`:** for a done ticket, true when a commit reachable from the base branch names its key in its title, a range included, or when every work link of the ticket is an artifact (GENESIS.md: such a ticket is on main once it is done); false otherwise. For a ticket that is not done it is false. Each work link has an `on_main` of its own: a PR when its recorded `commit` is reachable from the base branch, or, without one, when a commit on the base branch names the key and the PR's number (`(#143)`, the squash default); a branch when its tip is reachable; an artifact always. The base branch is `nexus-flow.toml`'s `[git] base` (RFC 0001). A caller without a checkout gets `null`, not known, for both.
- **A dependency is resolved** when its ticket is done and on main, or closed as `canceled` or `replaced`, or deleted. A done dependency that is not yet on main still blocks: dependent tickets may stack on it, building on its branch, but count as unblocked only once it is on main (GENESIS.md). For a caller without a checkout, a done dependency resolves to `null`, and so does the readiness of the tickets it blocks.
- **Neither a reference nor a request blocks.** Readiness is computed from the repository space alone (RFC 0001, "Derived state"), and a reference's state differs from member to member, so a ticket that waits for a request would be ready for pm and not for a coder who cannot resolve it (NoCentralDB RFC 0004, "How a reference resolves"). `nxs` shows the request's status next to such a ticket instead, and pm, who holds both spaces, hands it out when the request is done (RFC 0009).
- **`ready(now)`:** the ticket is `open`, not archived, not in a dependency cycle, every dependency is resolved, and `deferUntil` is unset or not after `now`.
- **`blocked`:** `open`, not archived, and some dependency is unresolved, or it is in a cycle.
- **`deferred(now)`:** `open`, not blocked, with `deferUntil` after `now`.
- **`next(now)`:** the ready tickets and those in progress, in the order of "What the model is generated from", tiered as v1 tiers it: work that can close now, then each started epic with its open children, then the rest. Ranking by goals is priority 2.
- **Flags**, each a string in a ticket's `flags`: `cycle` (it is in a dependency cycle; no edge is ever removed for it, as in v1), `holder_gone` (it is `in_progress` and its assignee is a member that is no longer one; NOC-170 stayed In Progress for four days with nobody on it, NEX-29), and `pr_closed_unmerged` (a PR linked to it was closed without merge after its last close), and, once R6 lands, `creator_mismatch` (its `createdBy` differs from the signer of its create).

The derived state is as fresh as the last sync for the tickets, and as the last `git fetch` of the base branch for "on main" (GENESIS.md, "Limits").

### Commands, JSON and the seam

The ticket commands are one noun and a verb, `nxs ticket <verb>`, as RFC 0001's `nxs session start` and `nxs member add` are. A relation is changed by a flag of `update`, not by a third word, so no command is nested deeper than RFC 0001's flat commands (NEX-38). The owner chose this shape over nouns of their own (`nxs dep add`, `nxs note add`), which would add top-level nouns and make `note` ambiguous once decisions and status notes come, and over three words (`nxs ticket dep add`) (NEX-57). Every command takes a key wherever it takes a ticket, prints JSON by default and Markdown with `--md`, and keeps v1's flag names where v1 had them.

| Command | Does | v1 |
| --- | --- | --- |
| `nxs ticket create --type <type> --title <text> --description <text \| -> [--description-file <path>] [--priority <P0..P4 \| 0..4>] [--design <text>] [--dod <text>] [--due <time>] [--defer <time>] [--parent <key>] [--depends-on <key>]... [--label <label>]... [--assignee <identity>] [-q] [-]` | Creates a ticket, mints its key, parses its mentions | `create` |
| `nxs ticket update <key> [--set <field>=<value>]... [--add-dep <key>] [--remove-dep <key>] [--add-contributes <key>] [--remove-contributes <key>] [--add-mention <key>] [--remove-mention <key>] [--add-label <label>] [--remove-label <label>] [-]` | Changes fields and relations, in one change | `update`, `dep`, `contributes`, `mention`, `label` |
| `nxs ticket claim <key> [--assignee <identity>]` | Takes the ticket | `claim` |
| `nxs ticket close <key> --reason <text \| -> [--as done\|canceled\|replaced] [--replaced-by <key>]` | Closes it with a resolution | `close` |
| `nxs ticket reopen <key> --reason <text>` | Opens it again | new |
| `nxs ticket archive <key>...`, `nxs ticket unarchive <key>...` | Hides and shows, with v1's cascades | `archive`, `unarchive` |
| `nxs ticket note <key> <text \| -> [--reply-to <note id>]` | Adds a note | `note add` |
| `nxs ticket work <key> (--pr <url> \| --branch <name> \| --artifact <url>) [--state <state>] [--commit <sha>]` | Adds or updates a work link | new |
| `nxs ticket ref <key> (--serves \| --cites) <target> [--label <text>] [--remove]` | Adds or removes a reference | new |
| `nxs ticket show <key>` | One ticket, with its notes, links and derived state | `show` |
| `nxs ticket list [--status <status>] [--type <type>]... [--in <key>] [--label <label>] [--assignee <identity>] [--owner] [--sort id\|rank] [--limit <n>] [--paginate] [--token <token>]` | Lists tickets | `list` |
| `nxs ticket next`, `ready`, `blocked`, `deferred`, `closed`, `archived`, `recap` | v1's lanes, with v1's flags | the same |
| `nxs ticket mentioned-by <key>` | The tickets that mention one | `mention by` |
| `nxs ticket search <text>` | Full-text search over titles, texts and notes | `search` |
| `nxs ticket branch <key>` | Prints the ticket's branch name | new |
| `nxs ticket history <key>` | Every change with its author | `show`'s history; waits for R6 |

v1's `nxf thread` (links to chat threads) and `nxf entry` (list fields) have no counterpart: chat data lapses (NEX-20), and the model has no custom fields (NEX-23).

**JSON.** A ticket in `list` and the lanes is an object with `id` (the object id), `key`, `type`, `title`, `status`, `resolution`, `priority` (the ordinal, or `null`), `assignee` (`{kind, id, role}` or `null`), `parent` (a key or `null`), `labels`, `due`, `defer_until`, `created_at`, `updated_at`, `closed_at`, `archived_at`, `request` (RFC 0009's `{id, relation, status}`, with `status` `null` for a reader who cannot resolve the request, or `null`), and the derived `ready`, `blocked`, `deferred`, `done`, `on_main` (`true`, `false` or `null`) and `flags`. `show` adds `description`, `design`, `completion_criterion`, `closing_comment`, `replaced_by`, `replaces`, `deps` and `dependents` (each `{key, status, done, on_main}`), `children`, `contributes_to`, `mentions`, `mentioned_by`, `notes` (each `{id, body, reply_to, created_at, created_by}`), `work` (each `{kind, url, branch, state, commit, on_main, created_at}`) and `references` (each `{kind, space, id, label, state}`, where `state` is the reader's: `ok`, `not_accessible` or `missing`). Keys are snake_case, as v1's; empty arrays and unset values are written as `[]` and `null`, so every record of a command has the same keys. Ordering follows v1's determinism rules (RFC 0001, "Reading and writing"). An error is v1's error object, with the new codes `ambiguous_key`, `key_not_found`, `close_needs_work`, `close_needs_replacement`, `not_a_ticket_space` and, from `nxs member add`, `no_free_tag`.

**The seam.** Each command has its counterpart on `Engine` (the decision on CLI and library parity stays in force), with the derived state computed inside, so manufakt.io never re-implements a rule:

| Symbol | Does |
| --- | --- |
| `Engine::ticket_create`, `Engine::ticket_update`, `Engine::ticket_claim`, `Engine::ticket_close`, `Engine::ticket_reopen`, `Engine::ticket_archive`, `Engine::ticket_unarchive` | The writes above |
| `Engine::ticket_note`, `Engine::ticket_work`, `Engine::ticket_ref` | Notes, work links, references |
| `Engine::ticket` | One ticket by key or object id, as `show` |
| `Engine::tickets` | A `TicketQuery`: the filters of `list` and the lanes, with a page and its cursor |
| `Engine::ticket_next` | `next`, tiered |
| `Engine::ticket_search`, `Engine::ticket_branch` | Search, branch name |
| `Engine::ticket_history` | Once R6 lands |

The types they take and return are `Ticket`, `TicketKey`, `TicketQuery`, `Note`, `WorkLink`, `Reference`, `Holder` and `Derived`.

### Left to priority 2

Named here, designed later by rewriting this RFC:

- **Subscriptions to ticket comments.** A session learns that the owner answered as soon as its copy syncs, without polling (GENESIS.md, "Decide first, then build"). It is a `nxs ticket subscribe <key>` on `Engine::subscribe` (RFC 0001), which NoCentralDB already offers (R3).
- **Ranking by goals.** pm ranks tickets by the goals in focus, through the `serves` references (GENESIS.md, "Goals set the focus"); RFC 0005 designs the rule. Until then `priority` orders `next`, as in v1, and the move keeps it (NEX-29).
- **Which tickets serve a new goal.** A model's proposal to the owner when a goal is added (GENESIS.md); RFC 0005.
- **A request's tickets.** How pm turns a request into tickets and reports back; RFC 0009 designs it, on the ticket's `request` field above.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Commands:** the `nxs ticket` verbs and flags in the table above, and, on RFC 0001's member commands, `nxs member add --tag <tag>` and `nxs member retag <member> <tag>`; `nxs member list` gains `tag` and the marker `tag_shared`.
- **JSON output:** the ticket record, `show`'s additions, the `assignee` object, the `flags` values and the error codes above.
- **Library seam:** the `Engine::ticket*` symbols and their types above.
- **Stored formats:** the `request` field and `byRequest` index, which RFC 0009 brings and this RFC declares; the collections `ticketsTicket`, `ticketsNote`, `ticketsWork` and `ticketsReference` with their fields and indexes; the `tag` field this RFC adds to RFC 0001's `spaceMember`; the `type`, `status`, `resolution`, `kind` and `state` values; the key format `<tag>.<number>`; the `member:` and `account:` prefixes of `assignee`; and the branch-name rule.

### What nexus-flow needs from NoCentralDB

**Met:** single and many links in one space, with add-wins sets (NoCentralDB RFC 0004, "Links", "Sets"); references into another space, indexable by target (NoCentralDB RFC 0004, "References"; RFC 0005, "References into another space"); deletion that can be undone (NoCentralDB RFC 0004, "Creating and deleting an object"); an equality prefix that can fix a key to unset (NoCentralDB RFC 0006, "Declared indexes"); indexes per space (`scope: "space"`, NoCentralDB RFC 0005, "Indexes"); one change per write (NoCentralDB RFC 0009, "One write, one change"); and subscriptions (R3).

**Requests**, besides RFC 0001's R6 (who signed a change, for `history` and to retire the author fields) and R7 (computed reach, for readiness):

- **R14. Indexes over sets and many links.** An index whose rows are a set's elements or a field's links, so that "the tickets with label X" and "the tickets that depend on Y" are one equality lookup each. Until then they are walks over the tickets the derived state reads.
- **R15. Full-text search.** A declared full-text index over string fields. Until then `search` reads a rebuildable cache of nexus-flow's own.

## What it does not do

- **A key can collide** when two inviters give two members one tag offline. A collision shows both tickets with a suffix and makes the bare key ambiguous; it never re-points a link, and pm retags one member once it is seen.
- **A key is unique in its repository only.** Without a prefix, `ka.12` in two repositories names two tickets, and a key cited in text from another repository links to this repository's ticket of that key if there is one.
- **A mention can be a false match.** A word of the form `<tag>.<number>` whose tag this space uses, written for another reason, links the ticket of that key.
- **A mention of a ticket that has not arrived** is not linked: the key stays text, and a later write of the same text links it.
- **A key is final by `nxs`'s rule, not NoCentralDB's,** and so is a note's body. A writer that does not use `nxs` can change either; the change is signed and, once R6 lands, visible in the history.
- **Concurrent edits of one text keep one,** and the other as a readable conflict, until NoCentralDB's rich text exists.
- **A PR's state is as fresh as the last time someone recorded it.** `nxs` does not ask GitHub; a PR merged or closed without anyone recording it is still `open` in the ticket.
- **"On main" is as fresh as the last `git fetch`** of the base branch, and `null` for a caller without a checkout, and so is the readiness of a ticket whose only open blocker is done but not yet known to be on main.
- **Waiting for another repository does not block readiness.** A ticket that waits for a request reads as ready once its own dependencies are; pm, who sees the request's status, holds it back until the request is done (RFC 0009).
- **Filters by label, reverse edges and search cost more than their result** until R14 and R15: they walk the tickets read for the derived state, or a local cache.
- **The history of a ticket cannot be read** until R6. Its changes are all in the space's log and signed; only reading them back with their authors waits.
- **Who wrote a ticket, note, work link or reference is a field,** not a checked signature, until R6 (RFC 0001, "Who decided"). A ticket's `createdBy` is checked against its create's signer once R6 lands.
- **The tags run out at 936 members at one time.** A space with more current members cannot add another until one is removed.
- **Every read is as fresh as the last sync** (GENESIS.md, "Limits"): a ticket another session claimed or closed since then reads as it was.

## Migration

This RFC moves no data; RFC 0008 designs the move, replaying every v1 op so history survives (NEX-19). It gives RFC 0008 the place of every field v1 stores for an item, an edge or a note:

| v1 | In v2 |
| --- | --- |
| Item id (`<prefix>.<code>`) | A new object id, and a key with the tag of the member that runs the move; the v1 id is kept in the first note so old references can be found |
| `type`, `title`, `description`, `design`, `completion_criterion`, `status`, `priority`, `due`, `defer_until`, `assignee`, `closing_comment` | The fields of the same meaning, in the table under "The ticket"; `assignee`, free text in v1, is mapped to an identity where RFC 0008 can, and kept as written otherwise (see "Who holds a ticket") |
| `closed_at`, `archived` | `closedAt`, `archivedAt` |
| `deleted` | Object deletion |
| `belongs_to` (retired in v1) and the `parent` edge | `parent` |
| Edges `dep`, `contributes_to`, `mentions` | `dependsOn`, `contributesTo`, `mentions` |
| Labels | `labels` |
| Notes, with author and tombstone | `ticketsNote`, with `createdBy`; a tombstoned note is a deleted note |
| Item timestamps | `createdAt`, `updatedAt` |
| The op log's `author`, `lamport`, `site`, `wall_clock` | The space's log, through RFC 0008's replay; read back with R6 |
| A closed item | `status` `closed`, `resolution` `done` where it has a work link, else `canceled`, with the closing comment kept; RFC 0008 decides how it finds a v1 item's work |
| `body` (E1's placeholder) | Never written by v1's CLI; dropped |
| Thread links (`nxf thread`) | Dropped with v1's chat data (NEX-20) |
| Chunks | Dropped: only nexflow.it's plugin wrote them, and nexflow.it stays on v1 (NEX-18) |
| Custom and list fields | Not in the model (NEX-23); RFC 0008 says what a workspace that used them gets |
| `next_page_cache` | Machine-local; `--paginate` keeps its own cache under `derived/<member id>/` |

## Open questions

None. The owner answered them on 2026-11-20 (NEX-54 to NEX-57, NEX-73), and the answers are written into the text above. For which members get a tag (NEX-73), two options were not taken: only the members that create tickets (devices, pm, manufakt.io), with a new pm taking over its predecessor's tag, because a role session that may write notes may also create tickets and a prompt that forbids it is no guarantee; and tags that are never reused, because one member per session would use up the 936 tags within months.

## History

- 2026-11-17: published as a draft in #160.
- 2026-11-20: decided by the owner (NEX-54 to NEX-57, NEX-73), in #160.
