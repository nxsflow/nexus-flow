# RFC 0009: Work across repositories

Status: Draft

This RFC designs how several repositories coordinate their work, following `factory/GENESIS.md`, "Work across repositories" and "Goals set the focus": the coordination space that joins a group of repositories, the requests one repository's pm addresses to another's, the thread below each request, how a receiving pm turns a request into tickets of its own, how a pm learns of a request without polling, and how the goals in the coordination space reach each repository's ranking. It builds on RFC 0001, which already places the coordination space, its members and its sync. Once it is decided, a coding ticket can build the `requests` module: its collections in nexus-flow's model, the `nxs space` and `nxs request` commands, and their counterparts on the library seam.

## What it solves

The owner runs several repositories whose work depends on each other. In this RFC's worked example they are nexus-flow, central-infra, app-foundations and nexflowit-app, which the owner decided join soon (NEX-30; decision of 2026-11-15, "Work across repositories stays in v2's scope"). When nexflowit-app needs an endpoint that central-infra runs, its pm needs a way to ask that is durable, auditable and the same for every agent framework, because the frameworks' own session-to-session messaging differs and changes fast ([how sessions reach each other](../research/2026-11-14-how-sessions-reach-each-other.md)).

GENESIS.md sets the frame:

- **Each repository's tickets stay its own.** No pm writes into another repository's space; writing there would need the writer role on that space, which is too much.
- **A coordination space joins a group of repositories** and holds the group's goals and the requests between them. Every role that coordinates, pm above all, is a member there and may write.
- **A request is shaped like a ticket:** addressed to a member, such as the pm of another repository, with a status (open, accepted, declined, done) and a thread below it.
- **The receiving pm turns a request into tickets** in its own repository's space and links them to the request.
- **A pm subscribes to requests addressed to it,** and learns of one as soon as its copy of the coordination space syncs. A wake-up through the agent framework is optional, behind a small adapter.

RFC 0001 already decides the frame this RFC works in: a coordination space per group of repositories, a repository in at most one of them, its id committed in `nexus-flow.toml`, pm as a writer there (RFC 0001, "Spaces" and "Roles in a space"), one relay per member database (RFC 0001, "Sync"), and `nxs space join-coordination` (RFC 0001, "Creating and finding a space").

On 2026-11-20 the owner answered the questions this RFC asked (NEX-45 to NEX-50): a request is addressed to a repository; `nxs` enforces who may change a request's status; a request blocks the tickets that wait for it, for every reader, so role sessions read the coordination space; an admin of the coordination space adds each repository's pm; requests take readable keys as tickets do; and the command names stand as proposed. The answers are written into the text below. The third answer changes two decided RFCs, RFC 0001's roles table and derived state and RFC 0002's readiness, which each get a follow-up of their own.

Out of scope, each in its own RFC: the goal collection, what "in focus" means and how pm ranks tickets against goals (RFC 0005); the ticket collection, ticket ids, readiness and "on main" (RFC 0002); the daily note and what it reports about requests (RFC 0007). NoCentralDB's own design is in nocentraldb/nocentraldb, `docs/rfc/`, cited here as "NoCentralDB RFC" by number and section; an RFC number without that prefix is one of nexus-flow's own.

## The design

### The coordination space

A coordination space is a NoCentralDB space of the kind RFC 0001 calls coordination (RFC 0001, "Spaces"). It holds three collections of this RFC's module, `requests`, and the goals of RFC 0005. The storage crate refuses to write any of them into a repository space, and a ticket into the coordination space (RFC 0001, "The model").

#### Creating it

A developer creates a coordination space from a checkout of one of the group's repositories:

```
nxs space create-coordination [--name <repository name>]
```

It creates the space on the developer's device, which becomes its first admin (NoCentralDB RFC 0009, "Spaces"), writes the space id under `[space] coordination` in `nexus-flow.toml` (RFC 0001, "Creating and finding a space"), and writes the repository record of the checkout's repository (see "Repositories in the space"). It refuses when `nexus-flow.toml` already names a coordination space, since a repository belongs to at most one (RFC 0001, "Spaces"). As with `nxs init`, a relay that does not allow the developer to create spaces refuses the space, which then stays local until the developer is allowlisted (RFC 0001, "Creating and finding a space").

The person who creates it is normally the owner, whose devices are admins of every space (RFC 0001, "Roles in a space"). A space has no name in NoCentralDB (NoCentralDB RFC 0009, "Spaces"); the group needs none either, since a repository belongs to one group and the repository records say which repositories it joins.

#### Members and roles

The members and their roles are RFC 0001's, with one change: the owner's devices are admins, other developers' devices and each repository's pm are writers, manufakt.io is a writer as a device of the person's account (RFC 0001, "Roles in a space"), and every role session a pm starts (coder, legal, …) is a **reader** (decided by the owner, NEX-47). A role session must see when its ticket waits for a request, since that block crosses spaces (see "A request blocks the tickets that wait for it"); as a reader it sees requests, their threads and the goals, and cannot change any of them. Only pms and the people's devices write there. This replaces RFC 0001's rule that role sessions are not members of the coordination space (decided by the owner on 2026-11-17, NEX-38), and RFC 0001's roles table is rewritten to match in its own follow-up.

Who adds a role session to the coordination space is still open: pm adds its sessions to the repository space as their admin (RFC 0001, "Starting a role session"), but it is only a writer in the coordination space, and only an admin can add a member (NoCentralDB RFC 0008, "Roles and rights"). The open question "Who adds a role session to the coordination space" asks the owner. Until it is answered, a role session that is not yet a member reads the request on its ticket as `notAccessible`, and its ticket counts as blocked (see "A request blocks the tickets that wait for it"), so nothing is handed out on a guess.

Only an admin can add a member (NoCentralDB RFC 0008, "Roles and rights"). So whenever a repository gets a new pm, an admin of the coordination space adds it there, as RFC 0001's "Starting pm", step 4, does with `nxs member add <invitation> --role pm`. In a group whose repositories all belong to the owner, that is the owner's device in every case. In a team where another developer commissions a repository's pm, that developer's device stays a writer, as RFC 0001's table has it, and an admin of the coordination space adds their pm for them (decided by the owner, NEX-48). A team then waits for an admin at every pm replacement; the owner has asked for this to be made easier in a later version.

*Not taken:* making the developer who commissions each repository's pm an admin of the coordination space, which would let them add their own pm but also remove the other repositories' members.

#### Repositories in the space

Each repository in the group has one record in the collection `requestsRepository`, so that a request can be addressed to it by name and its pm can find what is addressed to it:

| Field | Type | Holds |
| --- | --- | --- |
| `name` | `nc.string` | The repository's short name, such as `central-infra`; by default the last path segment of the git remote `origin`, without `.git` |
| `space` | `nc.string` | The repository space's id, as `nexus-flow.toml` names it |
| `left` | `nc.boolean` | Set when the repository has left the group; unset otherwise |

A repository record is never deleted: requests to and from it link to it and stay readable after it leaves.

**Joining.** A developer who is already a member of the coordination space runs, in a checkout of the joining repository:

```
nxs space join-coordination <space id> [--name <repository name>]
```

RFC 0001 designs this command to record the space id in `nexus-flow.toml`. This RFC adds that it writes the repository's record, or clears `left` on a record whose `space` is this repository's, and that it refuses a `--name` another record already holds with another `space`. Two repositories that join offline at the same time can still end up with the same name; `nxs` then shows both, and addressing a request by that name fails as ambiguous until one is renamed with `nxs space join-coordination <space id> --name <new name>` run again. The repository's pm must then be added by an admin, as above, before it can read the space.

**Leaving.** `nxs space leave-coordination`, run by a member in a checkout of the leaving repository, sets `left` on its record and removes `coordination` from `nexus-flow.toml`. `nxs` then refuses new requests to it. Requests already addressed to it keep their status; the requesting pms see that the repository left and decide what to do with them. Removing the repository's pm and devices from the coordination space stays an admin's step with `nxs member remove` (RFC 0001, "Ending a session's member"), since only an admin can remove a member, and a removed member keeps its copy of everything it read (NoCentralDB RFC 0009, "Invitations and members").

*Not taken:* deleting the record when a repository leaves, which would leave its requests pointing at nothing; and a record per member instead of per repository, which would need rewriting each time a pm is replaced.

#### What each repository needs locally

A repository that takes part in a group needs nothing beyond RFC 0001's frame:

- `nexus-flow.toml` names the coordination space under `[space] coordination`.
- Its pm's member database holds both spaces, since one member database holds the member's view of every space it belongs to (RFC 0001, "The local data on disk"), so pm reads requests and its own tickets locally, in one database.
- Every repository of the group syncs through the same relay, since a member database syncs all its spaces through one (RFC 0001, "Sync").

### The request

A request is one object in the collection `requestsRequest` of the coordination space:

| Field | Type | Holds |
| --- | --- | --- |
| `title` | `nc.string` | What is asked, in one line |
| `body` | `nc.string` | The request in full, Markdown |
| `from` | `nc.link("requestsRepository")` | The repository that asks |
| `to` | `nc.link("requestsRepository")` | The repository asked |
| `status` | `nc.oneOf(["open", "accepted", "declined", "done"])` | The request's status (see "Status") |
| `goal` | `nc.link("goalsGoal")` | The goal the request serves, if any (see "Goals and ranking") |
| `openedBy`, `openedAt` | `nc.string` | The member id that opened it and when, as RFC 3339 UTC |
| `statusAt` | `nc.string` | When `status` last changed, as RFC 3339 UTC |

The model declares its indexes: `byTo` on `to`, `status` and `openedAt` with `equal: 2`, which gives a repository's incoming requests of one status oldest first; `byFrom`, the same on `from`, for its outgoing requests; and `byGoal` on `goal` and `status` with `equal: 2`. Every query runs through one of them (NoCentralDB RFC 0005, "Indexes").

`from`, `to` and `goal` are links, not references, since all three targets live in the same space as the request (NoCentralDB RFC 0004, "Links").

`openedBy`, `openedAt` and `statusAt` are written by the member that writes the request, and NoCentralDB does not check them against the change's signature. They are the author field RFC 0001 brings as a stop-gap, together with the time of the change, and go once NoCentralDB can say who signed a change and when (RFC 0001, "Who decided"; requirement R6).

#### Addressed to a repository

GENESIS.md addresses a request "to a member, such as the pm of another repository". This RFC addresses it to the repository, through its record, and whoever is that repository's pm reads it: the member that runs `nxs` as pm in a checkout whose `nexus-flow.toml` names the repository space in the record's `space`. pms are replaced, sometimes several times while a request is open, and each new pm is a new member with a new id (RFC 0001, "Starting pm"); a request addressed to a member id would be lost with every replacement, and the requesting pm would have to know who the other repository's pm is today.

The same rule says who acts for the requesting side: any member that runs `nxs` in a checkout of the `from` repository. Every request therefore has two sides, and every member working in a repository of the group sees its requests as **incoming** (`to` is its repository), **outgoing** (`from` is) or neither.

manufakt.io has no checkout: it acts for a repository through the library seam, which takes the repository record as an argument where `nxs` reads it from `nexus-flow.toml` (see "Library seam").

A request is addressed to a repository only, never to a person (decided by the owner, NEX-45). A question for the owner goes into a ticket with the `needs-owner` label, as GENESIS.md asks ("Decide first, then build"), so the owner reads one queue of questions, not a second inbox beside the tickets and the daily note.

*Not taken:* a request addressed to a member id (lost when pm is replaced); a request addressed to a person, such as the owner, through a record of its own beside the repositories (a second inbox for the owner); and a request addressed to a role in a repository other than pm. A pm turns every request into tickets and opens lanes for every role (GENESIS.md, "Roles and lanes"), so a request to a legal role goes to its repository's pm, who hands it to a legal session as a ticket.

#### Status

A request moves between four statuses:

| From | To | Who | With |
| --- | --- | --- | --- |
| (new) | `open` | the requesting side | `nxs request create` |
| `open` | `accepted` | the receiving side | `nxs request accept` |
| `open` or `accepted` | `declined` | the receiving side | `nxs request decline --reason <text>`; a reason is required |
| `accepted` | `done` | the receiving side | `nxs request done` |
| `declined` or `done` | `open` | the requesting side | `nxs request reopen --reason <text>`; a reason is required |

`done` means the receiving side has delivered what was asked; the receiving pm decides when that is, since a request may need less than every ticket it led to, or more. `nxs request show` lists the receiving repository's tickets for the request, with their "done" and "on main" states (RFC 0002), so pm decides with them in view. The requesting side that wants to drop a request says so in its thread, and the receiving side declines it; withdrawing is not a status of its own, because GENESIS.md names four.

Rights in NoCentralDB apply to a whole space, so any writer of the coordination space can write any status on any request (NoCentralDB RFC 0008, "Roles and rights"). The table above is `nxs`'s rule: `nxs` and the library seam refuse a transition that is not in it, with the error `request_transition_not_allowed`, but a member that writes through another client is not stopped (decided by the owner, NEX-46). A rule that NoCentralDB enforces would need rights per object, which it does not offer and will not (NoCentralDB RFC 0008, "Roles and rights"); `nxs`'s rule gives every agent the same protocol at no cost. A role session is a reader of the coordination space, so it cannot change a request through any client. *Not taken:* no rule at all, with the thread as the only record, which would let a requesting pm mark its own request done.

Each transition also writes an entry into the request's thread (see "The thread"), with the new status and the reason, so the request's history stays readable even though NoCentralDB cannot yet list an object's changes (requirement R6).

Two members who change the status at the same time write two values; the last writer wins (NoCentralDB RFC 0004, "Basic values"), both thread entries stay, and `nxs` reports the request with `conflict: true` until the next status change (NoCentralDB RFC 0009, "Conflicts, lost removals and link states").

#### The thread

Below each request is a thread of entries, each an object in the collection `requestsComment`:

| Field | Type | Holds |
| --- | --- | --- |
| `request` | `nc.link("requestsRequest")` | The request |
| `kind` | `nc.oneOf(["comment", "status"])` | A comment, or the record of a status change |
| `status` | `nc.oneOf(["open", "accepted", "declined", "done"])` | For `kind: status`, the new status |
| `body` | `nc.string` | The comment, or the reason for a status change, Markdown |
| `author`, `at` | `nc.string` | The member id that wrote it and when, the stop-gap as above |

Its index `byRequest` on `request` and `at`, with `equal: 1`, reads a thread in order. A thread entry is never changed after it is written: `nxs` offers no command that edits or deletes one, so concurrent comments never conflict and the thread is the request's history. Either side, and any other member of the coordination space, may comment with `nxs request comment <request> --body <text>`.

A thread lives in the coordination space, so it is shared with every member there, unlike a ticket's comments. The rule against secrets and personal data in records applies to it as to every record (GENESIS.md, "What it must never do").

*Not taken:* the thread as one text field on the request that each comment appends to (two comments written at the same time would conflict, and the last writer would win); and the thread kept as ticket comments in the receiving repository (the requesting side cannot read that space).

#### Request ids

A request has two ids, as a ticket does (decided by the owner, NEX-49). Its object id is NoCentralDB's, and every link and reference stores it. Its **key** is the readable id people and agents use, formed exactly as RFC 0002 forms a ticket's key ("Ticket ids"): the creating member's tag in the coordination space, a dot, and a number that counts up per tag, such as `ka.7`, written once when the request is created and never again by `nxs`. The request gets the field `key` (`nc.string`) and the index `byKey` on it, `equal: 1`. Every member of the coordination space gets a tag there from `nxs member add`, as RFC 0002 gives every member one in a repository space, and the same rules hold: tags are reused once freed, two members given one tag offline mint the same keys, and `nxs` then names both requests by their object ids until pm retags one of the members.

A key is unique only within its space, so the request `ka.7` and a ticket `ka.7` of a repository can both exist. The `nxs request` commands read keys in the coordination space and the `nxs ticket` commands in the repository space, so a command never mistakes one for the other; a key written in a ticket's text is resolved in the repository space only (RFC 0002, "Mentions"), so a request's key there links nothing, or a ticket that happens to share it. `nxs` accepts a request's object id wherever it accepts its key.

*Not taken:* the object id alone, accepted by a unique prefix as git accepts a short commit hash, which works without a counter but is not readable.

### From a request to tickets

A NoCentralDB link points only into its own space, so a ticket in a repository space cannot link to a request in the coordination space. It refers to it through a reference, `nc.ref`, which carries the request's space id and object id (NoCentralDB RFC 0005, "References into another space"; RFC 0001, "Spaces"). The ticket collection gets one field and one index for it, added to RFC 0002's collection by this RFC, additively:

| Field | Type | Holds |
| --- | --- | --- |
| `request` | `nc.ref("requestsRequest")` | The request the ticket serves or waits for, if any |

with the index `byRequest` on `request`, `equal: 1`, `scope: "space"`, which finds a repository's tickets for one request in one lookup, whatever the reference's state (NoCentralDB RFC 0005, "References into another space").

The receiving pm turns a request into tickets in four steps:

1. It reads the request and its thread, and asks in the thread what is unclear.
2. It accepts the request, or declines it with a reason.
3. It creates the tickets in its own repository space as RFC 0002 designs, and links each one with `nxs request link <request> <ticket>`, which sets the ticket's `request` reference. When the request names a goal and the ticket names none, the same command sets the ticket's goal reference as RFC 0005 designs it, so the ticket is ranked by the same goal in its own repository.
4. When the work is delivered, it marks the request `done`.

The requesting pm may link its own tickets to the same request with the same command, for a ticket that waits for what it asked. The `request` field means "serves" in the receiving repository and "waits for" in the requesting one; `nxs` tells them apart by whether the request's `to` is the repository the ticket's space belongs to, and shows it as `relation: "serves"` or `relation: "waits"`.

**What each side sees.** Every member of a repository space is also a member of the coordination space, a role session as a reader, so each resolves the reference to the request (state `ok`) once its copy holds it (NoCentralDB RFC 0004, "How a reference resolves"). A coder reads what to build from its ticket, which pm writes from the request, and reads the request and its thread for context. The requesting side never sees the receiving side's tickets, since it is not a member of that repository space; it follows the request's status and thread, which the receiving pm keeps current.

#### A request blocks the tickets that wait for it

A ticket whose `request` names a request addressed to another repository (`relation: "waits"`) is blocked until that request is `done`, for every reader (decided by the owner, NEX-47). The block crosses spaces, so every member that reads the ticket must see it, which is why role sessions read the coordination space (see "Members and roles"). A ticket that serves a request (`relation: "serves"`) is not blocked by it.

- **The rule.** A waiting ticket is blocked while its request's status is anything but `done`, and also while the reference does not resolve to `ok`: the reader's copy does not hold the request yet, or the reader is not a member of the coordination space. A reader that cannot see the request therefore never sees the ticket as ready; it errs on the side of waiting. Its JSON carries the flag `waits_for_request` among RFC 0002's `flags`, beside `blocked: true`.
- **Who resolves it.** Only a pm resolves the block, since role sessions cannot write in the coordination space: the receiving pm by marking the request `done`, or the waiting repository's pm by unlinking its ticket with `nxs request unlink <ticket>`, for example after the request was declined and the work is done another way. The waiting pm may also reopen a declined request.
- **Where it is computed.** The storage crate computes it with the rest of the readiness behind the seam (RFC 0001, "Derived state"), reading the request through the ticket's reference in the member's own database. NoCentralDB's computed fields never follow a reference (NoCentralDB RFC 0005, "References into another space"), so this part of readiness stays nexus-flow's own even once readiness within a space moves to NoCentralDB (requirement R7).
- **What it changes in the decided RFCs** is listed under "Changes to the decided RFCs".

*Not taken:* readiness within the repository space only, with the request's status shown beside the ticket and pm holding it back (pm could hand out a waiting ticket by mistake, and a coder would not see the block); and a block only for readers that can resolve the request, with readiness `null` for the others (two readers of one ticket would compute two answers).

*Not taken:* a link from the request to the tickets (a link cannot cross spaces), and a list of references on the request (NoCentralDB has no field of many references; NoCentralDB RFC 0005, "References into another space"). A ticket refers to one request at most, for the same reason; a ticket that would serve two requests is split, or names one and mentions the other in its text.

### Learning of a request

A pm learns of a request when its copy of the coordination space holds it, never by asking the network (GENESIS.md, principle 3, and "Limits"):

- **Reading.** `nxs request list` answers from the local database, through `byTo` and `byFrom`. With no flags it lists the requests of the pm's repository, incoming and outgoing, that are `open` or `accepted`.
- **Watching.** `nxs request watch` opens the member's database and keeps it open, so it syncs while it runs (RFC 0001, "Sync"). It prints one JSON line per request of the repository that is `open` or `accepted` (`change: "snapshot"`), then one line per change to a request or thread entry of the repository, local or from another member, as the change arrives. It is built on NoCentralDB's change event, which reports the space, collection and object ids each fold touched (NoCentralDB RFC 0009, "Subscribe"); `nxs` reads the objects and keeps those of its repository. Because it starts with the snapshot, a pm that was offline misses nothing: what arrived while it was away is in the snapshot.
- **`nxs request watch --once`** exits with status 0 after the first change that came from another member, after printing it. A framework that re-invokes a session when a background command exits wakes the session through it, with no adapter.

**The wake-up adapter is optional.** An adapter is a small program per agent framework that reads `nxs request watch`'s lines and wakes a session in that framework's own way (GENESIS.md, "Architecture and stack": "small adapters per framework, for wake-ups only"). This RFC fixes the line format the adapters read and ships none; each adapter is its own ticket, for the framework that needs it. A pm with no adapter learns of a request the next time it reads.

*Not taken:* a timer that runs `nxs request list` (that is polling, which GENESIS.md asks to avoid), and a mailbox at the relay (the relay cannot read content, and must not learn who asks whom; NoCentralDB RFC 0007, "What the relay sees").

### Goals and ranking

Goals live in the coordination space (GENESIS.md, "Goals set the focus"), so every pm in the group reads the same goals, from its own copy. RFC 0005 designs the goal collection, what "in focus" means and how pm ranks its tickets against the goals. This RFC adds two things:

- **A request may name the goal it serves,** through its `goal` link, which is a real link because goals and requests share the coordination space. `nxs request create --goal <goal>` sets it.
- **Requests that serve a goal in focus come first.** `nxs request list` orders the requests that serve a goal in focus before the others, then by `openedAt`, oldest first, so a receiving pm takes on first what serves a goal in focus (GENESIS.md: "work for a goal goes first"). The ranking of tickets themselves stays RFC 0005's: the tickets a pm creates from a request carry the request's goal (see "From a request to tickets"), and are ranked like every other ticket of that goal.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Commands:**
  - `nxs space create-coordination [--name <repository name>]`, and `nxs space leave-coordination`. `nxs space join-coordination <space id>` (RFC 0001) gains `--name <repository name>`.
  - `nxs repository list`: the repository records of the coordination space.
  - `nxs request create --to <repository name> --title <text> [--body <text> | --body-file <path>] [--goal <goal>]`.
  - `nxs request list [--incoming | --outgoing] [--status <status>]... [--all]`; `--all` lists every request of the space, not only the repository's.
  - `nxs request show <request>`.
  - `nxs request accept <request> [--comment <text>]`, `nxs request decline <request> --reason <text>`, `nxs request done <request> [--comment <text>]`, `nxs request reopen <request> --reason <text>`.
  - `nxs request comment <request> --body <text>`.
  - `nxs request link <request> <ticket>` and `nxs request unlink <ticket>`.
  - Wherever a command takes `<request>`, it takes the request's key or its object id.
  - `nxs request watch [--once]`.
- **JSON output** (every timestamp RFC 3339 UTC, every member a NoCentralDB member id):
  - A repository, in `nxs repository list` and wherever a request names one: `name`, `space`, `left`.
  - A request, in `nxs request list` and as the head of `nxs request show`: `id`, `key`, `title`, `from`, `to` (each a repository), `direction` (`incoming`, `outgoing` or `other`), `status`, `conflict`, `goal` (`id`, `title` and `in_focus`, or `null`), `opened_by`, `opened_at`, `status_at`, `comments` (the number of thread entries of kind `comment`).
  - `nxs request show` adds `body`; `thread`, each entry with `id`, `kind`, `status` (or `null`), `body`, `author`, `at`; and `tickets`, the repository's own tickets whose `request` names it, each with `id`, `title`, `relation` (`serves` or `waits`), and the "done" and "on main" states as RFC 0002 names them.
  - `nxs request watch`, one object per line: `change` (`snapshot`, `opened`, `status`, `comment` or `link`), `origin` (`local` or `remote`), `request` as in `nxs request list`, and `entry`, the thread entry the change wrote, or `null`.
  - A ticket's JSON (RFC 0002) gains `request`: `id`, `key`, `relation`, and `status`, the request's status, or `null` for a reader who cannot resolve the reference; and the flag `waits_for_request` among its `flags` while the request blocks it.
  - The error `request_transition_not_allowed`, with the request's `status` and the transition asked for; and `repository_name_ambiguous`, with the matching repositories.
- **Library seam:** `Engine::coordination_create`, `Engine::coordination_join`, `Engine::coordination_leave`, `Engine::repositories`, `Engine::request_create`, `Engine::requests`, `Engine::request`, `Engine::request_transition`, `Engine::request_comment`, `Engine::request_link`, `Engine::request_unlink`, `Engine::request_watch` (a stream of the same events `nxs request watch` prints), with the types `Repository`, `Request`, `RequestStatus`, `RequestTransition`, `ThreadEntry` and `RequestEvent`. Every call that acts for a side takes the repository it acts for; `nxs` passes the one `nexus-flow.toml` names, and manufakt.io the one the person chooses. Each `nxs request` verb has its counterpart here (the decision on CLI and library parity stays in force).
- **Stored formats:** the collections `requestsRepository`, `requestsRequest` (with its `key` and the key format `<tag>.<number>` RFC 0002 defines) and `requestsComment` with their fields and indexes, the `request` field and `byRequest` index on RFC 0002's ticket collection, and the reader role of role sessions in the coordination space.
- **Commands named** as proposed (decided by the owner, NEX-50). *Not taken:* nouns of their own, such as `nxs coordination create`, `join` and `leave`, which read shorter but split the space commands RFC 0001 began under `nxs space`.

### Changes to the decided RFCs

The owner's answers change four places in RFCs that are already decided. Each is rewritten in a follow-up of its own, as a decided RFC changes (`docs/rfc/README.md`, "How a decided RFC changes"); this RFC states the rule they follow.

- **RFC 0001, "Roles in a space":** role sessions become readers of the coordination space, where the table says "not a member", and the paragraph after it says why (NEX-47). The decision of 2026-11-17 that role sessions are not members of the coordination space is superseded.
- **RFC 0001, "Derived state":** readiness is computed from the repository space, except that a ticket that waits for a request reads that request through its reference in the member's coordination space (NEX-47).
- **RFC 0002, "Derived state", "Commands, JSON and the seam" and "What it does not do":** a request blocks the tickets that wait for it, with the flag `waits_for_request`, where RFC 0002 says that neither a reference nor a request blocks; and the ticket's `request` object gains `key` (NEX-47, NEX-49).
- **RFC 0002, "Ticket ids":** the member tags and the key rule hold in the coordination space too, for requests, and `nxs member add` gives a member a tag there as in a repository space (NEX-49).

### What nexus-flow needs from NoCentralDB

**Met:**

- **A reference into another space, and an index on it.** `nc.ref` carries the space id and the object id and resolves to `ok`, `notAccessible` or `missing`, and an index on it finds the tickets of one request (NoCentralDB RFC 0005, "References into another space"; RFC 0004, "References").
- **A change event without the network.** `db.changes.subscribe` reports the space, collection and ids of each fold, local or remote (NoCentralDB RFC 0009, "Subscribe"), for `nxs request watch`.
- **One database for both spaces.** A member's database holds every space it belongs to (NoCentralDB RFC 0006, "Where the view lives"), so pm reads requests and its tickets in one place.

**Requests:**

- **R6. Who signed a change, and when** (RFC 0001's request). Until it lands, a request's `openedBy`, `openedAt` and `statusAt`, and a thread entry's `author` and `at`, are written by the writer and not checked. Once a change's author and time can be read, they are read from the signature instead, and the fields go.

This RFC asks NoCentralDB for nothing beyond what RFC 0001 already requests.

### A worked example

The owner runs four repositories: nexus-flow, central-infra, app-foundations and nexflowit-app. All four belong to the owner, whose devices are admins of every space, and all four sync through manufakt.io's relay.

1. **The space.** In a checkout of nexus-flow, the owner runs `nxs space create-coordination`. The device creates the space, writes its id into nexus-flow's `nexus-flow.toml`, and writes the record `nexus-flow`. The owner commits the file. The owner's device adds nexus-flow's pm: `nxs member add <pm's invitation> --role pm` (RFC 0001, "Starting pm").
2. **The other repositories.** In checkouts of central-infra, app-foundations and nexflowit-app, the owner runs `nxs space join-coordination <space id>`, which writes the records `central-infra`, `app-foundations` and `nexflowit-app` and the space id into each `nexus-flow.toml`, and adds each repository's pm.
3. **A goal.** The owner adds the goal "Win 20 paying customers for nexflow.it" in the coordination space, in focus, as RFC 0005 designs.
4. **A request.** nexflowit-app's pm finds that the billing provider needs a webhook endpoint, which central-infra runs. It runs `nxs request create --to central-infra --title "A webhook endpoint for the billing provider" --body-file webhook.md --goal <goal>` and links its own ticket that waits for it: `nxs request link ka.7 <ticket>`, where `ka.7` is the new request's key. From then on that ticket reads as blocked.
5. **The receiving pm learns of it.** central-infra's pm runs `nxs request watch --once` as a background command. Once its copy has synced the new request, the command prints the line with `change: "opened"` and exits, and the framework wakes the pm. Without that, the pm sees the request the next time it runs `nxs request list`, first in the list, since it serves the goal in focus.
6. **A question, then the tickets.** central-infra's pm asks in the thread which events the endpoint must accept; nexflowit-app's pm answers there. central-infra's pm accepts the request, creates two tickets in central-infra's space, and links both with `nxs request link`; both take the request's goal. It opens a coding lane for the first. The coder reads its ticket, and the request and its thread for context, as a reader of the coordination space.
7. **A request declined.** nexflowit-app's pm also asks app-foundations for a shared sign-in screen. app-foundations' pm declines it with the reason that the component library will not take app-specific screens, and points in the thread to the primitives nexflowit-app can build it from. nexflowit-app's pm creates its own ticket for the screen.
8. **Done.** When both of central-infra's tickets are on main, its pm marks the request `done` with a comment that names the endpoint. nexflowit-app's pm sees the change in its watch. Its waiting ticket was blocked until then, for pm and for every coder alike; now it is ready, and pm hands it out.
9. **nexus-flow.** nexus-flow's pm reads no request addressed to it and does nothing; `nxs request list --all` shows it the group's requests, should the owner ask.

## What it does not do

- **Every read is as fresh as the last sync.** A request reaches the receiving pm only once its copy of the coordination space has synced it, and a status change reaches the requesting pm the same way (GENESIS.md, "Limits"). `nxs request list` and `show` say what the local copy holds, not what the other repository has decided since.
- **A member who reads one space but not the other sees only half.** A role session that is not yet a member of the coordination space, or whose copy has not synced the request, reads the `request` reference on its ticket as `notAccessible` or `missing`; its ticket's JSON shows the request's status as `null`, and the ticket counts as blocked. The requesting side never sees the receiving side's tickets, only the status and the thread its pm keeps. A developer device that is a member of the coordination space but not of a repository space sees that repository's requests, but none of its tickets.
- **An offline pm answers nothing.** A request to a repository whose pm is offline, or has no pm running, stays `open` until a pm of that repository reads it. Nothing escalates it, nothing tells the requesting side that nobody is reading, and no request expires. The `openedAt` and `statusAt` show how long it has waited; whether the daily note reports requests that wait long is RFC 0007's.
- **Island mode reaches no other repository, yet.** Without a relay, repositories cannot sync their coordination space until NoCentralDB lets members find each other without one (RFC 0001, requirement R11).
- **The status rule is `nxs`'s, not NoCentralDB's.** Every writer of the coordination space can write any field of any request: a member that writes through another client can set a status the table above does not allow, or change a request's title. The thread shows the transitions `nxs` made; until R6, nothing shows who changed a field.
- **Who wrote what is a field, not a checked signature,** until R6.
- **A waiting ticket stays blocked until a pm acts.** Nothing unblocks it but the request being `done` or the waiting pm unlinking it: a declined request keeps its waiting tickets blocked until their pm decides what to do with them.
- **Readiness across spaces is nexus-flow's own,** computed in memory by the storage crate, since NoCentralDB's computed fields never follow a reference.
- **Role sessions read the whole coordination space.** Every reader holds a full copy (NoCentralDB RFC 0008, "Roles and rights"), so a coder reads every request and goal of the group, not only the one its ticket waits for.
- **One request per ticket.** A ticket refers to one request at most, since NoCentralDB has no field of many references.
- **A repository belongs to one group** (RFC 0001, "Spaces"). A request can therefore go only to a repository of the same coordination space.
- **Leaving keeps the copies.** A repository that leaves keeps its members' copies of everything they read until an admin removes them, and after (NoCentralDB RFC 0009, "Invitations and members").
- **A new pm needs an admin.** Only an admin of the coordination space can add a repository's new pm; until one does, that repository receives requests but reads none. In a group of one owner's repositories that admin is always the owner, so every pm replacement waits for the owner's device. The owner decided this for v2 and asked for it to be improved in a later version (NEX-48).
- **A wake-up needs an adapter,** or a framework that wakes a session when `nxs request watch --once` exits. This RFC ships no adapter.

## Migration

Nothing to migrate. v1 has no requests and no coordination space: its board holds one repository's work. The only work across repositories in v1 is chat's handover, in which a persona commissions a persona of another workspace on the same machine through a border thread (`docs/specs/E5c-chat-orchestration-api.md`, the scope guardrail and its lifted cross-repo clause). Those threads are chat data, which lapses in v2 without being moved (decision of 2026-11-15, "v1 chat data and personas lapse, after one review for lessons"). A group of repositories therefore starts on v2 with an empty coordination space, and an open handover in v1 becomes a request only when a pm opens one.

## Open questions

The owner answered questions 1 to 6 on 2026-11-20 (NEX-45 to NEX-50), and the answers are written into the text above. One question follows from the answer to the third:

7. **Who adds a role session to the coordination space.** Role sessions read the coordination space (NEX-47), but pm, who starts them, is only a writer there, and only an admin can add a member (NoCentralDB RFC 0008, "Roles and rights"). (a) pm is an admin of the coordination space, so it adds its sessions there as readers when it adds them to its repository space, and removes them when it archives them. (b) NoCentralDB lets a member add its own sub-agents to a space with a role no higher than reader, without being an admin, as a new requirement on NoCentralDB; pm stays a writer. (c) An admin's device, normally the owner's, adds every session. Option a works with NoCentralDB as it is, but every repository's pm could then add or remove any member of the group, the other repositories' pms included. Option b keeps pm's rights as they are and fits the chain person, pm, coder, but waits for NoCentralDB. Option c asks the owner to act for every session, which RFC 0001 ruled out for pairing. **Recommendation: a for v2, with b filed as a request on NoCentralDB so that pm can go back to writer once it lands.**

## History

- 2026-11-17: published as a draft in #158.
- 2026-11-20: the owner's answers to questions 1 to 6 (NEX-45 to NEX-50) written into the text, and question 7 added, in #158.
