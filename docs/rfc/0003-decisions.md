# RFC 0003: decisions in NoCentralDB

Status: Decided (2026-11-20)

This RFC designs how the `decisions` module, which replaces v1's `memory`, keeps its records in a repository space: the collections and their fields, how "in force", "superseded" and "on main" are derived, why no change can delete or rewrite a decision and where that guarantee ends, the indexes behind every filter, the `nxs` commands with their JSON, `nxs prime`, and the `Engine` symbols manufakt.io calls. It follows `factory/GENESIS.md`, "Decisions and rules" and "Done is not yet on main", inside the frame of RFC 0001. Once it is decided, a coding ticket can build the module, and RFC 0008 can move v1's memories and a repository's `factory/decisions/` folder into it.

## What it solves

A team that lets agents work for days needs to know what was decided, and an agent needs it before it starts (GENESIS.md, "What we build"). Today this repository keeps its decisions as Markdown files in `factory/decisions/` and reads them with `cargo xtask decisions` (`factory/DECISIONS.md`); nocentraldb keeps the same format. That format is the working model for this RFC. v1's `memory` module keeps durable notes instead, written with `nxm` and replayed by `nxs prime`.

The owner and the agents need:

- **A header that states the decision.** An agent that reads only the headers of the decisions in force knows every rule (GENESIS.md, "Decisions and rules").
- **A rule is a decision in force.** No memory file and no session-start hook: `nxs prime` prints the rules in force on demand.
- **A decision is never edited away.** A new decision supersedes it, and the old one stays readable, with what replaced it (principle 4).
- **Filters by ticket, topic, date, an index, and only what is in force,** read locally and fast (principle 3).
- **"Not yet on main"** for a decision that describes code, until its PR is merged.

The owner decided before this RFC was written that decisions live in NoCentralDB, not in git (NEX-33, after the review in NEX-30), in the repository space beside the tickets they link to (NEX-15), and how a v1 memory becomes a decision (NEX-21). The text below carries these.

Out of scope: the move of v1 memories and of a `factory/decisions/` folder into a space, which is RFC 0008's (this RFC gives the mapping it needs, under "Migration"); ticket ids, ticket states and the on-main rule for tickets (RFC 0002); goals (RFC 0005); requests between repositories (the RFC on work across repositories); a command that evaluates the log for stale or duplicate decisions, which GENESIS.md puts outside v2's scope.

NoCentralDB's design is cited as "NoCentralDB RFC", by number and section, as RFC 0001 does; an RFC number without that prefix is one of nexus-flow's own. Requirements R1 to R11 are RFC 0001's; this RFC adds R12 and R13.

## The design

### Where decisions live

Every decision lives in its repository's space (RFC 0001, "Spaces"), so a decision links to a ticket of the same repository with a real link (NEX-15). The storage crate refuses to write a decision into a coordination space (RFC 0001, "The model"). A decision that concerns a group of repositories is recorded in the repository whose ticket triggered it, and names the others' tickets, goals or requests by reference (see "What a decision is about").

### The collections

The module owns four collections, named by RFC 0001's rule. One decision is one object of `decisionsDecision` and the objects of the three other collections that its single write creates with it.

| Collection | One object per | Why it is its own collection |
| --- | --- | --- |
| `decisionsDecision` | Decision | The record itself |
| `decisionsTopic` | Topic of a decision | A set cannot be indexed (NoCentralDB RFC 0005, "Indexes"), and `--topic` needs an index |
| `decisionsSubject` | Ticket, goal or request a decision is about | Many links and references cannot be indexed either, and `--ticket` needs an index |
| `decisionsSupersession` | Decision that a newer one supersedes | "Superseded by" is a lookup from the older decision to the newer one, which needs an index on the older |

*Not taken:* topics as `nc.set(nc.string())`, tickets as `nc.links("ticketsTicket")` and supersedes as `nc.links("decisionsDecision")` on the decision itself. They read more naturally, but none of them can be indexed in NoCentralDB v1, so every filter would read the whole log. Because a decision and the objects that belong to it are written in one change and never changed again (see "Append-only"), the three join collections cannot drift from the decision.

RFC 0002 asks NoCentralDB for indexes over sets and many links (requirement R14). The join collections stay when R14 lands, and are the design, not a stop-gap. The model only grows (RFC 0001, "The model"), so they could not be removed, only deprecated. Moving to set fields would also need every decision written before R14 to gain those fields, and a decision takes no write after the change that creates it. Readers would then have to look in two places for the same data, with no gain. R14 would change nothing in this module.

#### `decisionsDecision`

| Field | Type | Holds | In a file today |
| --- | --- | --- | --- |
| `handle` | `nc.string()` | The decision's human-readable name, `YYYY-MM-DD-<ticket>-<slug>` (see "Handles") | The file name, without `.md` |
| `date` | `nc.string()` | The day the decision was made, `YYYY-MM-DD` | The heading's date |
| `tickets` | `nc.string()` | The heading's ticket part as written: `no ticket`, a list such as `NEX-9, NEX-10`, or a range such as `NEX-6 to NEX-13` | The heading's ticket part |
| `title` | `nc.string()` | The decision, stated in one line of at most 200 characters | The heading's title |
| `decidedBy` | `nc.string()` | Who decided, as written: a name, a role such as `pm` or `coder on NEX-36`, or a sentence such as "the coding agent on NEX-36, on Carsten's request" | `- Decided by:` |
| `why` | `nc.string()` | Why, usually one line | `- Why:` |
| `insteadOf` | `nc.string()` | The options not taken | `- Instead of:` |
| `body` | `nc.string()` | Everything below the fields, as Markdown: "The record", "The rule", any section of the writer's own; empty when there is none | The file's text below the field lines |
| `pr` | `nc.string()` | The URL of the pull request whose code this decision describes, if any | Not held today: a file arrives with its PR |
| `author` | `nc.string()` | The member id of the member that wrote the decision: RFC 0001's stop-gap until R6 | The commit's author |

Indexes, each with `scope: "space"`:

- `byDate`: `date` descending, then `handle` descending. `nxs decision list` reads it newest first and stops at `--since`; it prints oldest first, as today, which reverses the page in memory.
- `byHandle`: `handle`, `equal: 1`. Finds a decision by its handle.

A string field rather than `nc.richText()` for `body`, because rich text is reserved and not designed yet (NoCentralDB RFC 0005, "Field types"); a later RFC can add a rich-text field additively. No field is personal (see "Personal data and erasure").

#### `decisionsTopic`

| Field | Type | Holds |
| --- | --- | --- |
| `decision` | `nc.link("decisionsDecision")` | The decision |
| `topic` | `nc.string()` | One topic: a lowercase tag of letters, digits and `-`, starting with a letter or a digit |
| `date` | `nc.string()` | The decision's date, copied, so the index can order by it |

Index `byTopic`: `topic`, then `date` descending, `equal: 1`, `scope: "space"`.

A decision has at least one topic, as today.

#### `decisionsSubject`

What a decision is about: a ticket in its own space, or a ticket, goal or request in another space.

| Field | Type | Holds |
| --- | --- | --- |
| `decision` | `nc.link("decisionsDecision")` | The decision |
| `ticket` | `nc.link("ticketsTicket")` | A ticket in the same space, when the ticket exists there |
| `elsewhere` | `nc.ref(["ticketsTicket", "goalsGoal", "requestsRequest"])` | A ticket of another repository, a goal or a request, in another space |
| `label` | `nc.string()` | The subject's human-readable id as written, such as `NEX-22` (RFC 0002 designs ticket ids) |
| `date` | `nc.string()` | The decision's date, copied |

Indexes, each with `scope: "space"`: `byLabel` (`label`, then `date` descending, `equal: 1`), which serves `--ticket`, and `byTicket` (`ticket`, `equal: 1`), which a ticket's view uses to list the decisions about it.

`label` is always written, `ticket` or `elsewhere` when the subject resolves. A ticket named only by its label is not lost: a decision that names a ticket of a tracker the space does not hold, such as the Linear tickets this repository's decisions name today, keeps the label, and `--ticket NEX-22` finds it. The heading's range `NEX-6 to NEX-13` becomes one subject per ticket, and `tickets` keeps the range as written for the header. The first subject in the order of `tickets` is the decision's triggering ticket, the one its handle names.

A reference to another space stays unresolved for a reader who is not a member there (NoCentralDB RFC 0004, "References"); `nxs` shows its label and its state, `ok`, `notAccessible` or `missing` (NoCentralDB RFC 0005, "References into another space"). A role session is not a member of the coordination space (RFC 0001, "Roles in a space"), so a coder reads a goal a decision serves as its label only.

#### `decisionsSupersession`

| Field | Type | Holds |
| --- | --- | --- |
| `decision` | `nc.link("decisionsDecision")` | The newer decision |
| `supersedes` | `nc.link("decisionsDecision")` | The older decision it replaces |
| `date` | `nc.string()` | The newer decision's date, copied |

Index `bySuperseded`: `supersedes`, `equal: 1`, `scope: "space"`; and `byDate`: `date` descending, `scope: "space"`, which reads every supersession for "in force" (see below).

A decision can supersede several decisions, and be superseded by several, as today. A decision cannot supersede itself or a decision dated after it.

### Handles

Every decision has a handle, which agents and people type and read: `YYYY-MM-DD-<ticket>-<slug>`, the same name a file in `factory/decisions/` has today without `.md`, for example `2026-11-15-NEX-15-one-space-per-repository`. `<ticket>` is the triggering ticket's label, or `no-ticket`; `<slug>` is the title in lowercase words joined by `-`, cut at 60 characters on a word boundary, unless the writer gives a slug of their own with `--slug`.

The handle is written once with the decision, so a moved file keeps its name as its handle and every `Supersedes:` line and every reference in a text still leads to it. NoCentralDB cannot keep a field unique across members that write concurrently, so two decisions can share a handle; that needs the same day, the same ticket and the same title. A command that takes a handle refuses an ambiguous one and lists the matching decisions with their object ids, and every command takes an object id as well (32 lowercase hex digits, NoCentralDB RFC 0009, "The Rust SDK"). The writer of a decision gets a warning when its handle is taken already in its local copy: `nxs decision add` prints it on stderr, and a library caller reads the same through `Engine::decisions_with_handle` (NEX-237).

The owner chose this handle on 2026-11-20 (NEX-51). *Not taken:* the object id alone (never ambiguous, but unreadable in a prompt, and a moved file would lose the name that other decisions cite), a short prefix of the object id, as git abbreviates commits (short, but just as unreadable, with the same loss), and a running number such as `D-42` (members that write offline cannot agree on the next number).

### Writing a decision

A decision is written in one call, `nxs decision add`, which is one NoCentralDB write and therefore one change (NoCentralDB RFC 0009, "One write, one change"): the decision and its topics, subjects and supersessions arrive together on every member, or not at all.

The command checks before it writes, and refuses with a message that names the problem:

- `title`, `why`, `insteadOf` and at least one topic are present; `title` is one line of at most 200 characters, counted in characters. The limit is v1's for a memory's introduction, for the same reason: `nxs prime` prints every title in force to every session, so a title has to say the rule, not announce it.
- Each topic is a lowercase tag.
- `date` is a real calendar date, today's in UTC unless `--date` gives one. A decision recorded after the fact, such as one the owner made in a meeting, carries the day it was made.
- Each ticket label is a ticket id or a range of them, as today.
- Each decision it supersedes exists in the local copy, is not this decision, and is not dated after it.
- `pr`, when given, is an `https` URL.

`decidedBy` defaults to the role of the member that writes, from its role record (RFC 0001, "The role record"): `pm`, or `coder on NEX-42` for a coder on that ticket. An agent that records a decision the owner made writes the owner's name, as today. `author` is always the writing member's id and cannot be given.

### Append-only

**No change can delete or rewrite a decision through nexus-flow.** The module has no command and no `Engine` symbol that changes or deletes an object of its four collections, and the storage crate refuses, for every member and every caller, any write to an existing object of them: a field set, a delete or a restore. A decision is corrected by a new decision that supersedes it, and withdrawn by one that supersedes it and says so in its title.

This holds for every writer that goes through nexus-flow, which is every `nxs` and every embedder of the seam. It does not hold against a member that writes to the space with NoCentralDB's SDK directly, or with a modified `nxs`: rights apply to a whole space (RFC 0001, "What it does not do"), so any writer can set a field of a decision or delete it, and NoCentralDB today neither refuses that nor shows it (see "What it does not do"). What remains true is NoCentralDB's: every change is signed, the log is append-only (NoCentralDB RFC 0002, "The log on disk"), a sequential edit replaces the old value in the view but not in the log, a concurrent one stays visible as a conflict (NoCentralDB RFC 0004, "Late changes stay identifiable"), and a deletion can be undone (NoCentralDB RFC 0004, "Creating and deleting an object").

nexus-flow therefore asks NoCentralDB for **R12, write-once collections**: a collection the model declares as write-once, whose objects take field writes only in the change that creates them and cannot be deleted, so that every member parks a later set or delete as malformed, as it parks a malformed operation today. Once R12 lands, the decision collections declare it, and the guarantee holds against every writer. Until it lands, and as a check once it has, the storage crate shows tampering where it can see it:

- A decision field with a conflict reads with `"edited": true` in the JSON and the conflicting values under `"conflicts"`, read with `space.inspect` (NoCentralDB RFC 0009, "Conflicts, lost removals and link states").
- A decision that another decision supersedes, or that a subject or topic object links to, but that reads as deleted, is listed with `"deleted": true` instead of being dropped: `space.get` returns deleted objects (NoCentralDB RFC 0009, "Get by id"), and the join objects lead to it.
- Once R6 lands, a decision whose history holds a write after its creating change reads with `"edited": true` as well.

#### Personal data and erasure

GENESIS.md forbids writing personal data into decisions; referring to people by their identity is not personal data in that sense ("What it must never do"). No field of a decision is therefore marked personal (NoCentralDB RFC 0005, "Personal fields"), and a shred of a person reaches nothing in a decision.

The append-only guarantee meets its limit where personal data was written into a decision anyway, by mistake or because a person whose name a decision carries asks to have it erased. NoCentralDB can erase only fields marked personal, and does not reach free text (NoCentralDB RFC 0005, "Two limits", "Personal data in free text"). A field's value can be overwritten, but the log keeps it, so an overwrite hides the value without erasing it. nexus-flow cannot erase such a value today, and says so. It asks NoCentralDB for **R13, erasing an unmarked value**: a work order, signed off like a shred, that removes one field's value from every member's log, view and archive and leaves the change's signature checkable, as a shred does for a personal field (NoCentralDB RFC 0001, "The design in one page", shred and audit). Until R13 lands, nexus-flow offers nothing beyond this limit, and there is no exception to append-only (decided by the owner on 2026-11-20, NEX-53). *Not taken:* an admin-only `nxs decision redact <handle> --field <name>` that overwrites a field with "redacted" and records a decision that says so. It would hide the value from every reader of the view, but the value would stay in every member's log, so it would look like erasure without being one, and it would be the one exception to "a decision is never edited away".

### Derived state

Nothing below is stored. It is computed by the storage crate from the decision collections and the local git, the same way for `nxs` and for manufakt.io, as RFC 0001's derived state is ("Derived state").

**Superseded by** a decision are the newer decisions of its `decisionsSupersession` objects, found through `bySuperseded`.

**In force** is a decision that no decision supersedes. A rule is in force when it is made (GENESIS.md, "Decisions follow the same line"), so a decision is in force whether it is on main or not, and it stops being in force as soon as a decision that supersedes it is written, also one that is not yet on main. `--current` reads every supersession through `byDate` once and leaves out the decisions they name. That is an in-memory computation under RFC 0001's stop-gap: once NoCentralDB's computed fields and views can express it (R7), a view `inForce` replaces it.

Two decisions in force can contradict each other, for example when two members supersede the same decision at the same time, each with its own new rule. Nothing detects that; a later decision supersedes both. A command that looks for contradictions, stale decisions and duplicates is outside v2's scope (GENESIS.md, "Decisions and rules").

**On main.** A decision without `pr` has no code to merge, so it is on main once it is made, as a ticket closed with an artifact is (GENESIS.md, "Done is not yet on main"). A decision with `pr` is on main when either of two rules finds it there (decided by the owner on 2026-11-20, NEX-52):

1. **The pull request's commit.** A commit reachable from the base branch that `nexus-flow.toml` names has a title that ends with `(#<n>)`, as a squash merge on GitHub writes it, or starts with `Merge pull request #<n>`, as a merge commit does. `<n>` is the number at the end of the decision's `pr` URL; a URL that ends in no number skips this rule.
2. **The triggering ticket,** where the first rule finds no such commit: the decision is on main once its triggering ticket is on main by RFC 0002's rule. A decision with `no ticket` has no triggering ticket and follows the first rule only.

The first rule is precise for GitHub's squash and merge commits, which this factory uses; the second still ends in "on main" for a rebase merge or another forge's commit titles. Its cost: a ticket with several pull requests is on main after the first one merges, which may not be the one the decision describes. *Not taken:* either rule alone. The first leaves a decision "not yet on main" for ever after a rebase merge or on another forge; the second alone is wrong for every ticket with several pull requests, also where the first rule would have been right.

"On main" is derived from the local git and never fetched, as the ticket's state is (RFC 0001, "Derived state"), so it is as fresh as the last `git fetch`, and it is `null`, not known, for a caller without a checkout.

### Reading decisions

`nxs decision list` prints the index by default: one line per decision, its header, oldest first. `--full` prints every selected decision in full instead (decided by the owner on 2026-11-20). The header states the decision (GENESIS.md, "Decisions and rules"), so the index is what a reader needs first, and the whole log in full costs a session tens of thousands of tokens; a reader asks for the full text when it needs the reasons. The command takes the filters `cargo xtask decisions` takes today, with the same meaning:

| Flag | Reads through | Means |
| --- | --- | --- |
| none | `decisionsDecision.byDate` | The index of every decision, oldest first |
| `--full` | the same | Every selected decision in full instead of its index line |
| `--current` | `decisionsSupersession.byDate` in addition | Leave out superseded decisions |
| `--ticket <label>` | `decisionsSubject.byLabel` | Only the decisions about this ticket, goal or request |
| `--topic <tag>` | `decisionsTopic.byTopic` | Only the decisions with this topic |
| `--since <date>` | the date order of the index read | Only decisions made on or after this date |

The filters combine, with each other and with `--full`. The command reads through the index of the narrowest filter given, `--ticket`, then `--topic`, then the date, and applies the others to the decisions it found, in memory. "Superseded by" is computed over every decision, whatever the filters, as today.

*Not taken:* the full text by default with `--index` for the index, as `cargo xtask decisions` does today (decided by the owner on 2026-11-20). It makes the costly read the one a reader gets without asking.

`nxs decision show <handle or id>` prints one decision in full, with what it supersedes and what superseded it. It is the step from an index line to the reasons, so every index line carries the key `show` takes (see below).

Output is deterministic, JSON by default and Markdown with `--md` (RFC 0001, "Reading and writing"). One index line in JSON, as `nxs decision list` prints it by default:

```json
{
  "id": "6f1c…",
  "handle": "2026-11-15-NEX-15-one-space-per-repository",
  "date": "2026-11-15",
  "tickets": "NEX-15",
  "title": "One NoCentralDB space per repository holds its tickets, decisions and status notes",
  "in_force": true,
  "on_main": true,
  "superseded_by": []
}
```

One decision in full, as `nxs decision show` and `nxs decision list --full` print it:

```json
{
  "id": "6f1c…",
  "handle": "2026-11-15-NEX-15-one-space-per-repository",
  "date": "2026-11-15",
  "tickets": "NEX-15",
  "subjects": [
    { "label": "NEX-15", "kind": "ticket", "space": null, "object": "a03e…", "status": "ok" }
  ],
  "title": "One NoCentralDB space per repository holds its tickets, decisions and status notes",
  "decided_by": "Carsten",
  "why": "decisions and status notes point at tickets all the time, …",
  "instead_of": "a space per module per repository, …",
  "topics": ["nocentraldb", "spaces", "storage"],
  "body": "",
  "supersedes": [],
  "superseded_by": [],
  "in_force": true,
  "pr": null,
  "on_main": true,
  "author": "9b2d…",
  "edited": false,
  "deleted": false
}
```

- `subjects[].kind` is `ticket`, `goal` or `request`; `space` is `null` for a subject in the decision's own space and the other space's id otherwise; `object` is `null` for a subject known only by its label; `status` is `ok`, `notAccessible` or `missing`, as NoCentralDB reads it, or `null` with no `object`.
- `supersedes` and `superseded_by` list `{ "id", "handle" }`, in date order.
- `conflicts` appears only on an edited decision: per field, the values in conflict with the read value.
- `nxs decision list` prints `{ "decisions": [ … ] }` in date order, oldest first, same day by handle: index lines by default, full decisions with `--full`. An index line is a full decision cut to its first eight fields, so a reader that handles one shape handles the other.

In Markdown, a decision in full, as `nxs decision show` and `--full --md` print it, reads as a file in `factory/decisions/` reads today, heading, fields and body, followed by `> Superseded by …` when it is superseded and `> Not yet on main (<pr>)` when it is not on main; with `--full`, decisions are separated by a blank line, as today. An index line in Markdown is the line `cargo xtask decisions --index` prints today, `date · tickets · title`, followed by ` · superseded by <handles>` when it is superseded and ` · not yet on main` when it is not on main, and ending with ` · <handle>`, so an agent moving from the files to `nxs` reads the lines it knows, plus the key it passes to `nxs decision show`. That last part is the handle (NEX-51). Where two decisions share a handle, their index lines end with the object id instead, so every line names a key `show` accepts without refusing it.

### `nxs prime`

`nxs prime` prints what a session reads before it starts. Its decisions part is the index of the decisions in force: what `nxs decision list --current` prints, under the key `decisions` in JSON and the heading "Decisions in force" in Markdown. A line says ` · not yet on main` for a decision that is not on main yet, so an agent knows the rule holds while the code it describes may be missing from its checkout. The headers are enough to know every rule (GENESIS.md, "Decisions and rules"); an agent reads a decision in full with `nxs decision show` when it needs the reasons. The other parts of `nxs prime`'s output, such as the session's ticket, are designed by the RFCs of the modules they come from.

`nxs prime` reads the local copy only and does not sync (RFC 0001, "Reading and writing"). It writes no file and installs no hook (GENESIS.md, "Decisions and rules").

### The library seam

`Engine` gains, as RFC 0001 says each module RFC adds its reads and writes to it ("The library seam"):

| Symbol | What it does |
| --- | --- |
| `Engine::decision_add(NewDecision) -> Decision` | Writes a decision with its topics, subjects and supersessions, after the checks of "Writing a decision" |
| `Engine::decisions(&DecisionFilter) -> Vec<DecisionIndexLine>` | The index lines of the decisions the filter selects, with their derived state, in date order: `nxs decision list` |
| `Engine::decisions_full(&DecisionFilter) -> Vec<Decision>` | The same decisions in full: `nxs decision list --full` |
| `Engine::decision(&DecisionKey) -> Decision` | One decision by handle or object id; an ambiguous handle is an error that lists the matches |
| `Engine::decisions_with_handle(&str) -> Vec<ObjectId>` | The object ids of the decisions in the local copy that carry this handle; `nxs decision add` calls it after the write and warns on stderr when another decision holds the handle (decided by the owner, NEX-237) |
| `Engine::rules_in_force() -> Vec<DecisionIndexLine>` | What `nxs prime` prints as its decisions part |

`NewDecision`, `Decision`, `DecisionFilter` (`current`, `ticket`, `topic`, `since`), `DecisionKey` (a handle or an `ObjectId`), `DecisionIndexLine` and `Subject` are the types they take and return; their fields are the JSON fields above. The index and the full text are two symbols with two return types rather than one with a flag, so a caller's type says which it holds. Every `nxs decision` verb has its counterpart here (the decision on CLI and library parity stays in force). There is deliberately no symbol that changes or deletes a decision.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Commands:** `nxs decision add --title <text> --why <text> --instead-of <text> --topic <tag>… [--ticket <label>…] [--supersedes <handle or id>…] [--pr <url>] [--decided-by <text>] [--date <YYYY-MM-DD>] [--slug <slug>] [--body <text> | --body-file <path>]`, `nxs decision list [--full] [--current] [--ticket <label>] [--topic <tag>] [--since <YYYY-MM-DD>]`, `nxs decision show <handle or id>`, and the decisions part of `nxs prime`. Flat, as RFC 0001's commands are.
- **JSON output:** the fields under "Reading decisions", and `decisions` in `nxs prime`.
- **Markdown output:** the decision as a file reads today, and the index line of `cargo xtask decisions --index` with the decision's handle at its end.
- **Library seam:** the `Engine` symbols and types above.
- **Stored formats:** the collections `decisionsDecision`, `decisionsTopic`, `decisionsSubject` and `decisionsSupersession` with their fields and indexes, the handle format, and the 200-character title.

### What nexus-flow needs from NoCentralDB

Besides RFC 0001's requirements, of which this RFC rests on R6 (who signed a change, for `author` and for "edited") and R7 (computed views, for "in force"), and apart from RFC 0002's R14 (indexes over sets and many links), which this RFC does not need (see "The collections"):

- **R12. Write-once collections.** A collection the model declares write-once: its objects take field writes only in their creating change and cannot be deleted or restored, and every member parks any other write to them, so that the decision log is append-only against every writer, not only against those that go through nexus-flow.
- **R13. Erasing an unmarked value.** A work order, signed off as a shred is, that removes one field's value from the log, the view and the archive of every member and keeps the change's signature checkable, for personal data written by mistake into a field the schema could not mark.

## What it does not do

- **Append-only holds only through nexus-flow, until R12.** A writer of the repository space that uses NoCentralDB's SDK directly, or a modified `nxs`, can change or delete a decision. A concurrent change shows as a conflict and a deletion of a decision another object leads to shows as `deleted`, but a sequential edit of a field replaces the old value in the view, unflagged until R6; the old value stays only in the log.
- **A deleted decision nothing links to disappears from lists** until R12, because NoCentralDB's queries skip deleted objects. A decision without topics cannot exist through nexus-flow, so this needs a writer that deleted the topic objects too.
- **Personal data written into a decision cannot be erased,** until R13. Overwriting it hides it in the view but leaves it in every member's log. GENESIS.md forbids writing it, and nexus-flow does not scan text for it.
- **Two decisions in force can contradict each other.** Nothing detects it.
- **A decision is in force before its code is on main.** An agent reads a rule whose code its checkout may not have yet; `nxs prime` marks it.
- **A decision whose PR is closed without merging stays "not yet on main"** until a decision supersedes it or its triggering ticket is on main through another PR. `nxs` knows a PR only from the local git, which does not record that a PR was closed.
- **"On main" through the triggering ticket can be early.** When the first rule finds no commit, a decision follows its ticket, and a ticket with several PRs is on main after the first one merges.
- **"On main" is as fresh as the last `git fetch`,** and `null` for a caller without a checkout.
- **Two decisions can share a handle** when they are written concurrently with the same date, ticket and title. A command refuses the ambiguous handle and names the object ids.
- **A goal, a request or another repository's ticket reads as its label only** to a member who is not in that space.
- **The `author` field is written by the writer and not checked** against the change's signature, until R6 (RFC 0001, "Who decided"). `decidedBy` is free text and is never checked: it records who decided, which need not be who wrote.
- **"In force" is computed in memory** from every supersession in the space, until R7.
- **Decisions live in repository spaces only.** A coordination space holds none; a decision about a group of repositories is recorded in one repository and names the others by reference.
- **Every read is as fresh as the last sync** (GENESIS.md, "Limits"): a decision another member wrote, or one that supersedes a rule an agent is following, reaches the agent only once its copy has synced.

## Migration

This RFC moves no data. RFC 0008 moves a v1 workspace's memories and a repository's `factory/decisions/` folder into the repository space, with the mappings below, so that nothing a memory or a file holds is lost.

**A file in `factory/decisions/`**, as `factory/DECISIONS.md` describes it:

| In the file | Becomes |
| --- | --- |
| The file name, without `.md` | `handle` |
| The heading's date | `date` |
| The heading's ticket part | `tickets`, and one `decisionsSubject` per ticket it names, with its label; a range is expanded |
| The heading's title | `title` (every title in this repository's log fits in 200 characters today) |
| `- Decided by:` | `decidedBy` |
| `- Why:` | `why` |
| `- Instead of:` | `insteadOf` |
| `- Topics:` | One `decisionsTopic` per topic |
| `- Supersedes:` | One `decisionsSupersession` per file it names, to the decision whose handle is that file name |
| Any other field line, and the text below the fields | `body`, unchanged, in the order of the file |
| `> Superseded by …` in the output of `cargo xtask decisions` | Nothing: it is derived and was never in a file |
| The commit that added the file | `author` is the member that runs the move; the commit stays in git |

A file has no PR link, and its decision is on main, since the file itself was merged. The move writes each decision as one change, older decisions first, so a supersession always finds the decision it names.

**What reads the log today.** Until v2 carries decisions, the factory reads `factory/decisions/` with `cargo xtask decisions`, whose default is the full text and whose `--index` prints the index. This RFC does not change that tool. Three texts rely on its flags and change when the factory moves to `nxs`, which is the move's ticket, not this RFC's:

- `factory/roles/coder.md` and `factory/roles/pm.md` read the index with `cargo xtask decisions --current --index`, which becomes `nxs decision list --current`. They read the decisions of a ticket or a topic in full with `cargo xtask decisions --ticket <id>` and `--topic <tag>`, which become `nxs decision list --full --ticket <id>` and `--full --topic <tag>`: without `--full` they would get only the index lines. Both role files are under CODEOWNERS, so that change waits for the owner.
- `factory/FACTORY.md`, "The decision log", names the same commands, and `factory/CONFIG.md` and `factory/DECISIONS.md` describe `cargo xtask decisions` and its options.

`cargo xtask decisions --check`, the CI gate, reads the files and prints no decision, so the change does not touch it.

**A v1 memory**, by the mapping the owner decided in NEX-21: the introduction becomes `title`, the body becomes `body`, the date of the last write becomes `date`, the author becomes `decidedBy`, `tickets` is `no ticket`, the memory's key becomes a topic, and `why` and `insteadOf` read "carried over from memory". The handle is `<date>-no-ticket-<key as a slug>`. What becomes of a memory's category, scope, references to board items and reading position, which NEX-21's mapping does not name, and of forgotten memories, is RFC 0008's; this RFC's subjects can hold a memory's references to tickets as labels.

## Open questions

None. The owner answered them on 2026-11-20 (NEX-51 to NEX-53), and the answers are written into the text above: the handle under "Handles", when a decision's PR counts as on main under "Derived state", and that nothing is offered before R13 under "Personal data and erasure".

## Decisions

- **A taken handle is read through `Engine::decisions_with_handle`** ("The handle", "Commands, JSON and the seam"). Decided by the owner on 2026-11-27 (NEX-237). Why: `nxs` reaches the store only through the seam, so the warning needs a public read; a separate read keeps `decision_add`'s signature as written and serves a view too. *Not taken:* a `handle_taken` field in `decision_add`'s result and in the JSON `nxs decision add` prints; dropping the warning.

## History

- 2026-11-17: published as a draft in #159.
- 2026-11-20: `nxs decision list` prints the index by default and `--full` prints every decision in full, decided by the owner, in #159.
- 2026-11-20: decided by the owner: the handle (NEX-51), the PR's commit or else the triggering ticket for "on main" (NEX-52), and no redaction before R13 (NEX-53), in #159.
- 2026-11-27: the owner's answer to NEX-237 (`Engine::decisions_with_handle` for a taken handle) written into the text.
