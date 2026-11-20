# RFC 0006: Factory storage

Status: Decided (2026-11-20)

This RFC designs where the `factory` module keeps a team's way of working, and what becomes of the storage of v1's `chat`, which `factory` replaces. It follows `factory/GENESIS.md`, "What we build" (the `factory` row: roles, lanes, the merge gate and the gate command, stored in git) and "How work moves". It says where the role files, FACTORY.md, CONFIG.md and the merge rules live and which copy of them is the truth, which of the factory's running state needs storage and where that storage is, how the defaults nexus-flow ships reach a repository without overwriting a team's own, and that v1's chat storage ends, table by table. Once it is decided, a coding ticket can build `nxs factory install`, the check that ties a session's role to a role file, and the removal of v1's chat crates from `v2`.

## What it solves

A team that lets agents work for days needs its way of working written down where the owner reviews it and every agent reads it (GENESIS.md, "What we build"). v1 kept that way of working inside the tool: personas, channels and step chains in `.nxs-personas/`, a message log synced through v1's own op log, and a run engine with leases, queues and liveness records in `.nxs/`. v2 drops nexus-flow's own orchestration, keeps only roles and hand-offs, and leaves sub-agents to the agent frameworks (GENESIS.md, "Roles and lanes").

The owner and the agents need from the factory's storage:

- **Files the owner reviews in a pull request.** The roles, FACTORY, CONFIG and the merge rules decide what agents do and when the owner is asked, so a change to them goes through the owner's review (GENESIS.md, "The merge gate"; principle 1).
- **One rule for which copy counts**, so that a branch cannot loosen the rules that judge it.
- **A record of who holds which lane on which ticket**, so pm knows what runs and a new pm can take over (FACTORY.md, "Running it").
- **Defaults that run on day one and never overwrite a team's own** (GENESIS.md, "Every default belongs to the team", and the no-go "Never overwrite a team's own GENESIS, FACTORY, CONFIG, merge rules …").
- **A clean end for v1's chat data**, with nothing lost silently (principle 4).

The owner decided the two main choices before this RFC was written, and the text below carries them: the roles, FACTORY, CONFIG and the merge rules stay files in git, and the files are the truth (NEX-14); v1's chat data and personas lapse, neither moved nor exported, untouched on disk, and v2 reads none of it, except that the legal-text team is carried over as roles (NEX-26) and a marketing team is built new (NEX-27), after one review of the data for lessons (NEX-25; decided in NEX-20).

Out of scope, each elsewhere: the merge gate's rules, their JSON schema and the script that decides (the merge gate's own RFC; this RFC only says where they are kept); GENESIS.md, its template and `nxs genesis create` (RFC 0004); the decision log (RFC 0003); the ticket states and the field that links a ticket to its pull request, branch or artifact (RFC 0002); the move of a v1 workspace's tickets and memories (RFC 0008); the text of the shipped defaults, which is a default template the owner decides in its own ticket (GENESIS.md, "What has to come back through me"); and the factory's behaviour beyond storage, which a later revision of this RFC adds (decision of 2026-11-15, NEX-6).

This RFC builds on RFC 0001 (spaces, members and the `spaceMember` record, `nexus-flow.toml`, derived state) and cites NoCentralDB's RFCs as "NoCentralDB RFC", by number and section.

## The design

### The factory's files

A repository's way of working lives in one folder at the repository's root, the folder that holds `nexus-flow.toml` (RFC 0001, "Creating and finding a space"):

```
factory/
  FACTORY.md        # how work moves: the life of a ticket, what pm does, the rules for every role
  CONFIG.md         # the values a team fills in: owner, tracker, lanes, base branch, gate command, merge rule
  roles/
    pm.md           # shipped
    coder.md        # shipped
    <name>.md       # a team's own roles, such as legal or marketing
  merge-gate/       # the merge rules, their JSON schema and script; the merge gate's RFC designs what is inside
  status-note.md    # the daily note's template; RFC 0007 designs its content
```

**The path is fixed.** Every repository keeps the factory in `factory/`, so every agent's instructions can point at the same place, and the factories nexus-flow follows already do (nocentraldb's and this repository's). *Not taken:* a setting in `nexus-flow.toml` that names another folder (a setting earns its place only when a real team needs it, GENESIS.md, "Every default belongs to the team", and one can be added later without breaking anyone); and a hidden folder or `.nxs/` (the owner reviews these files in pull requests, so they should be as visible as the code; `.nxs/` is v1's workspace and ignored in git).

**Two neighbours are not part of the factory module.** A repository may keep its GENESIS.md in `factory/`, as this one does; where `nxs genesis create` writes it is RFC 0004's. And `factory/decisions/` holds this repository's decision log only until decisions move into NoCentralDB (RFC 0003, RFC 0008); the factory module neither installs nor reads it.

**nexus-flow reads nothing inside the files.** They are prompts for agents and text for people, and their shape is the team's (GENESIS.md, "Strong opinions on what, open on how"). nexus-flow reads only which files exist:

- **A role exists when its file exists.** The role's name is the file's name without `.md`, and it matches `^[a-z][a-z0-9-]{0,62}$`. The names of the frame's own roles in RFC 0001, "The role record" (`owner`, `developer`, `app`, `migration`) are not role names, so a role file cannot be named after them; `pm` is a role and has its file.
- **A session's role names a role file.** The role a `spaceMember` record gives a session (RFC 0001, "The role record") is the name of a file in `factory/roles/`. `nxs member add <invitation> --role <role>` for a session refuses a role that has no file in the working tree, and says which file is missing, so no session holds a role nobody wrote down.
- **CONFIG.md holds no value a script reads.** It is Markdown for agents and people. A value that a command must read the same way every time goes into `nexus-flow.toml`, as the base branch already does (`[git] base`, RFC 0001); this RFC adds no key there. The merge gate's RFC may add files under `factory/merge-gate/` that its script reads, in a format it designs.

### Which copy is the truth

The files in git are the truth (NEX-14). nexus-flow keeps no copy of them in NoCentralDB and no cache of them anywhere. Two kinds of reader use two copies, and each copy is fixed:

- **A session reads its working tree.** `nxs` and every agent read the factory's files as they are checked out where the session runs, local changes included (NEX-14). A branch can therefore try a changed role before it merges, and a session on an old branch works under the rules of that branch.
- **What judges a pull request reads the base branch.** Whatever decides about a pull request reads the factory's files from the commit the base branch points at (`[git] base` in `nexus-flow.toml`), never from the pull request's head. A pull request that loosens a merge rule or a role's exception is judged by the rules it would replace, and it merges only with the owner's review, because these paths are under CODEOWNERS (decisions of 2026-11-15, NEX-32 and NEX-33). How the merge gate reads them is its RFC's; this RFC fixes only which copy it reads.

*Not taken:* records in NoCentralDB as the truth (decided against in NEX-14: the owner's approval would have to be rebuilt inside NoCentralDB). A read-only mirror of the files into the repository space, which NEX-14 left open for when manufakt.io or a coordination space needs to read them, is not part of v2: no v2 reader needs it yet, and a later RFC can add it without changing which copy is the truth.

### The running state

While work runs, pm and the owner need to know which role holds which lane on which ticket, and how far each ticket is on its way through ticket, pull request, review and merge (GENESIS.md, "Roles and lanes"). Each of these already has a home with one writer, so the `factory` module declares no NoCentralDB collection of its own in v2:

| What must be known | Where it lives | Designed in |
| --- | --- | --- |
| Which session holds which role on which ticket | The session's `spaceMember` record: `member`, `role`, `ticket` | RFC 0001 |
| Which lanes are held | Derived from the `spaceMember` records and the tickets' states, never stored (below) | This RFC |
| A ticket's pull request, branch or artifact | A field of the ticket | RFC 0002 |
| pm's review and the owner's approval | The pull request's reviews on the forge, and the merge gate's status check | The merge gate's RFC |
| Merged | "On main", derived from git | RFC 0001, RFC 0002 |
| The agent framework's handle on a running session, such as a Claude Code session id, which pm needs to message, wake or archive it | A comment pm writes on the session's ticket when it starts the session (below) | This RFC |

**A lane is derived.** A lane is held by a current member whose `spaceMember` record has a session role and names a ticket, while that ticket is in a state that holds a lane. Which states hold a lane is the team's rule, in CONFIG.md ("A lane is free again"); the states themselves are RFC 0002's. In v2, pm applies the rule and CONFIG.md's limit when it reads the members (`nxs member list`) and the tickets; nexus-flow does not count lanes itself. A command that counts them is factory behaviour, for a later revision of this RFC, and the limit would then move into `nexus-flow.toml` so that a command can read it.

**pm keeps the framework's handle in a ticket comment.** When pm starts a session, it writes the agent framework's handle on that session, such as a Claude Code session id, as a comment on the session's ticket. pm reads it from there to message, wake or archive the session, and a new pm reads it there to take over the running sessions (decided by the owner, NEX-44). It needs no new field and outlives the session, and it is how this repository's factory works today. Its shape is the framework's, and nexus-flow neither reads nor checks it. *Not taken:* a `framework` field on the `spaceMember` record (an additive change to RFC 0001's collection, and nexus-flow would store a framework detail it otherwise leaves to the framework, principle 6); and storing no handle at all (a new pm could not take over the running sessions unless the framework hands them over).

**A session that waits for the owner holds no lane but stays a member.** Its member and its `spaceMember` record stay until the session ends (RFC 0001, "Ending a session's member"), so it can still answer review; its lane is free because its ticket's state no longer holds one.

*Not taken:* a lane collection of the factory's own, with the role, the ticket and the session. It would restate what the member record and the ticket's state already say, drift from them when a session dies without ending, and need a writer that keeps it current; derived state is never stored for that reason (GENESIS.md, "Done is not yet on main"). And v1's run state (runs, the working-tree lease and queue, liveness records, held wake-ups): how a session spawns, waits for and steers sub-agents is the framework's job in v2 (GENESIS.md, "Sub-agents are the framework's job"), and each session works in its own checkout, so there is no shared working copy to lease.

### Installing the shipped defaults

nexus-flow ships the least a factory needs to run (GENESIS.md, the `factory` row): `FACTORY.md`, a `CONFIG.md` to fill in, `roles/pm.md`, `roles/coder.md`, the daily note's template `status-note.md`, whose content RFC 0007 designs, and, once the merge gate's RFC is decided, its minimal set under `merge-gate/`. Each one's text is a default template, which the owner decides in its own ticket; this RFC decides how they are kept and installed.

**Where nexus-flow keeps them.** In nexus-flow's source tree under `templates/factory/`, in the layout they are installed in, embedded in `nxs` at build time, so installing needs no network. Beside them, `templates/factory/shipped.toml` lists, for each path, the SHA-256 of every version nexus-flow has ever shipped, oldest first. A release adds to the list and never removes from it, so `nxs` recognises an old default however many releases ago it was installed. A path nexus-flow stops shipping stays in the list, marked `retired = true`. This file is part of nexus-flow's source, not of a team's repository.

**`nxs factory install`** compares each shipped file with the file at the same path in the repository's `factory/`, after turning CRLF line endings into LF on both sides, so that an unchanged default checked out on Windows reads as unchanged:

| The repository's file | State | What `install` does |
| --- | --- | --- |
| Missing | `written` | Writes the current default |
| Identical to the current default | `current` | Nothing |
| Identical to an earlier default | `older_default` | Nothing; with `--update`, writes the current default and reports `updated` |
| Anything else | `team` | Nothing, with or without `--update` |
| Present, and its path is retired | `retired` | Nothing |

It never deletes a file, never writes a path the shipped list does not name, and never touches a role the team added. It never commits: what it writes lands in the working tree and reaches the base branch through a pull request, which CODEOWNERS sends to the owner. Even `--update` therefore changes nothing in force without the owner's review.

`nxs init` (RFC 0001) runs the same install as its last step, without `--update`. On an upgrade of nexus-flow, nothing is installed until someone runs `nxs factory install`.

**CODEOWNERS is reported, never written.** The roles and the merge rules must be under CODEOWNERS for the merge gate to hold (GENESIS.md, "The merge gate"). `nxs factory install` reads the repository's CODEOWNERS file, in the places and order GitHub looks for it (`.github/CODEOWNERS`, `CODEOWNERS`, `docs/CODEOWNERS`), and reports which of the factory's paths no rule in it covers. It never writes CODEOWNERS: that file is the team's own, and a change to it comes back to the owner (GENESIS.md, "What has to come back through me").

**Output.** JSON by default, Markdown with `--md`, files ordered by path, bytewise:

```json
{
  "files": [
    { "path": "factory/CONFIG.md", "state": "team" },
    { "path": "factory/FACTORY.md", "state": "current" },
    { "path": "factory/roles/coder.md", "state": "older_default" },
    { "path": "factory/roles/pm.md", "state": "written" }
  ],
  "codeowners": {
    "file": ".github/CODEOWNERS",
    "uncovered": ["factory/roles/pm.md"]
  }
}
```

`codeowners` is `null` when the repository has no CODEOWNERS file; `uncovered` lists paths in the same order as `files`.

*Not taken:* replacing an unchanged default on upgrade without being asked (a team's way of working would change because someone updated a binary, which no one chose); merging a team's changes with a new default (the result is a prompt nobody wrote, and a merge conflict in a role file is one more thing for the owner to review); and a manifest committed in the team's repository that records which version was installed (one more file to keep in step and to review, where the hashes in `nxs` answer the same question).

### v1's chat storage ends

v1's chat kept its data in four places: chat operations in the workspace's synced op log and the views folded from them (`.nxs/db.sqlite`); device-local tables in the same file; the run engine's own file (`.nxs/run.sqlite`); and files beside the workspace. None of it is moved or exported, all of it stays untouched on disk with the rest of the v1 data, and v2 reads none of it (NEX-20). Nothing is kept. The legal-text team's personas become role files by hand in NEX-26, not by a migration.

**Synced, folded from the chat operations in `.nxs/db.sqlite`:**

| Table | Held | In v2 |
| --- | --- | --- |
| `ops`, the operations of kinds `message`, `channel`, `profile`, `membership`, `thread` | The chat's share of the workspace's synced op log | Ended. The `ops` table itself is the core's; RFC 0008's migration reads it and skips every operation of these kinds. |
| `messages` | Every message posted in a channel or thread | Ended |
| `channels` | Channels with their name and kind | Ended |
| `membership_adds`, `membership_removes` | Who was a member of which channel | Ended |
| `profiles` | A persona's job title, description, capability tags, runtime binding and whom it reports to | Ended. The legal-text team is rewritten as role files (NEX-26). |
| `threads` | A conversation's root, parent, expected reply, deadline, name and machine | Ended |

**Device-local, in `.nxs/db.sqlite`:**

| Table | Held | In v2 |
| --- | --- | --- |
| `session_map`, `session_threads` | A role session's internal id, role, agent SDK session id, spawn depth, and the threads it was put in motion on | Ended. A session is a member with a `spaceMember` record (RFC 0001); the framework keeps its own session ids. |
| `pending_wakes` | Answers held until a session settled, then delivered at once | Ended. Wake-ups are the framework's, behind an adapter (GENESIS.md, "Limits"). |
| `agent_transcript` | The agent SDK's stream of each role session | Ended. Watching a session is the framework's job (GENESIS.md, "Decide first, then build"). |
| `channel_member_deadline` | The reply window of a member thread in a declared channel | Ended |
| `synthesis_claims` | Who started the synthesizer for a completed channel round | Ended |
| `working_tree_lease`, `working_tree_queue`, `failed_starts` | Which chain of sessions held the working copy, the triggers waiting for it, and those that could not start | Ended |
| `declaration_snapshot`, `declaration_freeze`, `role_declaration_version` | Copies of the `.nxs-personas/` catalogue, which copy an operation ran under, and which version a role last started under | Ended |
| `operation_base`, `parked_work`, `park_refusals`, `withdrawn_holders`, `withdrawn_sessions` | Where an operation started, the branches its work was parked on, failed parks, and withdrawn operations and their sessions | Ended. The parked branches are git branches and stay in git; see "Migration". |
| `session_interruption` | Sessions stopped at a quota or availability boundary, and their resumption | Ended |
| `stopped_advances` | Step-chain advances the supervisor stopped for a person | Ended |
| `border_threads`, `border_served` | Consultations with a persona in another workspace on the same machine | Ended. Work across repositories goes through requests in a coordination space (GENESIS.md, "Work across repositories"). |
| `owed_starts`, `owed_backfill_done`, `stranded_suspects`, `stranded_sessions`, `recovery_log`, `queued_obligations` | The liveness ladder: which session owed which answer, sessions that would not answer, and each recovery step | Ended |

v1 already removed `read_cursors` and the six `workflow_*` tables; a v1 workspace sheds them on open, so there is nothing of them left to end.

**`.nxs/run.sqlite`**, the second run engine (`crates/run-*`): `events`, `inbox`, `timers`, `kv`, `runs` and `schema`, the runs of step chains and their timers. Ended.

**Files:**

| Path | Held | In v2 |
| --- | --- | --- |
| `.nxs-personas/<handle>.yaml`, `.nxs-personas/channels.yaml` | Persona and channel declarations | Ended. The files stay where they are; NEX-26 rewrites the legal-text team as `factory/roles/` files. |
| The `[chat.liveness]` section of `.nxs/config.toml` | The liveness ladder's policy | Ended, with the rest of `.nxs/`, which v2 does not read (RFC 0001) |
| `.nxs/agent-logs/` | The run engine's agent logs | Ended |
| `<repository root>-v2/` | The second working copy the run engine worked in | Ended. It is a checkout, not storage, and may hold uncommitted work; see "Migration". |

**The code goes too.** On `v2`, the crates `chat`, `run-contracts`, `run-engine`, `run-host`, `run-store-sqlite` and `run-session-claude` are not part of v2. They keep shipping from `main` as long as v1 does; their removal from `v2` is its own ticket, as RFC 0001 does for v1's sync. The Node sidecar's future is the wake-up adapters', not this RFC's.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Command:** `nxs factory install [--update]`, which `nxs init` also runs, without `--update`. And a refusal added to `nxs member add` (RFC 0001): a session's role must name a file in `factory/roles/`.
- **JSON output** of `nxs factory install`: `files` (each with `path` and `state`: `written`, `current`, `older_default`, `updated`, `team` or `retired`), `codeowners` (`file`, `uncovered`, or `null`).
- **Stored formats:** the `factory/` folder at the repository's root and its layout; a role's name as its file's name, and the pattern it matches; that `nexus-flow.toml` holds every value a command reads and CONFIG.md none.
- **Library seam:** `Engine::factory_install(update: bool)`, the counterpart of the command, returning the same report (the decision on CLI and library parity stays in force).

### What nexus-flow needs from NoCentralDB

Nothing. The factory's files stay in git, and its running state lives in RFC 0001's `spaceMember` record and RFC 0002's tickets, so this RFC adds no collection to the model and no requirement on NoCentralDB.

## What it does not do

- **It does not stop a session from editing its own rules.** A session reads its working tree, so it can change a role file and act on the change at once. The guard is at the merge: what judges a pull request reads the base branch, and the paths are under CODEOWNERS. A repository without CODEOWNERS, or on a forge without it, has no such guard; `nxs factory install` reports it but cannot make one.
- **It does not read a role's content.** nexus-flow checks that a role's file exists, not what it says; a role file that is empty or wrong is the team's to find.
- **It does not count lanes.** pm applies CONFIG.md's limit itself in v2, so a pm that miscounts can open one lane too many; nothing in nexus-flow refuses it yet.
- **It does not read a session's framework handle.** The handle is a comment pm writes on the ticket, in the framework's shape. A reader has to find it among the comments, and a session started by hand, outside pm, has no handle anywhere unless someone writes it there.
- **It does not bring a new default to a team that changed the old one.** A `team` file never gets a newer default; the team reads the release notes and changes its file itself.
- **It recognises only defaults nexus-flow shipped.** A file that differs from every shipped version by one character is the team's, and `--update` leaves it.
- **It does not give manufakt.io the factory's files.** A caller with no checkout cannot read the roles or CONFIG until a later RFC adds a mirror.
- **A team that changes its base branch changes what judges a pull request.** The rules are read from whatever `[git] base` names.

## Migration

Nothing of the factory's own needs migrating: a repository that has no `factory/` gets the defaults from `nxs factory install`, and one that already has its own files keeps them, as `team`.

v1's chat data is not migrated (NEX-20). It stays in `.nxs/` and `.nxs-personas/`, untouched, and v1 keeps reading it from `main`. Two things in it can hold work that is not stored anywhere else, and RFC 0008 must make sure neither is lost silently when a team runs `nxs migrate` (principle 4):

- **Parked branches.** `parked_work` names the git branches v1 parked an operation's work on. The branches stay in git, but a park that was never resumed is work nobody may remember. `nxs migrate` lists every park with no `resumed_at`, with its branch and commit.
- **The second working copy.** `<repository root>-v2/` may hold uncommitted changes. `nxs migrate` reports it when it exists and is not clean.

Both are reports, not moves: the team decides what to do with the work. Before that, NEX-25 reviews the chat data once for lessons for the new factory.

## Open questions

None. The owner answered the one question this draft left open on 2026-11-20 (NEX-44), and the answer is written into the text above.

## Decisions

- **`migration` joins the frame's own roles** ("The factory's files"). Decided by the owner on 2026-11-20, when deciding RFC 0008 (#172). Why: the member that moves a v1 workspace holds the role `migration` in its `spaceMember` record, and a role file of that name would make it a session role. *Not taken:* writing the move as the device's own member, which would need no role of its own but could not tell the move's changes from the developer's work.

## History

- 2026-11-17: published as a draft in #157.
- 2026-11-20: decided by the owner (NEX-44), in #157.
- 2026-11-20: `migration` added to the frame's roles, from RFC 0008 (see "Decisions"), in #173 (NEX-83).
