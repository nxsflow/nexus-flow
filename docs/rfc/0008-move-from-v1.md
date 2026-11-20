# RFC 0008: the move from v1 to NoCentralDB

Status: Decided (2026-11-20)

This RFC designs how an existing v1 workspace and all its data reach NoCentralDB without loss. It follows `factory/GENESIS.md`, "What it must never do" ("Never break an existing workspace or its stored data. Format changes are additive or come with a migration, including the move from v1 to NoCentralDB"), and builds on the frame of RFC 0001 and on the module RFCs that say where each piece of v1 data goes: RFC 0002 for tickets, RFC 0003 for decisions, RFC 0006 for the factory and the end of chat. It lists every place v1 keeps data, says for each where it goes or that it ends, and designs `nxs migrate`: what it reads, what it writes, how v1's history and authors survive, how a second run, a half-finished run and a rollback behave, how a workspace synced across several devices moves, what happens to the v1 binaries and embedders still installed, and how the move is proven. Once it is decided, a coding ticket can build `nxs migrate` on the seam.

## What it solves

`nxs` is in daily use on v1, in this repository and in nexflow.it and manufakt.io. v2 keeps its data in NoCentralDB spaces (RFC 0001), so every v1 workspace that keeps working in v2 has to move once. The owner and the agents need from the move:

- **Nothing lost silently** (GENESIS.md, principle 4). Every v1 record either arrives in the repository space, or ends by a decision the owner made, and anything the move cannot carry is named in its report.
- **The team chooses the moment.** The move is an explicit command, run once per workspace, safe to run again; the v1 data stays untouched on disk and is the rollback (decided by the owner, NEX-22).
- **History in full.** Every v1 op is replayed as a NoCentralDB change written by a migration member, carrying the original author and time as data, so a ticket's history survives (NEX-19).
- **One copy that is written.** Once a workspace has moved, no v1 binary writes to it any more, and says why (NEX-22).
- **Proof.** The move checks itself: every v1 record is compared with what arrived, and the move is complete only when they match.

The owner decided the main choices before this RFC was written, and the text below carries them: the explicit `nxs migrate` with a marker and the untouched v1 data as rollback (NEX-22); every op replayed with its original author and time (NEX-19); memories become decisions by a mechanical mapping (NEX-21, written into RFC 0003); chat lapses, neither moved nor exported (NEX-20, written into RFC 0006); v1's sync and `nxf-relay` retire for NoCentralDB's relay, which manufakt.io hosts (NEX-17); the move concerns nexus-flow's coding workspaces, nexflow.it stays on v1 and manufakt.io moves to v2's library seam (NEX-18). On 2026-11-20 the owner answered the questions this RFC asked: how a done v1 item is found and is on main (NEX-78), how v1 binaries refuse to write (NEX-79), what becomes of forgotten memories (NEX-80), and that workspaces of other plugins are refused (NEX-81). The answers are written into the text below.

Out of scope: moving this repository's factory off Linear, the tracker CONFIG.md names until v2 carries tickets (Linear is not v1 data, and the factory's own move is its ticket); the text of the role files and of `factory/FACTORY.md` once they read decisions with `nxs` (RFC 0003, "Migration", gives it to the move's ticket); the release that carries v2 to installed machines (`docs/specs/release-management.md`); and removing v1's crates from `v2` once the move no longer needs them, which is its own ticket (RFC 0001, "What becomes of v1's crates").

NoCentralDB RFCs are cited as "NoCentralDB RFC", by number and section, as in RFC 0001; an RFC number without that prefix is one of nexus-flow's own. Requirements on NoCentralDB continue the numbering of RFCs 0001 to 0003.

## The design

### What v1 holds

v1 keeps a workspace in `.nxs/` at the root of a repository (`crates/foundation/src/workspace.rs`), and a little more under the user's home. One SQLite file, `.nxs/db.sqlite`, holds the op log of all three products (flow, memory, chat) and the views folded from it. Only the op log syncs; everything else is local to the replica, that is to one `.nxs/` on one machine (`docs/specs/E4-auth-identity-and-signed-ops.md`, 2.1). The table lists every store, where it goes, and who decided its end.

**The op log, `ops` in `.nxs/db.sqlite`** (synced; the source of truth). Each op has `op_id`, `lamport`, `site`, `domain`, `target_kind`, `target_id`, `field`, `op_type`, `value`, the self-declared `author`, `wall_clock`, the signature columns `key_id` and `sig`, and the receiver's verdict `provenance` (`own`, `verified`, `invalid` or `unsigned`). An op's kind is the tuple of domain, target kind, op type and field:

| Op kind | Holds | Goes to |
| --- | --- | --- |
| flow `item` `set` (the fields `type`, `title`, `description`, `design`, `completion_criterion`, `status`, `priority`, `due`, `defer_until`, `assignee`, `closing_comment`, `deleted`, `archived`, `closed_at`) | A ticket's fields | `ticketsTicket`, by RFC 0002's table ("Migration") |
| flow `edge` `add`/`remove` (`dep`, `contributes_to`, `mentions`, `parent`) | Edges | `dependsOn`, `contributesTo`, `mentions`, `parent` |
| flow `label` `add`/`remove` | Labels | `labels` |
| flow `note` `note_add`, and `set` of `deleted` | Notes and their redaction | `ticketsNote`; a redacted note see "Redacted notes" |
| flow `field` `set`/`add`/`remove` | Plugin custom fields and list fields (`nxf entry`) | Not in the model (NEX-23); see "Workspaces the move refuses" |
| flow `chunk` `append`/`supersede` | Opaque chunks, written only by nexflow.it's plugin | Not in the model (RFC 0002); kept in the op's migration record |
| flow `thread_link` `add`/`remove` | Links from tickets to chat threads | Lapse with chat (NEX-20, RFC 0002); kept in the op's migration record |
| memory `fact` `set`/`forget` (`body`, `introduction`, `category`, `scope`, `refs`, `ordinal`) | Memories | Decisions, by NEX-21's mapping (see "Memories") |
| memory `nxm:migration` mark | v1's own import mark, never folded | Kept in its migration record; no view |
| chat `message`, `channel`, `profile`, `membership`, `thread`, and the retired `read_cursor` and `workflow` kinds | Chat | Lapse (NEX-20, RFC 0006): not read, not recorded, counted in the report |

**The views in `.nxs/db.sqlite`**, folded from the op log and rebuildable: flow's `items`, `edge_adds`/`edge_removes`, `label_adds`/`label_removes`, `thread_link_adds`/`thread_link_removes`, `notes`/`note_tombstones`, `chunks`/`chunk_superseded`, `custom_fields`, `field_entries`, `item_timestamps` and the `present_*` views; memory's `memories`; chat's `messages`, `channels`, `membership_*`, `profiles`, `threads`; and `view_watermarks`. They are not read: the move folds the op log itself (see "How the ops are replayed"), so a view that is behind or damaged cannot change what arrives. flow's history is no table; v1 reads it from `ops`, and so does the move.

**Local tables in `.nxs/db.sqlite`** (not synced):

| Table | Holds | Goes to |
| --- | --- | --- |
| `trusted_keys` | The replica's trust list | Ends. Its verdicts survive as each op's `provenance` in its migration record. NoCentralDB's members and certificates replace it (RFC 0001, "Members"). |
| `replica_meta`, `log_base` | Migration bookkeeping and a based store's clock floor | End. A based store (one started from a format-2 snapshot) holds no history before its base and is refused, see "Workspaces the move refuses". |
| `next_page_cache` | Pagination | Ends (RFC 0002) |
| chat's local tables (`session_map`, `agent_transcript`, `parked_work`, the liveness ladder and the rest RFC 0006 lists) | Orchestration state | End (RFC 0006). `parked_work` is reported, see "What the move reports". |

**Files in `.nxs/`:**

| File | Holds | Goes to |
| --- | --- | --- |
| `replica.toml` | The replica's `site_id`, item-id `prefix` and `replica_uuid` | Read: it names the replica in the migration run. Ends. |
| `config.toml` | `active_modules`, `[flow] plugin`, `[workspace] name`, `[nxs] service`, `[chat.liveness]` | Read for the plugin and the name. Ends; v2's settings are `nexus-flow.toml` (RFC 0001). |
| `signing.key` | The replica's Ed25519 key | Never read or copied. v1's key ids and signatures survive in the migration records, so v1's signed log stays checkable; v2 signs with NoCentralDB's member keys. |
| `sync.toml` | The stream id, the sync cursors and an endpoint URL that may carry a password | The stream id is read: it says which replicas are one workspace (see "Several devices"). Nothing else is read; the endpoint is never copied anywhere. Ends with v1's sync (NEX-17). |
| `last-write`, `timers.json`, `border-open`, `memory-publish.json` | Markers for the v1 service, the chat timers and the memory publisher | End |
| `run.sqlite`, `agent-logs/` | The run engines | End (RFC 0006) |
| `files/` | Blobs of the test-only `file` plugin | End with custom fields (see "Workspaces the move refuses") |

**Beside the workspace and under the home directory:**

| Store | Holds | Goes to |
| --- | --- | --- |
| `.nxs-personas/`, `~/.nexusflow/personas/` | Persona and channel declarations | End (NEX-20); the legal-text team becomes role files by hand (NEX-26) |
| `<repository root>-v2/` and the `nxs/park/…` branches | The run engine's second working copy and parked work | Stay in git and on disk; reported (RFC 0006, "Migration") |
| `NEXUS_MEMORY.md`, the refs `refs/nexus-memory/*` and the branch `nxm/publish-memory` | The memory publisher's output | Stay in git, untouched; v2 writes no memory file (GENESIS.md, "Decisions and rules"). The team deletes them when it wants to. |
| `factory/decisions/` | A repository's decision log as files | Moved into the repository space by RFC 0003's mapping (see "The decision files") |
| `~/.nexusflow/workspaces.toml` | The v1 service's registry of workspaces | Read, to list the v1 workspaces of the machine (see "What the move reports"). Ends with the service. |
| `~/.nexusflow/config.toml`, `machine.toml`, `sync-daemon.*`, `orders/`, `logs/`, `bin/`, the launchd agent | The v1 sync service | End with v1's sync (NEX-17). The move does not stop or uninstall the service; see "The installed base". |
| The default MCP workspace under the platform data directory (`com.nxsflow.nxs/.nxs/`) | A full v1 workspace that belongs to no repository | Moved like any other, into a repository the team names (see "Workspaces outside a repository") |
| `~/.config/nxf/config.toml`, `~/.cache/nxf/update-check.toml` | v1's self-update channel and hint cache | End with v1's binaries |
| The relay's stores (`stream_ops`, the prefix registry and machine presence, in SQLite, Postgres or DynamoDB) | A copy of each stream's op log, on the relay operator's machine | Not read and not changed. Every op the relay holds is in some replica, and each replica runs the move (see "Several devices"). Whoever runs the relay decides when to delete its data. |

Nothing else in v1 persists data (`crates/` read on 2026-11-20). The goals, status notes and coordination spaces of v2 are new and have nothing in v1 to move.

### The command

`nxs migrate` moves the v1 workspace of the repository it runs in, and that repository's `factory/decisions/`, into the repository space `nexus-flow.toml` names. It is run once per replica (decided by the owner, NEX-22), on a developer's device that is an admin of the repository space, after `nxs init` (RFC 0001):

```
nxs migrate [--check] [--from <path>] [--undo] [--md]
```

- **Without a flag**, it moves, or carries on a move that stopped, or catches up what a replica holds that the space does not yet (below), and reports.
- **`--check`** reads everything the move would read, writes nothing, and prints the same report with what the move would do and every refusal. It is the dry run a team reads before it chooses its moment.
- **`--from <path>`** names a v1 workspace directory (a `.nxs/` folder) other than the repository's own, for a workspace that belongs to no repository.
- **`--undo`** rolls the move back on this replica (see "Rolling back").

It refuses, with an error that names the problem, when the repository has no `nexus-flow.toml` (`no_space`: run `nxs init`), when the device is not an admin of the repository space (`not_admin`), when there is neither a v1 workspace nor a `factory/decisions/` (`nothing_to_migrate`), when a v1 process holds the workspace open, such as the v1 sync service or a chat session (`v1_in_use`), and in the cases of "Workspaces the move refuses". Every refusal leaves the v1 data and the space as they were.

The move writes from the local copy and never waits for the network: it is a write, and the changes reach the relay when the device next syncs (RFC 0001, "Sync"). It does not sync v1 either: it reads the replica as it is on disk (see "Several devices").

*Not taken:* moving on the first run of v2, and two writable copies for a while (NEX-22); a separate command per kind of data (`nxs migrate tickets`, `nxs migrate memories`), which would let a team move half a workspace and leave the other half writable in v1.

### The migration member

Every v1 op is written by a migration member (NEX-19). `nxs migrate` creates one per run, on the device that runs it:

1. It creates the member's keys and database under `<data dir>/migration/<run>/` (RFC 0001, "The local data on disk").
2. The device pairs it into the developer's account as an `agent` (NoCentralDB RFC 0010, "Adding a device or an agent to an account"), both ends of the pairing in the same process, since the device already belongs to the account.
3. The device adds it to the repository space as writer, with a `spaceMember` record of role `migration` and a free ticket tag (RFC 0001, "The role record"; RFC 0002, "Ticket ids").
4. The run writes every change as this member.
5. At the end of the run, the device removes the member and deletes its database. Its changes stay valid (NoCentralDB RFC 0008, "The cutoff"), and its tag becomes free and is reused with its numbering continued (RFC 0002), so no key repeats.

Every change of the move is therefore signed, and its chain shows the developer's account, the migration agent and its `migration` role: the space shows who moved the data, and each change carries who wrote the v1 op it replays. `migration` joins `owner`, `developer` and `app` as a role of the frame that names no role file (RFC 0006, "The factory's files").

*Not taken:* writing as the device's own member (the space could not tell the move's changes from the developer's own work), and a sub-agent (a sub-agent is added by an agent, not by a device; NoCentralDB RFC 0003, "Sub-agents get an ad-hoc space"; RFC 0009, "Sub-agents and ad-hoc spaces").

### How the ops are replayed

The move reads the op log of the replica, in v1's own order, and writes one NoCentralDB change for each op that is not chat (NEX-19, NEX-20). v1 orders ops by `lamport`, then `site`; three of twenty real workspaces hold ops with the same pair (`crates/foundation/src/schema.rs`), so the move breaks that tie by `op_id`.

**The fold decides, not the op.** A v1 op's effect depends on what its writer had seen: a concurrent write to the same field loses by v1's last-writer-wins rule, and an edge removal removes only the adds it observed. Applying each op's raw effect in a sequence would therefore give a different result where v1 had concurrency. So the move folds the ops, one at a time, with v1's own reducers (`crates/core`, `crates/memory`) into an in-memory copy of the views, and writes, for each op, the difference the op made to the views: the fields it set, the edges and labels it added or removed, the note it added. An op that changed nothing, because it lost to a concurrent write or repeated one, still becomes a change, which carries only its migration record. After the last op, the space holds exactly what v1's views hold, and every intermediate state of a ticket is one v1 had.

The move reads the database read-only, with its own reader, and never through v1's `Store::open`, which writes on open (it migrates the schema, mints a `replica_uuid` and seats the signing key). It reads v1's schema 9; see "Workspaces the move refuses" for other versions.

**One change per op, in order.** Each change is one NoCentralDB write (NoCentralDB RFC 0009, "One write, one change"), so a reader never sees half an op, and a run that stops after any change can carry on from the next op.

**What each change writes.** For an op on an item: the ticket's fields from the fold, with `updatedAt` set to the op's `wall_clock` and `updatedBy` to `v1:<author>`, its v1 author as written; the item's first op creates the ticket, with `createdAt` and `createdBy` the same way. For a note: a `ticketsNote` with `createdAt` and `createdBy` from the op. RFC 0002 lets `createdBy` hold v1's author for migrated records ("Who created a ticket"); the prefix `v1:` tells it from a member id. Each change also writes the op's migration record (below).

**Keys.** A ticket's key is minted when its item's first op is replayed, with the migration member's tag and numbers in that order, so the oldest v1 item gets `<tag>.1` (RFC 0002, "Migration"). The v1 item id (`6j6v.pzkb`) is written as the ticket's first note, "Moved from v1 as `6j6v.pzkb`.", so old references can be found (RFC 0002), and it stays in the item's migration records.

**Assignees and authors stay as v1 wrote them.** v1's `assignee` and `author` are free text that v1 never checked. The move keeps them as text (`assignee` without a prefix, the `text` kind of RFC 0002, "Who holds a ticket"; `createdBy` and `updatedBy` as `v1:<author>`), and maps no name to a member or an account. *Not taken:* a mapping the team gives at the move, from v1 names to accounts, which would make the move ask for input it cannot check, and would write an identity v1 never recorded. A team that wants a ticket held by a member claims it in v2.

**Closed items and their work.** v1 records no work for an item, and v1-era commits name v1 item ids in their titles (`6j6v.pzkb — …`), not the keys the move mints. The move therefore finds a closed item's work by its v1 id, and "on main" follows it (decided by the owner, NEX-78):

- Every moved ticket keeps its v1 item id in a new field of RFC 0002's `ticketsTicket`, `v1Id` (`nc.string()`, unset for a ticket created in v2), written when the ticket is created and never changed, with the index `byV1Id` (**`v1Id`**, `scope: "space"`).
- A closed item is `done` when a commit reachable from the base branch names its v1 id in its title, as a token standing alone. For each such commit the move records a work link (RFC 0002, "Work links"): a `pr` link with the pull request's URL, `state` `merged` and the commit, when the title ends in `(#<n>)` and the repository's `origin` remote is on GitHub; otherwise a `branch` link named after the base branch, with the commit.
- A closed item without such a commit is `canceled`, with its closing comment kept, as RFC 0002's table has it.
- RFC 0002's on-main rule also accepts the v1 id: a done ticket with a `v1Id` is on main when a commit reachable from the base branch names its key or its v1 id in its title. A range of v1 ids does not exist, so the v1 id is matched alone.

This is an additive change to decided RFC 0002 (one field, one index, one rule; see "Changes to the decided RFCs"). Its limit: a commit title that names a v1 id for another reason makes the item `done` and on main. *Not taken:* RFC 0002's table alone, where every closed item would read `canceled` for ever, in `recap`, the daily note and manufakt.io; and an artifact link on every closed item, which stretches "artifact" and would call on main what never merged.

### The migration records

Each replayed change carries one object of a new collection, `migrationRecord`, the op's migration record. It holds everything the op was, so nothing of an op is lost even where the model has no field for it, and v1's signed log stays checkable after the move:

| Field | Type | Holds |
| --- | --- | --- |
| `run` | `nc.link("migrationRun")` | The run that wrote it |
| `source` | `nc.oneOf(["op", "file"])` | A v1 op, or a decision file (see "The decision files") |
| `origin` | `nc.string()` | The v1 `op_id`, or the file's path |
| `position` | `nc.int()` | The op's place in v1's order within the run |
| `kind` | `nc.string()` | The op's domain, target kind, op type and field, joined by `/`, or `decision-file` |
| `item` | `nc.string()` | The v1 item id or memory key the op is about, or the decision file's name |
| `ticket` | `nc.link("ticketsTicket")` | The ticket the op wrote to, for a flow op |
| `author` | `nc.string()` | The op's `author`, as v1 wrote it |
| `wallClock` | `nc.string()` | The op's `wall_clock`, RFC 3339 |
| `lamport`, `site` | `nc.int()` | The op's v1 coordinates |
| `keyId`, `sig` | `nc.string()` | The op's v1 signature envelope, unset for an unsigned op |
| `provenance` | `nc.oneOf(["own", "verified", "invalid", "unsigned"])` | The verdict of the replica the move read |
| `payload` | `nc.string()` | The op's canonical bytes (`Op::canonical_bytes`), base64url, from which its signature can be checked again; unset for a withheld op (see "Redacted notes") and for a file, whose content is in git |
| `outcome` | `nc.oneOf(["applied", "noEffect", "dropped", "withheld", "lateConflict"])` | What the move did with it: wrote its effect, found none, kept it only here (chunks, thread links, custom fields), withheld it, or kept it here because v2 had changed the field since (see "Several devices") |

Indexes, each with `scope: "space"`: `byOrigin` (**`origin`**), which finds an op already moved; `byTicket` (**`ticket`**, `position`), a ticket's v1 history in order; `byItem` (**`item`**, `position`), the same for a memory or an item; and `byRun` (**`run`**, `position`).

**History is readable at once.** `nxs ticket history <key>` (RFC 0002) waits for R6 to read who signed each change. For the part of a ticket's history that comes from v1 it does not need to: it reads the ticket's migration records through `byTicket`, oldest first, with each op's v1 author, time and what it changed, and then the v2 changes once R6 lands. This adds a reader to RFC 0002's `history`, not a new command.

**The records are write-once.** Like decisions, they are the record of what happened (RFC 0003, "Append-only"): the storage crate refuses every write to an existing `migrationRecord` object, and the collection declares R12 once NoCentralDB offers it, as the decision collections do. A `migrationRun` is not write-once: its `state`, `last`, `endedAt` and `report` change as the run goes on, and only `nxs migrate` writes them.

**Who created a moved ticket.** Once R6 lands, RFC 0002 flags a ticket whose `createdBy` differs from the signer of its create (`creator_mismatch`). A moved ticket's create is signed by a migration member and its `createdBy` names a v1 author, so the flag is not raised when `createdBy` starts with `v1:` and the signer's `spaceMember` record has the role `migration`.

*Not taken:* carrying the original author and time only in `updatedBy` and `updatedAt` (they are RFC 0001's author stop-gap and go once R6 lands, NEX-42, and they cannot hold a signature or an op the model has no field for); and a single export file of the v1 log in the repository, which NEX-19 decided against.

### The migration run

One `migrationRun` object per run records the run:

| Field | Type | Holds |
| --- | --- | --- |
| `workspace` | `nc.string()` | The v1 workspace's identity: its `sync.toml` stream id when it is bound to one, else its `replica_uuid` |
| `replica` | `nc.string()` | The `replica_uuid` and `prefix` of the replica the run read |
| `member` | `nc.string()` | The migration member's id |
| `startedAt`, `endedAt` | `nc.string()` | RFC 3339 |
| `state` | `nc.oneOf(["replaying", "verifying", "complete", "verificationFailed", "undone"])` | Where it is |
| `ops`, `last` | `nc.int()` | How many ops the run will replay, and the position of the last one written |
| `report` | `nc.string()` | The run's report as JSON, once it ends |

Index `byWorkspace` (**`workspace`**, `startedAt`), `scope: "space"`.

### The marker

When the run starts, before it replays anything, the move marks the replica as moved, so that no v1 binary writes to it from then on (NEX-22). It writes `.nxs/migrated.toml`:

```toml
# .nxs/migrated.toml, written by nxs migrate. Remove only with nxs migrate --undo.
format = 1
space = "<NoCentralDB space id>"
run = "<migrationRun object id>"
migrated_at = "2026-11-20T21:00:00Z"
by = "nxs <version>"
```

**Every v1 binary refuses the marked workspace** (decided by the owner, NEX-79). v1 has one mechanism that every release since schema 4 honours: a database whose header asks for a newer schema than the binary speaks is refused on open, with no write (`crates/foundation/src/schema.rs`, `assess_open`). Together with the marker, the move raises the header of `.nxs/db.sqlite` to 10, both `PRAGMA user_version` and the compatibility floor in `PRAGMA application_id`, and records the old values in the marker:

```toml
[header]
user_version = 9
application_id = 4
```

Every v1 binary already installed then refuses the workspace, the v1 sync service and chat included, with v1's own message, "written by a newer nxs — upgrade nxs". A last v1 release from `main` reads the marker and, instead of refusing outright, opens the workspace read-only and says that it moved, to which space, and that `nxs migrate --undo` rolls it back, so a team can still read its old chat threads with v1 (NEX-20). That release is a requirement on `main` and its own ticket; the move does not wait for it, since older binaries refuse the workspace either way.

The marker file and the two header fields are all the move writes to the v1 workspace; no row and no other byte of v1's data changes, and `--undo` restores the header from the marker. *Not taken:* the marker and a last v1 release only, which would leave every older v1 binary writing to a moved workspace, its writes reaching v2 only as late ops; and the raised header only, with which v1 could no longer read the workspace at all and would never say why.

The marker is written before the replay, not after, so that no v1 write lands between the moment the move read the log and the end of a long run. A run that stops half way therefore leaves the replica marked: v1 refuses to write until the run is carried on or undone, which the refusal says.

### A second run, and a run that stopped

`nxs migrate` is safe to run again (NEX-22). It looks up the runs for this workspace through `byWorkspace`, and the replica's ops through `byOrigin`:

| What it finds | What it does | `state` it reports |
| --- | --- | --- |
| No run | Starts one | `complete` |
| A run in `replaying` or `verifying`, from this replica | Carries it on: the migration member of the stopped run is gone with its process, so it adds a new one, and replays from the op after `last`, then verifies | `complete` |
| A complete run, and every non-chat op of the replica has its record | Writes the marker if this replica has none, and nothing else | `already_complete` |
| A complete run, and the replica holds ops without a record | Catches them up (see "Several devices") | `caught_up` |
| A run in `verificationFailed` | Refuses with `verification_failed` and the run's mismatches; the team reads them and runs `--undo` or files a ticket | — |
| An undone run | Starts a new run | `complete` |

A run that stopped half way is never replayed from the start: every change it wrote is complete, its records name the ops it replayed, and the fold of the ops before `last` is rebuilt in memory from the log, so the next op's difference is the same as in an uninterrupted run.

### Verifying

After the last op, the run checks what arrived against v1's fold, object by object, and sets `complete` only when nothing differs:

- every v1 item that is not deleted is a ticket with the same field values (by RFC 0002's mapping), parent, dependencies, contributions, mentions, labels and archive state; every deleted item is a deleted ticket;
- every note is a note with the same body, author and time, and every redacted note is a deleted, withheld note;
- every memory that is not forgotten is the decision NEX-21's mapping gives, and no forgotten memory is a decision (see "Memories"); every decision file is the decision RFC 0003's mapping gives;
- every closed item is `done` with its work links, or `canceled`, by the rule of "Closed items and their work";
- every op that is not chat has exactly one migration record, and the records' `payload` checks against `sig` wherever v1 recorded the op as `own` or `verified`.

A difference sets `verificationFailed`, and the report lists each one with the v1 value and the space's value. A change another member makes to a migrated ticket during the run is such a difference, so the report says to run the move before the team works in v2.

### Memories

Memories become decisions in force by NEX-21's mapping, as RFC 0003 writes it ("Migration"): the introduction is the `title`, the body the `body`, the day of the last write the `date`, the author the `decidedBy`, `no ticket`, the key as a topic, and "carried over from memory" as `why` and `insteadOf`. RFC 0003 left four details to this RFC:

- **The topic from the key.** A topic is a lowercase tag; the key is turned into one by lower-casing it and joining its runs of letters and digits with `-`, so an automatic key `f-<sha256>` stays `f-<sha256>`.
- **A memory with no introduction** takes the first line of its body, cut to 200 characters on a word boundary, as its title (RFC 0003, "Writing a decision": a title is one line of at most 200 characters).
- **Category** becomes a second topic, the same way, unless it is v1's default `unsorted`. **References to board items** (`refs`) become one `decisionsSubject` each, with the moved ticket's key as `label` and its link as `ticket`, or the v1 item id as `label` when that item did not move.
- **Scope** (`item`, `project`, `global`) and **reading position** (`ordinal`) have no place in a decision: a repository space is one project, and decisions are read by date. Both stay in the memory's migration records.

**Forgotten memories** are not written as decisions (decided by the owner, NEX-80). v1's `forget` hides a memory and can be undone, so a memory forgotten at the time of the move is one the team took back. Its ops are replayed like every other memory op, as changes that write their migration records, so its text, its history and the forget itself stay in the space; but no decision is written for it. The run's report lists each one (`forgotten_memory`) with its key, its last introduction and body, and when it was forgotten, and the report is stored on the run (`migrationRun.report`): `nxs migrate --check` on a moved workspace prints the stored report of its complete run, so a reader finds every forgotten memory there, and its full history in its migration records through `byItem`. A memory that was forgotten and then restored in v1 is not forgotten, and becomes a decision. *Not taken:* writing them as decisions withdrawn by one superseding decision, which keeps them in `nxs decision list` at the cost of a decision in force that says only that; and writing them in force, which would make rules a team took back rules again.

A decision takes no write after the change that creates it (RFC 0003, "Append-only"), so a memory cannot follow its ops change by change as a ticket does. Each memory op is replayed as a change that writes its migration record, in order, and once every op is replayed, each memory's decision is written in one change of its own, oldest memory first. The decision's `author` is the migration member; `nxs decision show` lists the memory's history through `byItem`.

### The decision files

A repository's `factory/decisions/` moves with the same run, after the memories, by RFC 0003's mapping ("Migration"), one decision per file, older first, so a supersession always finds the decision it names. Each file also gets a migration record with `source` `file`, its path as `origin` and `item`, and its git blob id in `kind` (`decision-file:<blob>`), so a second run moves only the files without a record.

The move reads the files from the commit the base branch points at (`[git] base`, RFC 0001), not from the working tree: a decision on an unmerged branch is not one the team made yet. A file merged after the move is moved by the next run, so decisions of pull requests that were open during the move are not lost. A `Supersedes:` line that names a file the folder does not hold keeps its text in `body`, and the report names it (`unresolved_supersedes`).

The folder stays in git, untouched. From the move on, the factory records decisions with `nxs decision add`; switching the roles and FACTORY.md to it is the move's ticket (RFC 0003, "Migration").

### Redacted notes

v1's only way to take text back is to redact a note. A redacted note's body is not written into the space: the note is created deleted, with an empty body, and its migration records keep the op's metadata and signature but no `payload` (`outcome` `withheld`). The v1 data on disk stays the only copy of the text, so nothing is lost, and a text someone took back, perhaps because it held personal data (GENESIS.md, "What it must never do"), is not copied into every member's log, where NoCentralDB could not erase it (RFC 0003, "Personal data and erasure"; R13). A deleted item is different: deleting was not a redaction in v1, so its fields move and the ticket is deleted after them.

### Several devices

A workspace synced through `nxf-relay` has one replica per device, each with the same stream id and its own `replica_uuid` and site. v1's sync does not move with v2 (NEX-17), so each replica moves on its own device, and the space joins them:

1. **The first device** runs `nxs migrate`. It replays every op its replica holds, of every site, and records the stream id as the run's `workspace`. Its report lists the other sites it found in the log, with their item prefix and the time of their last op, so the team knows which devices still have to run the move.
2. **Every other device**, once its v2 database has synced the space (`nxs sync`), runs `nxs migrate` too. It finds the complete run of its workspace, writes its own marker, and catches up the ops its replica holds without a record: ops that device wrote and never pushed, or that reached it after the first device read its log.
3. **The others meanwhile** see nothing change in v1 until they run the move: a v1 binary on another device keeps writing its own replica. Its writes reach the space when that device runs the move, by the catch-up below.

**Catching up.** The catch-up folds every op of the space's records together with the late ops, in v1's order, and compares the result with the fold of the records alone. Where a late op changes a field, an edge, a label or a note, the move writes the change, as one change per late op with its migration record, when the space still holds the value the move wrote. Where v2 has changed it since, v2's value stays, and the late op is kept in its record with the `outcome` `lateConflict`; the report lists each one with both values, and the team decides. Nothing is lost: the op is in the record, and in the replica on disk.

The team moves best by syncing every device with v1 before upgrading any of them, so that the first replica holds every op and there is nothing to catch up. The move cannot sync v1 for them: v2 has no v1 sync client, which lets v1's sync crates go with v1 (RFC 0001, "What happens to v1's sync").

A second clone of the same repository on one machine is a second replica of the same workspace (E4, 2.1) and moves the same way. A worktree shares its root's `.nxs/` through a symlink, so it is the same replica, and its run is `already_complete`.

*Not taken:* a v1 op written into the stream to tell every replica that the workspace moved, which only a v1 release that knows the op could read, and which needs v1's sync on the v2 side.

### Rolling back

The untouched v1 data is the rollback (NEX-22). `nxs migrate --undo` rolls the move back on the replica it runs in:

1. It sets the header of `.nxs/db.sqlite` back to the values the marker recorded and removes `.nxs/migrated.toml`, after which v1 writes to the replica again.
2. It sets the run's `state` to `undone`, and deletes the run's tickets, with their notes, work links and references (NoCentralDB deletion can be undone, NoCentralDB RFC 0004, "Creating and deleting an object").
3. Decisions are append-only and cannot be deleted (RFC 0003). It writes one decision instead that supersedes every decision the run wrote, titled "The move from v1 of <workspace> was undone", so none of them stays in force.
4. The migration records stay: they are the record of a move that happened and was undone.

A later `nxs migrate` starts a new run, with new tickets and keys. In a workspace on several devices, `--undo` runs on each device that wrote a marker; the space's run is undone by the first.

Rolling back is meant for the time before the team works in v2. Work done in v2 on a moved ticket since the move stays only in the deleted ticket, which v1 never sees; the report of `--undo` lists every deleted ticket that a member other than the migration member changed.

### Workspaces the move refuses

The move refuses a workspace it cannot carry faithfully, says why, and changes nothing. The v1 data stays readable and writable with v1.

| Case | Error | Why |
| --- | --- | --- |
| The workspace's `[flow] plugin` is not `issue-tracker` | `unsupported_plugin` | The model is the `issue-tracker` plugin's (NEX-23), and v2 is a tool for coding work (NEX-18). A `personal-todo` workspace and a consumer's, such as nexflow.it's, stay on v1 (decided by the owner, NEX-81). *Not taken:* mapping `personal-todo` into the ticket model (`project` to `epic`, `todo` and `termin` to `chore`, its priorities to `P0` to `P4`), a lossy mapping for data that is not coding work. |
| The database is at a schema below 9 | `v1_format_old` | The move's reader knows v1's last schema only. Opening the workspace once with the last v1 release brings it to 9. |
| The database is at a schema above 9, which a v1 release after this RFC wrote | `v1_format_unknown` | The move cannot read what it does not know; v2 then needs a reader for that schema before it moves the workspace. |
| A based store (`log_base`), started from a snapshot | `v1_based_store` | Its history before the base is not in the log, so the move cannot replay it in full (NEX-19). The team moves a replica that holds the full log instead, if one exists. |
| An op whose fold fails in v1's own reducers | `v1_fold_failed` | The views v1 shows could not be rebuilt either; the op is named. |

Custom fields and list fields of an `issue-tracker` workspace, which `nxf entry` wrote, are not refused: they have no field in the model (NEX-23), and their ops are kept in their migration records with `outcome` `dropped`. The report lists every ticket that held one, with the field and its last value. Chunks and thread links are kept the same way.

### Workspaces outside a repository

A v1 workspace that belongs to no repository, such as the default workspace v1's MCP server creates under the platform data directory, has no repository space of its own. The team moves it into the repository space it belongs with by running `nxs migrate --from <path to .nxs>` in that repository. Several workspaces can move into one space: each is a run of its own, and their tickets get keys from their own run's migration member, so no key repeats.

### What the move reports

`nxs migrate` prints JSON by default and Markdown with `--md` (RFC 0001, "Reading and writing"). Besides the counts and the run's state, it reports what can hold work that is stored nowhere else, so that the team decides about it (RFC 0006, "Migration"; principle 4):

```json
{
  "state": "complete",
  "workspace": { "path": ".nxs", "id": "<stream id or replica_uuid>", "replica": "<replica_uuid>", "prefix": "6j6v", "plugin": "issue-tracker", "schema": 9 },
  "space": "<space id>",
  "run": "<migrationRun object id>",
  "member": "<migration member id>",
  "tag": "mg",
  "counts": {
    "ops": 9268, "applied": 8890, "no_effect": 310, "dropped": 60, "withheld": 8, "late": 0, "late_conflicts": 0,
    "chat_ops_skipped": 41022, "tickets": 1204, "notes": 2113, "memories": 142, "decision_files": 0
  },
  "verified": true,
  "reports": [
    { "kind": "parked_work", "branch": "nxs/park/…", "commit": "…" },
    { "kind": "second_working_copy", "path": "../nexus-flow-v2", "clean": false },
    { "kind": "other_replicas", "site": 3, "prefix": "k2p9", "last_op_at": "…" },
    { "kind": "dropped", "ticket": "mg.17", "what": "custom_field", "field": "…", "value": "…" },
    { "kind": "late_conflict", "ticket": "mg.4", "field": "title", "v1": "…", "v2": "…" },
    { "kind": "forgotten_memory", "key": "…", "introduction": "…", "body": "…", "forgotten_at": "…" },
    { "kind": "unresolved_supersedes", "decision": "…", "names": "…" },
    { "kind": "v1_workspace_elsewhere", "path": "…" },
    { "kind": "v1_binary", "path": "…", "version": "…" },
    { "kind": "v1_service", "registered": true }
  ],
  "mismatches": []
}
```

- `state` is one of `complete`, `already_complete`, `caught_up`, `checked` (with `--check`) and `undone` (with `--undo`). `--check` on a workspace whose run is complete prints that run's stored report, with `state` `checked`, and any late ops it would catch up.
- `parked_work`: every row of v1's `parked_work` with no `resumed_at`, with its branch and commit (RFC 0006).
- `second_working_copy`: `<repository root>-v2/`, when it exists and is not clean (RFC 0006).
- `other_replicas`: the other sites in the log (see "Several devices").
- `forgotten_memory`: every memory forgotten at the time of the move, which becomes no decision, with its key, last introduction and body, and when it was forgotten (see "Memories").
- `v1_workspace_elsewhere`: every v1 workspace in `~/.nexusflow/workspaces.toml` and the default MCP workspace that has not moved, so a team finds the workspaces it has not run the move for.
- `v1_binary`: every `nxf`, `nxm`, `nxc` or v1 `nxs` on `PATH`, with its version, and `v1_service`: whether v1's sync service still has the workspace registered (see "The installed base").
- `mismatches`: what verifying found, empty unless `state` is a refusal with `verification_failed`.

Every array is present, empty when there is nothing to say, and ordered by `kind`, then by the report's own keys, bytewise. The codes of the errors are those above: `no_space`, `not_admin`, `nothing_to_migrate`, `v1_in_use`, `unsupported_plugin`, `v1_format_old`, `v1_format_unknown`, `v1_based_store`, `v1_fold_failed` and `verification_failed`.

### The installed base

**v1's binaries.** v1 installs `nxs` with `nxf`, `nxm` and `nxc` as symlinks to it (`install.sh`). v2 ships `nxs` only (GENESIS.md, "Architecture and stack"), so an install of v2 replaces v1's `nxs`, and the three symlinks then point at v2's. v2's `nxs`, when started under one of those names, prints that the command is retired in v2 and names its `nxs` counterpart, and exits with the error `retired_entry_point`. Upgrading the binary moves no workspace (NEX-22): v2's `nxs` in a repository whose `.nxs/` has no marker works on the repository space as RFC 0001 describes, and says on standard error, not in its JSON, that the v1 workspace has not moved and that `nxs migrate --check` shows what the move would do.

A v1 binary that stays installed, or is installed again for a rollback, refuses a moved workspace by its raised header, and the last v1 release opens it read-only and says why (see "The marker"). A replica on another device is refused only once that device has run the move.

**v1's sync service** keeps running after the move until the developer uninstalls it with v1's binary or by hand; the move neither stops nor uninstalls it, since it belongs to v1 and serves the workspaces that have not moved. It refuses a moved and marked workspace like every v1 binary (see "The marker"), and the report says when the workspace is still registered.

**Embedders.** nexflow.it embeds v1's facade and stays on v1 (NEX-18): its workspaces use its own plugin, so the move refuses them (`unsupported_plugin`), and nothing in v2 changes v1's facade on `main`. manufakt.io moves to v2's library seam (NEX-18): it opens a moved workspace's repository space with `Engine::open` or `Engine::open_space` (RFC 0001) and reads no v1 data. What manufakt.io kept of its own beside v1's facade is manufakt.io's to move.

### The library seam

`Engine` gains the counterparts of the command (the decision on CLI and library parity stays in force):

| Symbol | What it does |
| --- | --- |
| `Engine::migrate(MigrateOptions) -> MigrationReport` | Moves, carries on or catches up, as `nxs migrate`; `MigrateOptions` holds `from: Option<PathBuf>` |
| `Engine::migrate_check(MigrateOptions) -> MigrationReport` | `nxs migrate --check` |
| `Engine::migrate_undo(MigrateOptions) -> MigrationReport` | `nxs migrate --undo` |

`MigrationReport` has the fields of the JSON above.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Command:** `nxs migrate [--check] [--from <path>] [--undo] [--md]`, and the refusal of v2's `nxs` under the names `nxf`, `nxm` and `nxc`.
- **JSON output:** the report above, its `state` and `reports[].kind` values, and the error codes `no_space`, `not_admin`, `nothing_to_migrate`, `v1_in_use`, `unsupported_plugin`, `v1_format_old`, `v1_format_unknown`, `v1_based_store`, `v1_fold_failed`, `verification_failed` and `retired_entry_point`.
- **Library seam:** `Engine::migrate`, `Engine::migrate_check`, `Engine::migrate_undo`, `MigrateOptions`, `MigrationReport`.
- **Stored formats:** the collections `migrationRun` and `migrationRecord` with their fields and indexes; the role `migration` in RFC 0001's `spaceMember`; the `v1:` prefix in `createdBy` and `updatedBy`; the first note of a moved ticket; the field `v1Id` and the index `byV1Id` added to RFC 0002's `ticketsTicket`, and the v1 id in RFC 0002's on-main rule; the mapping of a memory's key, category and references; `.nxs/migrated.toml` with its `[header]` table, and the raised header of a moved `.nxs/db.sqlite`; and the migration member's directory `<data dir>/migration/<run>/`.

### Changes to the decided RFCs

This RFC's design and the owner's answers change four places in RFCs that are already decided. Each is rewritten in a follow-up of its own, as a decided RFC changes (`docs/rfc/README.md`, "How a decided RFC changes"); the change to RFC 0002's on-main rule is NEX-83. This RFC states the rule they follow.

- **RFC 0002, "The ticket", "Indexes" and "Derived state":** `ticketsTicket` gains `v1Id` (`nc.string()`, the v1 item id of a moved ticket, unset otherwise, written once) and the index `byV1Id`; a done ticket with a `v1Id` is on main when a commit reachable from the base branch names its key or its v1 id in its title (NEX-78; see "How the ops are replayed"). RFC 0002's "Migration" table points to this RFC for how a closed item becomes `done` or `canceled`.
- **RFC 0002, "Who created a ticket" and "Derived state":** `creator_mismatch` is not raised for a ticket whose `createdBy` starts with `v1:` and whose create a member with the role `migration` signed; `createdBy` and `updatedBy` take the `v1:` prefix for moved records (see "The migration records").
- **RFC 0002, "Commands, JSON and the seam":** `nxs ticket history` and `Engine::ticket_history` read a moved ticket's migration records through `byTicket` for its v1 part, without waiting for R6 (see "The migration records").
- **RFC 0001, "The role record", and RFC 0006, "The factory's files":** `migration` joins `owner`, `developer` and `app` as a role of the frame, and no role file can be named `migration`.

### What nexus-flow needs from NoCentralDB

Nothing new is required for the move itself: it writes ordinary changes as an ordinary member. It rests on what RFC 0001 found met: an agent paired into the person's account (R8), and past changes of a removed member staying valid (NoCentralDB RFC 0008, "The cutoff"). It adds `migrationRecord` to R12 (write-once collections), which RFC 0003 asks for, and reads the v2 part of a moved ticket's history only once R6 lands.

## What it does not do

- **It does not move what lapses.** Chat ops, chat tables, personas, run state and agent logs stay on disk, readable with v1 only (NEX-20).
- **It does not reach other devices.** Each replica moves on its own device; a device that never runs the move keeps its v1 writes to itself, and a v1 binary there writes on until it does.
- **Older v1 binaries do not say why.** A v1 binary older than the last v1 release refuses a moved workspace with v1's message "written by a newer nxs — upgrade nxs", which does not mention the move.
- **A forgotten memory is no decision** (NEX-80). `nxs decision list`, `nxs prime` and manufakt.io's decision views do not show it, so a team that looks for what it once decided does not find it in the decision log. It is found in the move's stored report (`nxs migrate --check`, `forgotten_memory`) and in its migration records; a team that wants it back as a rule records it again with `nxs decision add`.
- **A v1 id named by accident counts.** A commit title that names a v1 item id for another reason makes that item `done` and on main.
- **A late op can lose to v2.** A v1 write that reaches the space after v2 changed the same field is kept in its record and reported, not applied.
- **It does not verify v1's signatures against trust.** It keeps each op's signature and the replica's verdict; it does not decide again whom v1 trusted. An op v1 marked `invalid` is replayed like any other, as v1 folded it.
- **v1 authors are names, not identities.** A moved ticket's `createdBy`, `updatedBy` and `assignee` hold v1's self-declared text; nothing ties them to an account.
- **A moved ticket gets a new key.** Branch names, commit titles and texts that name a v1 item id keep naming it; the ticket's `v1Id`, its first note and its migration records lead from the old id to it, but `nxs` does not take a v1 id as an argument.
- **The move is long for a large workspace,** one change per op: about 9,300 changes for this repository's own v1 board in August 2026. Every member that joins the space pulls them once (R9).
- **Rolling back after work in v2 drops that work from v1's view.** It stays in the deleted tickets in the space.
- **Redacted note texts stay only in v1.** A reader of the space sees that a note existed and was redacted, not what it said.
- **It reads one schema of v1.** A workspace below or above v1's schema 9 is refused until it is brought to it, or until v2 learns the newer one.
- **It does not delete or stop anything of v1**: the data, the relay's copy, the service and the binaries stay until the team removes them.
- **The decision files move from the base branch.** A decision that is on a branch only moves once it is merged and the move runs again.

## Migration

This RFC is the migration. Each module RFC's "Migration" section gives the mapping for its own data, and this RFC carries it out.

## How the move is proven

- **Unit and property tests** on generated v1 op logs, with concurrent writes to one field, concurrent edge adds and removes, duplicate `(lamport, site)` pairs, redacted notes, forgotten memories and deleted items: after the move, the space equals v1's fold, every op has one record, and a run stopped after any change and carried on gives the same space as one that was not.
- **Golden workspaces** built with v1's own binary at schema 9, moved in CI with `NXS_DATA_DIR` in a temporary directory, with the report checked byte for byte.
- **Real workspaces.** Before a release offers the move, the owner runs `nxs migrate` on copies of real workspaces, this repository's v1 board first, with `NXS_DATA_DIR` pointing at a scratch directory and a space that is thrown away afterwards; the run's verification must pass, and its counts are recorded on the move's ticket. A copy of a workspace with personal data never goes into CI.

## Open questions

None. The owner answered them on 2026-11-20 (NEX-78 to NEX-81), and the answers are written into the text above: how a done v1 item is found and is on main under "How the ops are replayed", how v1 binaries refuse to write under "The marker", forgotten memories under "Memories", and other plugins under "Workspaces the move refuses".

## History

- 2026-11-20: published as a draft in #172.
- 2026-11-20: decided by the owner (NEX-78 to NEX-81), in #172.
