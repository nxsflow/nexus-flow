# RFC 0007: status in NoCentralDB

Status: Decided (2026-11-20)

This RFC designs how the `status` module keeps the daily notes in NoCentralDB: in which space a note lives and why, the collections and their fields, how a note refers to the tickets, decisions, pull requests, requests and goals it reports on, what is derived on read and never stored, why no change rewrites a note and where that guarantee ends, the indexes behind every read, how a note's health is computed from the goals in focus, what manufakt.io reads to show the notes and what waits for the owner, and the `nxs status` commands with their JSON and the `Engine` symbols beside them. It follows `factory/GENESIS.md`, "The daily status", inside the frame of RFC 0001. Now that it is decided, a coding ticket can declare the collections in `model/model.ts` and build the commands on the seam; writing the note on a schedule is priority 2's and is named under "Left to priority 2".

## What it solves

GENESIS.md: "`status` writes one note a day, by default at 06:00 in the time zone the coordination space was set up in. It says what changed since yesterday, what works now, what waits for the owner and what was decided, so the owner reads one note instead of every transcript." manufakt.io shows the notes as a table of their own, and until it does, the owner asks a pm, "which lists the status updates and what waits for the owner, and works through them with it". The module ships "a daily-note template and a schedule" (GENESIS.md, "What we build").

The owner and the agents need:

- **One note a day per repository, kept,** that outlives every session (principle 4), read locally (principle 3).
- **A note that points at what it reports on:** the tickets, decisions and pull requests of the day, and the requests and goals of the group, so the owner can follow each line to its record instead of to a transcript.
- **What waits for the owner, now,** not only as it was when the note was written: an item the owner has answered since reads as answered.
- **A table manufakt.io can show** without parsing Markdown and without a server-side read: manufakt.io is a member of the spaces, as a device of the person's account (RFC 0001, "Members").
- **The team's own note.** The template is the team's, and an upgrade never overwrites it (GENESIS.md, "What it must never do").
- **No secrets, tokens or personal data in a note** (GENESIS.md, "What it must never do"): notes are copied and kept.

Today this repository's pm posts the note as a status update on a Linear initiative, with a health and five parts: what shipped, which pull requests wait for the owner, what broke or is blocked, what pm decided, and the open questions to the owner (`factory/FACTORY.md`, "How the owner hears back"). That note is the working model for this RFC.

The owner decided before this RFC was written that a repository's status notes live in its repository space, beside the tickets and decisions they point at (NEX-15; RFC 0001, "Spaces"). On 2026-11-20 the owner answered the questions this RFC asked (NEX-65 to NEX-69): no note of its own for a group of repositories, a health on every note whose way there is supported deterministically and oriented on the goals, corrections as new notes, pm and people's devices as the writers, and `nxs status <verb>` as the commands. The answers are written into the text below.

**Scope.** As for the other module RFCs, this RFC designs what priority 1 needs: the storage, and the commands that write and read it. Writing the note on a schedule, the stored default time zone, and a wake-up for pm are priority 2's, designed later by rewriting this RFC (decision of 2026-11-15, "A module's storage in NoCentralDB is designed in the module's own RFC").

Out of scope, each in its own RFC: tickets, the owner's queue and "on main" (RFC 0002), decisions and "in force" (RFC 0003), goals, their order and a ticket's focus (RFC 0005), which the health rule reads, the template's path in the factory's installed files (RFC 0006), requests and the coordination space (RFC 0009), and the move of v1's data (RFC 0008).

NoCentralDB's own design is cited as "NoCentralDB RFC", by number and section, as RFC 0001 does; an RFC number without that prefix is one of nexus-flow's own. Requirements R1 to R11 are RFC 0001's, R12 and R13 RFC 0003's, R14 and R15 RFC 0002's. This RFC adds none.

## The design

### Where a note lives

**A note lives in its repository's space.** Each repository's pm writes one note a day about its repository, into its repository space (NEX-15). The storage crate refuses to write a status collection into a coordination space (RFC 0001, "The model").

The reason is what a note points at. A note reports on tickets and decisions, and both live in the repository space. A link reaches only objects in its own space (NoCentralDB RFC 0004, "Links"), so only a note in the same space links to them with a real link, which resolves for every reader of that space and which an index can hold. The writer is the repository's pm, which is admin of that space (RFC 0001, "Roles in a space") and sees all of its tickets.

**The group's view is a read, not a note.** An owner with several repositories in one coordination space reads all their notes of one day at once. A member database holds every space its member belongs to, and an index declared without `scope: "space"` spans all of them (NoCentralDB RFC 0006, "Declared indexes"; RFC 0009, "Queries and indexes"). The owner's devices and manufakt.io are members of every repository space of the group (RFC 0001, "Roles in a space"), so one query through `statusNote.byDateAcrossSpaces` (see "Indexes") lists every repository's notes of a day, and `nxs status list --all-spaces` prints them. A repository without a coordination space gets a note all the same.

The owner chose that a group gets no note of its own (NEX-65). *Not taken:*

- **The coordination space only, one note per group.** One note to read, but it could not link to any ticket or decision: every line would be an `nc.ref` (NoCentralDB RFC 0005, "References into another space"), which no reader resolves unless it is a member of that repository space, and which pm would write for repository spaces it is not a member of. It needs one writer for the group, while each pm sees only its own repository. And a repository without a coordination space would have no note.
- **Both: a note per repository and a summary note per group,** written by one pm of the group into the coordination space. Two records of the same day that can disagree, a second writer that must wait for every repository's note, and lines that are references, unresolved for most readers. The read across spaces above gives the owner the same view without storing it twice.

### The collections

The module owns three collections, named by RFC 0001's rule. One note is one object of `statusNote` and the objects of `statusItem` and `statusGoal` that its single write creates with it.

| Collection | One object per | Why it is its own collection |
| --- | --- | --- |
| `statusNote` | Note | The note itself: its day, its period, its headline and its text |
| `statusItem` | Line of a note that reports on something | A note points at many records; many links cannot be indexed (NoCentralDB RFC 0005, "Indexes"), each line carries its own kind and text, and "which notes reported on this ticket" needs an index on the ticket |
| `statusGoal` | Goal in focus the note's health was computed from | What the health rule saw for each goal, kept with the note, since the tickets it counted move on (see "Health") |

*Not taken:* the lines as fields of the note, such as `tickets: nc.links("ticketsTicket")`: a set of links carries no kind and no text per element, and cannot be indexed until R14. RFC 0003 keeps its join collections for the same reason, after R14 too ("The collections"), and so does this RFC: a line is written once with its note and never changes (see "Correcting a note").

#### `statusNote`

| Field | Type | Holds |
| --- | --- | --- |
| `date` | `nc.string()` | The day the note reports on, `YYYY-MM-DD`, in `timeZone` |
| `timeZone` | `nc.string()` | The IANA time zone the day is counted in, such as `Europe/Berlin` |
| `since` | `nc.string()` | The start of the period the note covers, RFC 3339 in UTC |
| `until` | `nc.string()` | The end of the period, RFC 3339 in UTC: when the note's facts were gathered |
| `repository` | `nc.string()` | The repository's short name, as RFC 0009 forms it: the last path segment of the git remote `origin`, without `.git` |
| `health` | `nc.oneOf(["onTrack", "atRisk", "offTrack"])` | The note's health: the computed health, or the writer's when it overrides it (see "Health"); unset when neither gives one |
| `healthComputed` | `nc.oneOf(["onTrack", "atRisk", "offTrack"])` | The health the rule computed when the note was written; unset when the rule has no goal to go by |
| `healthWhy` | `nc.string()` | Why the writer's health differs from the computed one; set only when it does |
| `healthBasis` | `nc.oneOf(["noCoordinationSpace", "noGoalInFocus", "noWorkForGoals"])` | Why the rule computed no health; unset when it computed one |
| `headline` | `nc.string()` | The note in one line of at most 200 characters |
| `body` | `nc.string()` | The note's text, Markdown, as the team's template asks for it (see "The template") |
| `corrects` | `nc.link("statusNote")` | The earlier note this note corrects, if any (see "Correcting a note") |
| `author` | `nc.string()` | The member id of the member that wrote the note: RFC 0001's stop-gap until R6 |

`headline` is what manufakt.io's table and `nxs status list --index` show in one row, so it has a limit, as a decision's title has (RFC 0003, "Writing a decision"). `body` is a string, not `nc.richText()`, which NoCentralDB reserves and has not designed (NoCentralDB RFC 0005, "Field types"); a rich-text field can be added later, additively. No field is marked personal (see "Secrets and personal data").

#### `statusItem`

A line of a note: one thing the note reports, of one kind, pointing at at most one record.

| Field | Type | Holds |
| --- | --- | --- |
| `note` | `nc.link("statusNote")` | The note |
| `kind` | `nc.oneOf(["changed", "works", "waits", "decided", "blocked"])` | Which part of the note it belongs to (below) |
| `position` | `nc.int()` | Its place within its kind, from 1 |
| `text` | `nc.string()` | The line, one line of at most 200 characters |
| `ticket` | `nc.link("ticketsTicket")` | A ticket of this repository, when the line is about one |
| `decision` | `nc.link("decisionsDecision")` | A decision of this repository, when the line is about one |
| `pr` | `nc.string()` | A pull request's URL, when the line is about one |
| `elsewhere` | `nc.ref(["requestsRequest", "goalsGoal", "ticketsTicket"])` | A request or a goal in the coordination space, or another repository's ticket, when the line is about one |
| `label` | `nc.string()` | The target's readable name when the line was written: a ticket key such as `ka.42`, a decision's handle, `#160` for a pull request, a request's or a goal's title |
| `date` | `nc.string()` | The note's date, copied, so the indexes can order by it |

A line points at one record or at none: a line such as "CI was red for two hours this morning" points at nothing. A line about a ticket and its pull request points at the ticket; the pull request is found from the ticket's work links (RFC 0002, "Work links"). `label` is always written for a line that points at something, so a reader who cannot resolve the target still reads its name, as a decision's subject keeps its label (RFC 0003, "`decisionsSubject`").

The kinds are GENESIS.md's four parts of the note and the Linear note's "what broke or is blocked":

| Kind | Means | In the Linear note today |
| --- | --- | --- |
| `changed` | What changed since the last note: shipped, merged, closed | What shipped |
| `works` | What works now that did not before | (in the prose) |
| `waits` | What waits for the owner: a question, a pull request to review, a decision to make | The pull requests that wait for the owner; the open questions to the owner |
| `decided` | What was decided without the owner | What pm decided |
| `blocked` | What broke or is blocked | What broke or is blocked |

The kinds are fixed, so manufakt.io and `nxs status waiting` can read them the same way in every repository; the team shapes the prose of the note in its template. A kind added later is an additive change (NoCentralDB RFC 0005, "The additive-change rule"), and an old reader shows an unknown kind as written (NoCentralDB RFC 0005, "Field types": `nc.oneOf` keeps an unknown option).

`elsewhere` names `goalsGoal` and `requestsRequest`, which RFC 0005 and RFC 0009 declare. Should this RFC's collections reach the model before theirs, `elsewhere` names only the collections that exist then, and each RFC adds its own as a target when it declares it, which is additive.

### How a note refers to what it reports on

| What | How the line points at it | Resolves for |
| --- | --- | --- |
| A ticket of this repository | `ticket`, a link | Every member of the repository space |
| A decision of this repository | `decision`, a link | Every member of the repository space |
| A pull request | `pr`, its URL | Everyone: a URL is text. Its state comes from the ticket's work link with that URL (RFC 0002, `ticketsWork.byUrl`), if one records it |
| A request or a goal | `elsewhere`, a reference into the coordination space | Members of the coordination space: pm, the owner's devices, manufakt.io, and role sessions once added there as readers (RFC 0009; RFC 0005, "A member who reads one space but not the other") |
| Another repository's ticket | `elsewhere`, a reference into that repository's space | Members of that space only |

A pull request is a URL rather than a link to a work-link object, because a note may report a pull request that no ticket records, such as a merge of `main` into the base branch. A reference resolves per reader to `ok`, `notAccessible` or `missing` (NoCentralDB RFC 0005, "References into another space"), and nothing derived from it is stored, as in RFC 0002 ("References out of the space").

### The template

The template is a Markdown file the team owns, in git, beside the factory's files: `factory/status-note.md`. It is a prompt for the writer, not a form nexus-flow fills in: it says what the note's `body` covers and in what order, and the writer, normally pm, writes the body by it. nexus-flow reads nothing inside it, as it reads nothing inside the factory's other files (RFC 0006, "The factory's files"); it only reports, in `nxs status draft`, whether the file exists and where.

nexus-flow ships a default, whose text the owner decides in its own ticket, as for every default template (GENESIS.md, "What has to come back through me"). It is installed and kept as RFC 0006 keeps the factory's defaults: `factory/status-note.md` is one more path in the shipped list, written by `nxs factory install` when it is missing and never overwritten once the team changed it (RFC 0006, "Installing the shipped defaults"). The template is in git, not in a space, because the owner reviews it in a pull request, as the factory's files (NEX-14).

The lines of the note are not part of the body. The Markdown of a note is its headline, its health, its body, and then its lines under one fixed heading per kind (see "Reading notes"), so the template shapes the prose, and the lines keep the shape manufakt.io reads.

*Not taken:* a template with placeholders nexus-flow fills in, such as `{{waits}}` (nexus-flow would read and own the format of a file the team owns, and every team's change would have to keep the placeholders intact); and the template in the repository space (the owner would review it outside a pull request).

### Writing a note

pm writes a note in two steps: `nxs status draft` gathers the facts, pm writes the note from them by the template, and `nxs status write` stores it.

**`nxs status draft [--since <time>] [--date <YYYY-MM-DD>] [--time-zone <zone>]`** reads the local copy only and prints a draft: the note's JSON (see "Reading notes") without its ids, with `headline` and `body` empty, `health` and `health_computed` set to the health the rule computes and `goals` to what it computed it from (see "Health"), and `items` filled with what can be gathered without judgement. Its period:

- `until` is now, in UTC.
- `since` is `--since`, or else the `until` of the newest note in the repository space that no note corrects, or else 24 hours before `until`.
- `time_zone` is `--time-zone`, or else the machine's local zone (`TZ`, else the system's); `date` is `--date`, or else the day `until` falls on in that zone.

Its lines, each with its kind and target, in the order of each read:

| Kind | Gathered from | Read through |
| --- | --- | --- |
| `changed` | Tickets closed in the period, with their resolution and "on main" | RFC 0002, `ticketsTicket.byClosedAtAll` |
| `changed` | Tickets that changed in the period and are not closed | RFC 0002, `ticketsTicket.byUpdatedAt` |
| `decided` | Decisions dated in the period | RFC 0003, `decisionsDecision.byDate` |
| `waits` | The owner's queue now | RFC 0002, "Questions to the owner" |
| `waits` | The repository's requests whose status changed in the period, incoming and outgoing, for a member of the coordination space | RFC 0009, `requestsRequest.byTo`, `byFrom` |
| `blocked` | Tickets flagged `cycle`, `holder_gone` or `pr_closed_unmerged` | RFC 0002, "Derived state" |

`works` stays empty in a draft: what works now is a judgement, not a fact a query finds. Pull requests that wait for the owner's review are not gathered either: whether a pull request waits for the owner is the forge's state and the merge gate's (RFC 0006, "The running state"), and `nxs` reads no network; pm adds them as `waits` lines with their URLs. The draft also carries `template`: the template's path and whether it exists. A draft is deterministic: the same copy, period and options give the same draft.

**`nxs status write <file | ->`** reads a note in the draft's shape from a file or standard input and writes it as one NoCentralDB write, which is one change (NoCentralDB RFC 0009, "One write, one change"): the note and its lines arrive together on every member, or not at all. It checks before it writes, and refuses with a message that names the problem:

- `headline` is present, one line of at most 200 characters; each line's `text` is one line of at most 200 characters.
- `date` is a real calendar date; `time_zone` is a known IANA zone; `since` is before `until`, both RFC 3339.
- Each line's `kind` is a known kind, and each line points at one target or none. A `ticket` or `decision` exists in the local copy of this space, given by key, handle or object id; a `pr` is an `https` URL; an `elsewhere` is `<space id>/<object id>`, or a key or handle `nxs` resolves in a space the member holds.
- No text holds a secret by the patterns under "Secrets and personal data" (`secret_in_note`).
- The member that writes holds the role `pm`, `owner` or `developer` in its `spaceMember` record (RFC 0001, "The role record"); a role session such as a coder is refused (`status_writer_not_allowed`). The owner chose these writers (NEX-68): pm writes the status (GENESIS.md, "Who it is for"), and a coder cannot write the repository's note by mistake. *Not taken:* any writer of the repository space, under which any session could add a note the owner then reads as the day's. A team that wants another role to write notes needs this rule changed.
- `corrects`, when given, names a note of this space with the same `date` (see "Correcting a note").
- `health` that differs from the health the rule computes at write time comes with a `health_why` of one line of at most 200 characters (`health_override_needs_reason`); `health` left `null` takes the computed one (see "Health").
- The space is a repository space; the storage crate refuses a note in a coordination space (`not_a_repository_space`).

`repository`, `author`, `health_computed` and `goals` are written by `nxs` and cannot be given: `nxs status write` computes the health again on its own copy, for the note's `since` and `until`, so what is stored is what the rule saw when the note was written, not what the draft saw earlier. `nxs status write` prints the note as `nxs status show` does.

Two notes for the same day can exist: NoCentralDB cannot keep a value unique across members that write offline, and object ids are drawn at random (NoCentralDB RFC 0004, "Two creates of one object id"). `nxs status write` warns when the local copy already holds a note of that day that no note corrects, and writes it all the same; readers show both (see "Reading notes").

### Health

Every note carries a health: `on_track`, `at_risk` or `off_track`. The owner asked that the way there be supported deterministically and oriented on the goals (NEX-66). So `nxs` computes a health by a fixed rule from the goals in focus and this repository's tickets that serve them, and the writer starts from it. The writer may override it, and then says why; the note keeps both.

**The inputs.** All are read locally, from the member's copies of the repository space and of the coordination space `nexus-flow.toml` names, at the note's `until`, and none is a threshold anyone sets:

- **The goals in focus, in their order:** RFC 0005, "The order of the goals in focus", through `goalsGoal.byStatus`.
- **A goal's tickets here:** the tickets of the repository space whose `focus` is `direct` or `inherited` and whose `focus_goal` is that goal (RFC 0005, "Ranking"), archived tickets left out. A ticket counts for one goal only, the one it ranks by.
- **For each of them,** from RFC 0002's derived state: `status`, `ready`, `blocked`, `done`, `flags`, `due`, `closedAt`; and whether it waits for the owner: it is in the owner's queue (RFC 0002, "Questions to the owner"), or one of its unresolved dependencies is.
- **The period:** the note's `since` and `until`.

**The counts per goal,** each over the goal's tickets here:

| Count | Tickets that |
| --- | --- |
| `open` | are `open` or `in_progress` |
| `in_progress` | are `in_progress` |
| `ready` | are ready (RFC 0002) |
| `blocked` | are blocked (RFC 0002) |
| `done_in_period` | were closed as `done` with `closedAt` from `since` up to `until` |
| `overdue` | are `open` or `in_progress` with a `due` before `until` |
| `flagged` | are `open` or `in_progress` and carry a flag (`cycle`, `holder_gone`, `pr_closed_unmerged`) |
| `waiting_for_owner` | are `open` or `in_progress` and wait for the owner, as above |

**The verdict per goal:**

1. **`no_work`** when `open` and `done_in_period` are both 0: the goal has no work in this repository, and it does not count.
2. **`off_track`** when `open` is above 0 and `in_progress`, `ready` and `done_in_period` are all 0: the goal has work here, and none of it moved in the period or can move now.
3. **`at_risk`** when `overdue`, `flagged` or `waiting_for_owner` is above 0, or when `ready` is above 0 while `in_progress` and `done_in_period` are both 0: something stands in the way, or ready work for a goal in focus is lying idle.
4. **`on_track`** otherwise.

**The note's computed health** is the worst verdict among the goals that count, `off_track` before `at_risk` before `on_track`. Every goal in focus weighs the same: the order of the goals says which work goes first (RFC 0005, "Ranking"), not which may stall unseen. The computed health is `null`, and the draft says why in `health_basis`, when the member holds no copy of the coordination space (`no_coordination_space`), when no goal is in focus (`no_goal_in_focus`), or when every goal in focus is `no_work` here (`no_work_for_goals`).

**An override.** The writer may set another `health`, or one where the rule gives none, with a `health_why` that says why in one line, such as "the release is blocked on the relay outage, which no ticket here shows". The note then shows both. A note whose writer gives no health takes the computed one, and a note with neither has none.

**What the rule saw is kept.** A ticket closed or reopened after the note changes the counts, so the counts are written with the note, as one `statusGoal` object per goal in focus, in the same change:

| Field | Type | Holds |
| --- | --- | --- |
| `note` | `nc.link("statusNote")` | The note |
| `goal` | `nc.ref(["goalsGoal"])` | The goal in the coordination space |
| `label` | `nc.string()` | The goal's key and title when the note was written, such as `cb.1 Win 20 paying customers for nexflow.it` |
| `position` | `nc.int()` | Its place in the order then, `1` for the first |
| `verdict` | `nc.oneOf(["onTrack", "atRisk", "offTrack", "noWork"])` | Its verdict |
| `open`, `inProgress`, `ready`, `blocked`, `doneInPeriod`, `overdue`, `flagged`, `waitingForOwner` | `nc.int()` | The counts |
| `date` | `nc.string()` | The note's date, copied |

The rule is `nxs`'s, the same for every repository, and a later version of nexus-flow may refine it; the counts and `healthComputed` written with each note say what the rule saw that day, so an old note stays explained by its own record. A team's own health rule is not designed now, as a team's own ticket fields are not (NEX-23).

*Not taken:* a health the writer sets with no rule beside it, as the Linear note has it today (it is the judgement the owner asked to have supported deterministically); a health computed and never overridden (the rule cannot see what no ticket records, such as an outage); thresholds in days, such as "blocked for more than three days" (a number someone must choose and keep, where the period between two notes already says how long nothing moved); and weighting goal 1 above the others (a goal in focus that stalls is worth the owner's attention whatever its place).

### Correcting a note

**No change rewrites or deletes a note through nexus-flow.** The module has no command and no `Engine` symbol that changes or deletes an object of its two collections, and the storage crate refuses any write to an existing object of them. A note is what the owner was told on that day; a correction keeps both.

A note is corrected by a new note for the same day that names it in `corrects`, written with `nxs status write` and a `corrects` field in its input. The new note is complete on its own: its lines are all the day's lines, not only the changed ones. Readers show the newest note of a day that no note corrects, and `nxs status show` lists the notes it corrects under `corrected`, so the earlier text stays readable. The owner chose this (NEX-67), for the reason a decision is never edited away (GENESIS.md, principle 4). *Not taken:* editing a note in place with `nxs status update`, which takes the text the owner read that morning out of the view, so a note is no longer a record of what was reported; and editing in place until the next note is written, a window for typos at the cost of a rule that depends on what another member has synced.

This holds for every writer that goes through nexus-flow, as RFC 0003's append-only guarantee does, and ends where it ends: a member that writes to the space with NoCentralDB's SDK directly can change or delete a note, because rights apply to a whole space (RFC 0001, "What it does not do"). Once RFC 0003's R12 lands, both status collections declare themselves write-once, and the guarantee holds against every writer. Until then, the storage crate shows what it can see, as RFC 0003 does ("Append-only"): a field of a note or a line with a conflict reads `"edited": true`, with the conflicting values under `"conflicts"` (NoCentralDB RFC 0009, "Conflicts, lost removals and link states"), and a note that a line or a correction leads to but that reads as deleted is listed with `"deleted": true`.

### Secrets and personal data

GENESIS.md forbids writing secrets, tokens or personal data into the status note. Naming members and the owner by their identity is not personal data in that sense (GENESIS.md, "What it must never do"), so a line may name who decided or who holds a ticket.

**Secrets.** `nxs status write` refuses a note whose `headline`, `body`, any line's `text` or `label` matches a known secret pattern, and names the field and the pattern, never the match: a private key block (`-----BEGIN … PRIVATE KEY-----`), a GitHub token (`ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`, `github_pat_`), an AWS access key id (`AKIA` or `ASIA` and 16 upper-case letters or digits), a Slack token (`xox[abprs]-`), a JSON web token (three base64url parts, the first starting `eyJ`), and a Linear API key (`lin_api_`). The list is nexus-flow's, ships with `nxs`, and grows with releases. A pattern cannot find every secret; it finds the ones that leak most often, before they are copied to every member.

**Personal data.** No field of a note is marked personal (NoCentralDB RFC 0005, "Personal fields"), and nexus-flow does not scan text for personal data: it cannot tell a customer's name from a project's. Personal data written into a note by mistake cannot be erased, for the reasons RFC 0003 gives for a decision ("Personal data and erasure"): only a field the schema marks personal is erasable, and an overwrite hides a value without removing it from the log. RFC 0003's R13 would let it be erased. The default template asks the writer not to name customers or third parties, and to point at a ticket instead.

### Derived state

Nothing below is stored. The storage crate computes it on read, the same way for `nxs` and for manufakt.io, as RFC 0001's derived state is ("Derived state").

- **The note of a day** is the newest note of that date, by `until`, that no note corrects. A note is `corrected` when a note's `corrects` names it.
- **A line's target now.** Each line is read with the current state of its target, from the reader's own copy: for a ticket, its key, status, `done` and `on_main` (RFC 0002, "Derived state"); for a decision, its handle and `in_force` (RFC 0003, "Derived state"); for a pull request, the `state` and `on_main` of the work link with that URL, or `null` when no ticket records it; for a request, its status (RFC 0009); for any reference, its resolution state.
- **`open`, for a `waits` line:** whether it still waits, now. A ticket waits while it is not closed; a decision does not wait once it is recorded; a pull request waits until its work link is recorded `merged` or `closed`; a request waits while it is `open`. It is `null` when the reader cannot tell: a pull request no ticket records, a target the reader cannot resolve, or a line that points at nothing.

The line keeps what was reported, and `open` says what holds now, so the owner sees on an old note which of its questions are answered.

### Indexes

Every query runs through a declared index (NoCentralDB RFC 0006, "Declared indexes"). A query skips deleted objects.

| Collection | Index | Fields (equality prefix in bold) | Scope | Read by |
| --- | --- | --- | --- | --- |
| `statusNote` | `byDate` | `date` desc, `until` desc | space | `list`, `show <date>`, the newest note for a draft's `since` |
| | `byDateAcrossSpaces` | `date` desc, `until` desc | every space of the member | `list --all-spaces`, `waiting --all-spaces`, manufakt.io's table |
| | `byCorrects` | **`corrects`** | space | whether a note is corrected, and by which |
| `statusItem` | `byNote` | **`note`**, `kind`, `position` | space | a note's lines, by kind and position |
| | `byTicket` | **`ticket`**, `date` desc | space | the notes that reported on a ticket, newest first, for a ticket's view |
| | `byDecision` | **`decision`**, `date` desc | space | the notes that reported on a decision |
| `statusGoal` | `byNote` | **`note`**, `position` | space | what a note's health was computed from |
| | `byGoal` | **`goal`**, `date` desc | space | a goal's verdicts in this repository, day by day |

`byDateAcrossSpaces` is the one index without `scope: "space"`. A member that holds several repository spaces, such as the owner's device or manufakt.io, reads every repository's notes of a day in one ordered page through it; each item carries its space (NoCentralDB RFC 0009, "Queries and indexes").

### Reading notes

`nxs status list [--since <YYYY-MM-DD>] [--until <YYYY-MM-DD>] [--limit <n>] [--all] [--all-spaces] [--index]` lists the notes, newest day first, through `byDate`, or `byDateAcrossSpaces` with `--all-spaces`. It shows the note of each day, and with `--all` corrected notes too. `--limit` defaults to 30. `--index` prints one row per note instead of the whole note.

`nxs status show [<date | id>]` prints one note in full, by default the newest. A date selects the note of that day; when two notes of one day are both uncorrected, it fails with `ambiguous_note` and lists their ids and headlines, and every command takes a note's object id as well.

`nxs status waiting [--all-spaces]` prints what waits for the owner now: every `waits` line of the newest note of each space that is still `open`, and every ticket of the owner's queue (RFC 0002, "Questions to the owner") that no such line names, each with its space. It is what pm reads when the owner asks what waits for them, and what manufakt.io shows beside the notes.

Output is deterministic, JSON by default and Markdown with `--md` (RFC 0001, "Reading and writing"). One note in JSON:

```json
{
  "id": "4c0e…",
  "space": "b81f…",
  "repository": "nexus-flow",
  "date": "2026-11-17",
  "time_zone": "Europe/Berlin",
  "since": "2026-11-16T05:00:00Z",
  "until": "2026-11-17T05:00:00Z",
  "health": "at_risk",
  "health_computed": "on_track",
  "health_why": "The relay was down for six hours; no ticket shows it yet",
  "health_basis": null,
  "goals": [
    {
      "goal": { "id": "e19a…", "space": "7c2d…", "label": "cb.1 Win 20 paying customers for nexflow.it", "state": "ok" },
      "position": 1, "verdict": "on_track",
      "counts": { "open": 4, "in_progress": 1, "ready": 1, "blocked": 2, "done_in_period": 1, "overdue": 0, "flagged": 0, "waiting_for_owner": 0 }
    }
  ],
  "headline": "RFC 0001 is decided; four module RFCs are in draft",
  "body": "…",
  "items": [
    {
      "kind": "changed", "position": 1, "text": "RFC 0001 decided and merged",
      "target": { "kind": "ticket", "id": "a03e…", "label": "ka.5", "space": null, "status": "closed", "done": true, "on_main": true, "state": "ok" },
      "open": null
    },
    {
      "kind": "waits", "position": 1, "text": "RFC 0003, question 1: the handle",
      "target": { "kind": "ticket", "id": "77d1…", "label": "ka.51", "space": null, "status": "open", "done": false, "on_main": false, "state": "ok" },
      "open": true
    },
    {
      "kind": "waits", "position": 2, "text": "Review of RFC 0002",
      "target": { "kind": "pr", "id": null, "label": "#160", "url": "https://github.com/…/pull/160", "space": null, "status": "open", "done": null, "on_main": false, "state": null },
      "open": true
    }
  ],
  "corrects": null,
  "corrected_by": null,
  "corrected": [],
  "author": "9b2d…",
  "edited": false,
  "deleted": false
}
```

- `health` and `health_computed` are `on_track`, `at_risk`, `off_track` or `null`, snake_case as every JSON value of v1. `health_why` is the writer's reason when the two differ, else `null`. `health_basis` is `null`, or why the rule computed no health: `no_coordination_space`, `no_goal_in_focus` or `no_work_for_goals` (see "Health").
- `goals` lists what the rule saw for each goal in focus, in the goals' order: `goal` (its `id`, `space`, `label` and the reader's `state`), `position`, `verdict` (`on_track`, `at_risk`, `off_track` or `no_work`) and `counts`.
- `items` are ordered by kind, in the order `changed`, `works`, `waits`, `decided`, `blocked`, then by `position`.
- `target` is `null` for a line that points at nothing. Its `kind` is `ticket`, `decision`, `pr`, `request`, `goal` or `remote_ticket`; `space` is `null` in the note's own space and the other space's id otherwise; `state` is `ok`, `not_accessible` or `missing`, as the reader resolves it (`null` for a pull request). `status`, `done` and `on_main` are the target's current state where its kind has one, else `null`; a decision carries `handle` and `in_force` instead.
- `corrects` and `corrected_by` are `{ "id", "headline" }` or `null`; `corrected` lists the notes this one corrects, in `show` only.
- `conflicts` appears only on an edited note, as in RFC 0003.
- `nxs status list` prints `{ "notes": [ … ] }`; with `--index`, each note carries only `id`, `space`, `repository`, `date`, `health`, `health_computed`, `headline`, the number of `waits` lines still `open` as `waiting`, and `corrected_by`. `nxs status waiting` prints `{ "waiting": [ … ] }`, each entry an item as above with its note's `id`, `date` and `space`, or, for a ticket of the owner's queue that no note names, `note: null`. `nxs status draft` prints the note without `id`, `author`, `corrected_by`, `corrected`, `edited` and `deleted`, with `space` and `repository` filled, and with `template` (`{ "path", "exists" }`).
- Keys are snake_case, as v1's; empty arrays and unset values are written as `[]` and `null`.

In Markdown, `nxs status show` prints `# <repository> · <date>`, the health, followed by `(computed: <health>; <why>)` when the writer overrode it, the headline, one line per goal in focus with its position, label and verdict, the body as written, then one heading per kind that has lines ("What changed", "What works now", "What waits for the owner", "What was decided", "What broke or is blocked") with one bullet per line: its text, its label, and `(answered)` on a `waits` line that is no longer `open`. `--index --md` prints one line per note: `date · repository · health · headline · <n> waiting`.

### What manufakt.io reads

manufakt.io is a writer of the person's repository spaces and of their coordination space, as a device of the person's account (RFC 0001, "Members", "Roles in a space"). It reads its own member database through the seam, as any member does, with no read on a server (RFC 0001, "The library seam"):

- **The table of notes:** `Engine::status_notes` with `all_spaces`, through `byDateAcrossSpaces`, one row per repository and day, with its `--index` fields, so the table can show the health and whether the writer overrode it.
- **A note:** `Engine::status_note`, with its lines and their targets resolved from manufakt.io's own copy. manufakt.io is a member of the coordination space, so requests and goals resolve for it.
- **What waits for the owner:** `Engine::status_waiting` with `all_spaces`.
- **Live updates:** `Engine::subscribe` (RFC 0001), which re-runs a query when a note arrives.

manufakt.io opens spaces without a checkout (`Engine::open_space`, RFC 0001), so every `on_main` it reads is `null` (RFC 0001, "Derived state"). It needs nothing else from nexus-flow, and nothing from a server.

### The library seam

`Engine` gains, as RFC 0001 says each module RFC adds its reads and writes to it ("The library seam"):

| Symbol | What it does |
| --- | --- |
| `Engine::status_draft(&DraftOptions) -> StatusDraft` | Gathers the draft of "Writing a note" |
| `Engine::status_write(NewStatusNote) -> StatusNote` | Writes a note and its lines, after the checks of "Writing a note" |
| `Engine::status_notes(&StatusFilter) -> Vec<StatusNote>` | The notes the filter selects, newest day first, with their derived state |
| `Engine::status_note(&NoteKey) -> StatusNote` | One note by date or object id, or the newest; an ambiguous date is an error that lists the matches |
| `Engine::status_waiting(all_spaces: bool) -> Vec<WaitingItem>` | What `nxs status waiting` prints |

`DraftOptions` (`since`, `date`, `time_zone`), `StatusDraft`, `NewStatusNote`, `StatusNote`, `StatusItem`, `ItemTarget`, `Health`, `GoalHealth`, `StatusFilter` (`since`, `until`, `limit`, `all`, `all_spaces`, `index`), `NoteKey` (a date, an `ObjectId`, or the newest) and `WaitingItem` are the types they take and return; their fields are the JSON fields above. Every `nxs status` verb has its counterpart here (the decision on CLI and library parity stays in force). There is deliberately no symbol that changes or deletes a note.

### Left to priority 2

Named here, designed later by rewriting this RFC:

- **The schedule.** Writing the note every day, by default at 06:00 in the coordination space's time zone (GENESIS.md). Until then pm writes it when its own scheduled task wakes it, as this factory does today (`factory/CONFIG.md`, "Daily note").
- **The stored default time zone.** GENESIS.md counts the day in "the time zone the coordination space was set up in", so that every repository of a group dates its notes alike. Recording that zone belongs to creating the coordination space (RFC 0009), and a repository without one needs a zone of its own; until then the writer's machine or `--time-zone` gives it, and the note records the zone it used.
- **A wake-up for the owner's side.** A subscription to new notes for an adapter or for manufakt.io's notifications, on `Engine::subscribe` (R3, met).

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Commands:** `nxs status draft [--since <time>] [--date <YYYY-MM-DD>] [--time-zone <zone>]`, `nxs status write <file | ->`, `nxs status list [--since <YYYY-MM-DD>] [--until <YYYY-MM-DD>] [--limit <n>] [--all] [--all-spaces] [--index]`, `nxs status show [<date | id>]`, `nxs status waiting [--all-spaces]`. One noun and a verb, as RFC 0001's and RFC 0002's commands are (decided by the owner, NEX-69). *Not taken:* `nxs note <verb>`, since "note" is also a ticket's comment (RFC 0002, "Notes"); and a bare `nxs status` that prints the newest note beside its verbs, a noun that is both a read and a group of verbs.
- **JSON output:** the fields under "Reading notes", the draft's `template`, and the input `nxs status write` reads, which is the draft's shape.
- **Errors:** `secret_in_note` (with `field` and `pattern`), `status_writer_not_allowed`, `health_override_needs_reason`, `ambiguous_note` (with the matching ids and headlines), `not_a_repository_space`.
- **Markdown output:** the note as described under "Reading notes", and the index line.
- **Library seam:** the `Engine` symbols and types above.
- **Stored formats:** the collections `statusNote`, `statusItem` and `statusGoal` with their fields and indexes, the five kinds, the health rule's inputs, counts and verdicts, the 200-character headline and line, the secret patterns as a rule of `nxs status write`, and the template's path `factory/status-note.md`.

### What nexus-flow needs from NoCentralDB

Nothing new. The design rests on RFC 0001's R6 (who signed a change, for `author`) and, for derived state it reads from other modules, R7; on RFC 0003's R12 (write-once collections, for "Correcting a note" to hold against every writer) and R13 (erasing an unmarked value, for personal data written by mistake); and on what NoCentralDB already offers: indexes across every space of a member (NoCentralDB RFC 0006, "Declared indexes"), one write as one change (NoCentralDB RFC 0009), and subscriptions (R3).

## What it does not do

- **No note is rewritten through nexus-flow, but until R12 only through nexus-flow.** A writer that uses NoCentralDB's SDK directly can change or delete a note; a concurrent change shows as a conflict, a deletion of a note that a line or a correction leads to shows as `deleted`, and a sequential edit is unflagged until R6.
- **Two notes for one day can exist.** Members that write offline cannot be stopped from both writing one; readers show both and `show <date>` refuses to choose.
- **The secret patterns find known shapes only.** A password, a key without a known prefix, or a secret split across lines passes. Personal data is not detected at all, and once written it cannot be erased until R13.
- **The draft does not know what waits for the owner's review on the forge.** `nxs` reads no network, so pm adds the pull requests that wait; a pull request no ticket records reads `open: null`.
- **A note covers what its writer's copy held.** It is as fresh as the writer's last sync (GENESIS.md, "Limits"): in island mode, or with a relay that withholds changes, a note leaves out what has not arrived, and says nothing of it.
- **"What works now" is a judgement.** Nothing checks it against the code.
- **The computed health sees only tickets.** It reads the goals in focus and this repository's tickets that serve them; an outage, a slow review or work no ticket records is invisible to it, which is why the writer may override it. It sees this repository only: a goal that stalls in another repository of the group shows in that repository's note. A member without a copy of the coordination space gets no computed health, and a ticket that serves no goal in focus never moves it.
- **A line's state is the reader's.** A member without the coordination space reads a request or a goal as `not_accessible`, with its label only; manufakt.io reads every `on_main` as `null`.
- **There is no note for a group of repositories.** The owner's view of a group is a read across the repository spaces they are a member of; a member who holds only some of them sees only those.
- **The day is counted in the zone the writer gives,** until priority 2 stores a default: two repositories of one group can date the same morning differently if their writers' machines differ.
- **The `author` field is written by the writer and not checked** against the change's signature, until R6 (RFC 0001, "Who decided").

## Migration

Nothing to migrate. v1 has no daily note: `nxs recap` lists recently closed tickets from the board, and stores nothing. This repository's notes so far are status updates on a Linear initiative (`factory/CONFIG.md`), not v1 data; they stay in Linear, and RFC 0008 does not move them unless the owner asks for it.

## Open questions

None. The owner answered them on 2026-11-20 (NEX-65 to NEX-69), and the answers are written into the text above.

## History

- 2026-11-17: published as a draft in #162.
- 2026-11-20: decided by the owner (NEX-65 to NEX-69), in #162.
