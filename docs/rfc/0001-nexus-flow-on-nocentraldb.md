# RFC 0001: nexus-flow on NoCentralDB

Status: Draft

This RFC sets the frame every module of v2 stores its data in: the NoCentralDB spaces nexus-flow uses, who is a member of them and with which role, how nexus-flow's NoCentralDB model is written and grown, where the local copy lives and how `nxs` reads and writes it, how it syncs, how derived state such as readiness is computed, and which library seam manufakt.io calls. It follows `factory/GENESIS.md`, "Architecture and stack": every module stores its data in NoCentralDB, "a ticket space per repository, and a coordination space per group of repositories", every read is local, and `nxs` is the only entry point. Once it is decided, the module RFCs (0002 to 0007) design their collections inside this frame, RFC 0008 designs the move from v1 into it, and a coding ticket can build the shared layer: the pin, the model build, opening a space, members, sync and the seam.

## What it solves

v1 keeps its state in a `.nxs/` workspace of its own: an op-log CRDT on SQLite (`crates/core`), its own signing keys and trust list (`crates/foundation`), its own sync protocol, background service and relay (`crates/sync`, `crates/service`, `crates/server`). v2 moves all of that onto NoCentralDB, so that nexus-flow keeps only what is its own: the modules' rules, the derived state and the `nxs` surface (GENESIS.md, "What we build"). The move is priority 1 (decision of 2026-11-15, "Priorities: migrate to NoCentralDB, then build the new factory, then the rest").

The owner and the agents need from this frame:

- **One place per repository** where a session finds that repository's tickets, decisions and status notes, and one shared place per group of repositories for goals and requests (GENESIS.md, "Work across repositories").
- **A signature on every change** that says which role did the work and which person commissioned it (GENESIS.md, "What has to come back through me": "a ticket always shows who decided").
- **Reads that never wait for the network** (GENESIS.md, principle 3), with deterministic JSON output.
- **Nothing lost on a branch switch or when a session ends** (principle 4): the local copy outlives both.
- **One rule per derived state**, computed the same way by `nxs` and by manufakt.io (GENESIS.md, "Done is not yet on main").

The owner decided the frame's main choices before this RFC was written, and the text below carries them: the dependency and stop-gaps (NEX-13), the space layout (NEX-15), who a member is (NEX-16), the sync (NEX-17), the library seam (NEX-18) and the ticket model (NEX-23).

Out of scope, each in its own RFC: the collections and fields of each module (tickets 0002, decisions 0003, genesis 0004, goals 0005, factory 0006, status 0007), human-readable ticket ids (0002, from NEX-16 and NEX-24), and the move of v1 workspaces and their data (0008). Genesis and the factory stay files in git (NEX-14), so this RFC gives them no space.

NoCentralDB's own design is in nocentraldb/nocentraldb, `docs/rfc/`: 0001 the overview, 0005 the schema and `model.lock.json`, 0006 the local view, 0007 the relay, 0008 membership and roles, 0009 the SDK, 0010 accounts, 0011 plugin field types. This RFC cites them by number and does not restate them. Where nexus-flow needs something NoCentralDB does not offer, it is a requirement on NoCentralDB under "What nexus-flow needs from NoCentralDB", not a workaround here.

## The design

### The dependency on NoCentralDB

nexus-flow depends on NoCentralDB's Rust SDK (`crates/nocentraldb` in nocentraldb/nocentraldb, NoCentralDB RFC 0009) through a pinned git revision in the workspace `Cargo.toml`: `rev = "<full commit hash>"`, never a branch or a tag. The pin moves deliberately, one pull request per move, and that pull request carries nothing else. Because `Cargo.toml` and `Cargo.lock` are code-owned, every move waits for the owner (decided in NEX-13).

Only one crate of nexus-flow depends on the SDK: the storage crate under the seam (see "The library seam"). Every other crate reaches NoCentralDB through it, so that a change in NoCentralDB's API at a pin move touches one crate.

When nexus-flow needs something NoCentralDB lacks, pm files it as a request on NoCentralDB's tracker. nexus-flow may build a stop-gap meanwhile, only behind its own seam, named as temporary in the RFC that brings it, with a ticket to remove it (NEX-13). This RFC brings two stop-gaps: the derived state (see "Derived state") and a local view per session (see "The local copy on disk"). When the RFC is decided, pm opens a ticket to remove each, blocked by the NoCentralDB request it waits for.

*Not taken:* waiting for a crates.io release (priority 1 would stall on another repository's date) and vendoring a copy (two copies drift).

### Spaces

nexus-flow uses two kinds of NoCentralDB space. Rights apply to a whole space, and a link points only to an object in the same space, so the layout follows what must link to what.

| Kind | One per | Holds | Designed in |
| --- | --- | --- | --- |
| Repository space | Repository | The repository's projects and tickets, decisions, and status notes | RFC 0002, 0003, 0007 |
| Coordination space | Group of repositories | The group's goals, and the requests between its repositories | RFC 0005, and the RFC on work across repositories |

**One repository space per repository**, not one per module (NEX-15). Decisions and status notes point at tickets all the time, and only one space lets them use real links. The cost is that a member who may read tickets may also read decisions. When a team needs finer rights, a small extra space can be added later, the way NoCentralDB's sub-agent spaces work; this RFC designs none.

**A coordination space** joins a group of repositories. A repository belongs to at most one coordination space in v2. A ticket in a repository space cannot link into the coordination space; it names a goal or a request by its NoCentralDB object id, and the module RFCs say how such a reference is shown when it cannot be resolved (it dangles until the reader is a member there).

**Genesis and the factory** stay files in git (NEX-14). No space holds them.

#### Creating and finding a space

`nxs init` in a repository creates the repository space when the repository has none, with the developer's device as its first admin (NoCentralDB RFC 0008), and writes the space's id into a committed file at the repository's root:

```toml
# nexus-flow.toml, committed. Holds no secret: a space id grants nothing.
format = 1

[space]
repository = "<NoCentralDB space id>"
coordination = "<NoCentralDB space id>"   # optional

[git]
main = "main"                             # the branch "on main" is checked against
```

A session in a fresh clone or a cloud container finds its space by reading this file; it does not ask anyone. The file is committed so that every clone, worktree and branch names the same space. A space id is not a secret: holding it gives no access, which needs a member key (requirement R1 below).

`nxs space join-coordination <space id>` records a coordination space in the file once an admin of that space has added the repository's pm (see "Members"). `nxs init` never overwrites a `nexus-flow.toml` that already names a space (GENESIS.md: never overwrite a team's configuration).

*Not taken:* keeping the space id only in the local data directory (every clone and every cloud container would have to be told it by hand), and deriving the id from the git remote URL (a renamed or forked repository would silently get a new space).

### Members

A NoCentralDB member is a device, an agent or a sub-agent, each with its own key, and an account groups a person's devices and agents (NoCentralDB RFC 0010). nexus-flow uses three kinds of member:

| Member | Created | Key lives | Removed |
| --- | --- | --- | --- |
| A developer's device | When the developer first runs `nxs init` or `nxs device join` on it | In the device's key store, as NoCentralDB RFC 0010 keeps it | By the developer, when the device is retired or lost |
| An agent session | When the session starts, under the account of the developer who commissioned it | In the session's own process, generated there; never written to the repository, the environment or a log | When the session ends, or by its commissioner when the session is archived |
| manufakt.io | When a developer connects manufakt.io to the space | Where manufakt.io keeps its members' keys | By the developer |

**One member per agent session** (NEX-16). Its name is the role on its trigger, for example `coder on NEX-42` or `pm`, and it sits under the account of the developer who commissioned it. Every change in the space then shows which role did the work and which person commissioned that role. The design names no particular person: nexus-flow is an open-source tool for any developer or team. How a ticket is written in that name (`NEX-42` or otherwise) follows RFC 0002.

#### Roles in a space

NoCentralDB's roles are reader, writer and admin, per space (NoCentralDB RFC 0008).

| Member | Repository space | Coordination space |
| --- | --- | --- |
| The owner's devices | admin | admin |
| Other developers' devices | writer; admin when the owner makes them one | writer |
| pm session | admin, so it can add and remove the sessions it starts | writer |
| Every other role session (coder, legal, …) | writer | not a member |
| manufakt.io | reader by default; writer when the owner lets it write | reader |

pm is admin of its repository space because it starts sessions, and starting one means adding its member. Other role sessions are writers: they change tickets and decisions but cannot add members. A role session is not a member of the coordination space: pm ranks the work against the goals and turns requests into tickets (GENESIS.md, "Goals set the focus", "Work across repositories"), so a coder never needs it. Whether a coder should read the goals is an open question below.

#### Starting a session's member

No secret travels between sessions. A starting session generates its own key and asks to be added; its commissioner adds the public key.

1. The new session runs `nxs session start --role <role> [--ticket <ref>]`. It generates a key pair in the session's process, keeps the private key in a file only that process's user can read, under the session's temporary directory (see "The local copy on disk"), and prints a join request as JSON: the public key, the proposed member name and the space id. The request holds nothing secret and may travel through any channel: a session message, a ticket comment, a terminal.
2. The commissioner (pm, or a developer's device) runs `nxs member add <join request>`. It adds the public key as a member of the repository space with the role from the table above, under the commissioner's account, and syncs.
3. The new session's next `nxs sync` finds itself a member and pulls the space.

In a cloud container this works the same: the key is generated in the container and dies with it. A cloud environment holds secrets only as proxy-injected headers, which cannot carry a signing key, and a key stored in the environment would be shared by every session started there, which defeats one member per session.

#### Ending a session's member

`nxs session end` removes the session's own member, syncs once, and deletes its key file. A session that dies without ending leaves a member behind; its commissioner removes it with `nxs member remove <member>` when it archives the session, and pm does so for every session it archives. Until NoCentralDB offers members that lapse on their own (requirement R4), a dead session's member stays a member until someone removes it. Its key is gone with the container, so nobody can sign as it.

A removed member's past changes stay signed and attributed: the history of a ticket keeps showing `coder on NEX-42`, under the account that commissioned it, after the session is long gone (requirement R3).

#### Who decided

A change carries the signature of the member who made it, and the member carries its name and account. A ticket's history therefore shows, for every change, the role (`coder on NEX-42`), the person who commissioned it, and whether the owner signed it with one of their own devices. GENESIS.md's "a ticket always shows who decided" is met without a field anyone can set by hand: a decision that only the owner may take is one signed by the owner's device. How each module reads this into its records (for example the "Decided by" of a decision) is designed in its own RFC.

### The model

nexus-flow has one NoCentralDB model for the whole app, written and built the way NoCentralDB RFC 0005 describes, and committed in the repository:

```
model/
  nexus-flow.schema.ts   # the source, one file per module once it grows: tickets.ts, decisions.ts, …
  model.lock.json        # the built model, committed; the Rust code reads only this
```

**One model, both kinds of space.** A NoCentralDB model belongs to an app, not to a space. nexus-flow's model declares every collection of every module; the storage crate refuses to write a collection into a space of the wrong kind (a goal into a repository space, a ticket into a coordination space). That rule is nexus-flow's own, and the module RFCs say which kind of space each collection belongs to.

**Each module owns its collections.** A collection's name starts with its module's name (`tickets`, `decisions`, `status`, `goals`, `requests`), so two module RFCs never declare the same name. The exact form of the prefix follows NoCentralDB's naming rules (NoCentralDB RFC 0005).

**One fixed ticket model** for every repository, generated from the standard `issue-tracker` plugin, with what NEX-24 carries over from Linear (NEX-23). A team's own types and fields are not designed now; a later RFC can add them additively.

**The model only grows.** Schema changes in NoCentralDB are additive (NoCentralDB RFC 0005), and nexus-flow keeps to that: a field or collection, once in a decided RFC and on `v2`, is never renamed or removed. A change that would need one goes through a new collection and a migration, decided by the owner (GENESIS.md: a change to a stored format comes back to the owner).

**Building it.** The schema tooling NoCentralDB ships for RFC 0005 runs in CI at the version the pin names; nexus-flow pins it like any other dependency. A CI step rebuilds `model.lock.json` from the source and fails when it differs from the committed file, and fails when the new lock is not an additive change of the one on the base branch (requirement R8). The Rust build reads only the committed lock (`include_str!`), so building `nxs` needs neither Node nor the schema tooling.

*Not taken:* a model per module (NoCentralDB has one model per app), and a generic field-value collection for team fields (NEX-23 decided one fixed model instead).

### The local copy on disk

What replaces v1's `.nxs/`: the local view of each space lives outside the repository, in nexus-flow's data directory, keyed by space id:

| Platform | Data directory |
| --- | --- |
| macOS | `~/Library/Application Support/nexus-flow/` |
| Linux | `$XDG_DATA_HOME/nexus-flow/`, else `~/.local/share/nexus-flow/` |
| Windows | `%LOCALAPPDATA%\nexus-flow\` |

```
<data dir>/
  spaces/<space id>/     # NoCentralDB's local view of the space (NoCentralDB RFC 0006); its format is NoCentralDB's
  derived/<space id>/    # nexus-flow's rebuildable cache, if any (see "Derived state"); deletable at any time
  sessions/<session id>/ # a running session's key and state; removed by `nxs session end`
```

`NXS_DATA_DIR` overrides the directory, so that tests, cloud containers and several instances on one machine stay apart.

The local copy lives outside the working tree so that a branch switch, a second worktree or a fresh clone of the same repository opens the same copy and loses nothing (GENESIS.md, principle 4). Every worktree of a repository names the same space in `nexus-flow.toml`, and so reads one local view.

Several members on one machine (the developer's device and the sessions it runs) share one local view of a space, each signing its own changes with its own key (requirement R5). Until NoCentralDB offers that, each session keeps its own view under `sessions/<session id>/`, pulled from the relay. That is a stop-gap under NEX-13, behind the storage crate, and it costs disk and a first sync per session; in island mode, where there is no relay to pull from, a session on the device's machine cannot get a view of its own until R5 lands.

*Not taken:* the local view inside the repository's `.nxs/` (lost on a fresh clone, duplicated per worktree, and easy to commit by mistake).

### Reading and writing

`nxs` finds the repository's root from the working directory, reads `nexus-flow.toml`, opens the local view of the repository space (and of the coordination space when it is a member there), and answers from it.

- **No read touches the network.** Every read runs against the local view, through an index the model declares (every query in NoCentralDB runs through one). A read that would need data the local view does not hold answers from what it holds and says so in its output; it never fetches.
- **A write commits locally and returns.** It is signed by the member the command runs as and is visible to every later local read at once. Sending it to the relay is sync's job, below.
- **The member a command runs as** is the session's member when `NXS_SESSION` names a running session, otherwise the device's member.
- **Output stays deterministic.** JSON by default, Markdown with `--md`, with the same ordering and field rules as v1 (the decisions on golden JSON determinism stay in force). The JSON fields of each module are designed in its RFC.

### Sync

v2 retires v1's own sync for NoCentralDB's relay (NEX-17). The relay cannot read content (NoCentralDB RFC 0007). manufakt.io hosts a relay; a team may run its own; a space with no relay works in island mode.

- **`nxs sync`** pulls from and pushes to the relay once, for every space the repository names, and exits. `nxs sync --watch` keeps doing so until stopped, for a framework that runs a background task or a developer who keeps a terminal open.
- **After a write**, `nxs` starts a detached `nxs sync` for that space and returns without waiting for it, so a change reaches the relay within seconds without a daemon and without the network on the command's path.
- **The relay** is named per space in the local configuration that `nxs sync --relay <url>` writes into the data directory, not in `nexus-flow.toml`: which relay a machine syncs through is the machine's choice, and a URL may carry a host a team keeps private. NoCentralDB RFC 0007 says how a relay is reached and authorised.
- **Island mode.** With no relay, a space lives on one machine, as v1's board does today. Sessions on that machine share it once NoCentralDB offers R5, and until then only the device's member works in it; a cloud session cannot reach it, and a coordination space is useful only with a relay.
- **Freshness.** `nxs sync --status` reports, per space, when it last pulled and pushed, from the local view and without the network. Reads do not report it.

**What happens to v1's sync.** `crates/sync`, `crates/service` (the background sync service) and the `nxf-relay` in `crates/server` are not part of v2. They keep shipping from `main` as long as v1 does; on `v2` they are removed once RFC 0008's migration no longer needs them, and that removal is its own ticket.

*Not taken:* syncing at the start of every command (puts the network on the read path), and a background sync service as in v1 (one more process to install, supervise and update on every platform, which NEX-17 retired).

### Derived state

Readiness, blocked and "on main" are never stored (GENESIS.md, "Done is not yet on main"). NoCentralDB reserves computed fields and views (`nc.reach`, `nc.view`) but has not designed them yet (requirement R7), so nexus-flow computes them itself, behind the seam.

- **Readiness and blocked** are computed by the storage crate from the dependency links in the local view of the repository space: it reads the tickets and their links through the model's indexes and walks the graph in memory. The rules themselves (what counts as done, as blocked) are RFC 0002's.
- **"On main"** is computed from the repository's local git: a ticket is on main when a commit reachable from the branch `nexus-flow.toml` names (`[git] main`) names the ticket, squash merges included. It reads the local git and never fetches, so it is as fresh as the last `git fetch`. A caller with no checkout (manufakt.io) gets `null`, meaning not known, instead of a guess.
- **A cache** under `derived/<space id>/` may hold what the computation found, keyed by the local view's version and the git commit it read. It is rebuilt whenever either moves, never synced, and deleting it changes no answer.

This is a **stop-gap** under NEX-13: it is temporary, it lives only behind the storage crate's functions that return derived state, and once NoCentralDB's computed fields can express reach (R7), those functions read them instead and the in-memory walk goes.

### The library seam

v2 keeps one library seam, designed for manufakt.io, the owner's view onto nexus-flow (NEX-18). nexflow.it is not a v2 consumer: it stays on v1 from `main` until it decides its own move, and nothing in v2 serves its proprietary plugin.

The seam is the crate `nexus-flow-facade` in a new major version, with `Engine` as its entry point, as in v1. It is the only crate an embedder links. `nxs` is built on the same seam, and every `nxs` verb has a counterpart on it (the decision on CLI and library parity stays in force). The seam this RFC brings:

| Symbol | What it does |
| --- | --- |
| `Engine::open(repository: &Path)` | Opens the spaces `nexus-flow.toml` names, as the device's member |
| `Engine::open_space(space: SpaceId, git: Option<&Path>)` | Opens a space by id, for a caller without a checkout (manufakt.io); "on main" is then `null` |
| `Engine::as_session(session: SessionId)` | The same engine, writing as a running session's member |
| `Engine::session_start`, `Engine::session_end` | Start and end a session's member, as `nxs session start` and `end` |
| `Engine::member_add`, `Engine::member_remove`, `Engine::members` | Add, remove and list the members of a space |
| `Engine::sync`, `Engine::sync_status` | One sync, and the last pull and push per space |
| `Engine::subscribe` | A stream of changes to the local view, for a view that redraws |

Each module RFC adds its own reads and writes to `Engine` (tickets, decisions, goals, requests, status notes), with derived state computed inside, so manufakt.io never re-implements a rule.

What becomes of v1's crates on `v2`:

| Crate | In v2 |
| --- | --- |
| `foundation` | Retired. Its workspace, schema, signing and trust layers are NoCentralDB's job now. Repository discovery and `nexus-flow.toml` move into the new storage crate. |
| `core` | Retired. The op-log CRDT is replaced by NoCentralDB. RFC 0008's migration reads v1 data through it, and it is removed once the migration no longer needs it. |
| `facade` | Rewritten as the seam above, in a new major version. v1's facade keeps shipping from `main` for nexflow.it. |
| `sync`, `service`, `server` | Retired (see "Sync"). |
| new: `store` | The only crate that depends on the NoCentralDB SDK: the model, opening spaces, members, sync, derived state. Not public; the facade re-exports what an embedder needs. |

*Not taken:* embedders reading NoCentralDB directly (every embedder re-implements the derived state, and a model change breaks them), and the CLI as the only interface (a process per call, no types); both were options in NEX-18.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Commands:** `nxs init` (creates the repository space and `nexus-flow.toml`), `nxs device join`, `nxs space join-coordination <space id>`, `nxs session start --role <role> [--ticket <ref>]`, `nxs session end`, `nxs member add <join request>`, `nxs member remove <member>`, `nxs member list`, `nxs sync [--watch] [--status] [--relay <url>]`.
- **JSON output:** the join request (`space`, `public_key`, `name`, `role`, `format`), `nxs member list` (`member`, `name`, `role`, `account`, `kind`: `device`, `session` or `app`), `nxs sync --status` (`space`, `pulled_at`, `pushed_at`, `relay`).
- **Environment variables:** `NXS_SESSION`, `NXS_DATA_DIR`.
- **Library seam:** the `Engine` symbols in the table above, and `SpaceId`, `SessionId`, `Member` as the types they take and return.
- **Stored formats:** `nexus-flow.toml` (`format = 1`), `model/model.lock.json` as nexus-flow's model, the data directory's layout, and member names of the form `<role> on <ticket>`.

### What nexus-flow needs from NoCentralDB

Requirements this design rests on. Where NoCentralDB already offers one, the module that uses it cites the NoCentralDB RFC; where it does not, pm files it as a request on NoCentralDB's tracker.

- **R1. A space id grants nothing.** Knowing a space's id, without a member key, reads nothing, so the id can be committed.
- **R2. Join by public key.** An admin adds a member from its public key and a proposed name, so that no secret travels between the new member and its commissioner.
- **R3. Removed members stay attributed.** A removed member's past changes stay verifiable and keep its name and account for every reader of the space.
- **R4. Members that lapse.** A member can be added with an expiry or a lease, so a session that dies without ending stops being a member on its own.
- **R5. One local view, several members.** Several members on one machine share one local view of a space, each signing with its own key.
- **R6. A first sync for a new member.** A new member with an empty local view (a cloud container) pulls a whole space from the relay in one sync.
- **R7. Computed reach.** Computed fields or views (`nc.reach`) that express "every ticket this one depends on is done", so readiness need not be computed outside NoCentralDB.
- **R8. An additive check.** The schema tooling can say whether one `model.lock.json` is an additive change of another, so CI can enforce it.
- **R9. Change notification.** The local view tells an open handle when it changed, from a local write or a sync, for `Engine::subscribe`.
- **R10. A git-pinnable SDK.** The Rust SDK builds from a git revision without network access beyond cargo's, and without the schema tooling when a `model.lock.json` is given.

## What it does not do

- **It does not protect one module from another.** Rights apply to a whole space: a writer of the repository space can change its tickets, its decisions and its status notes alike. A decision record is protected from being edited away by RFC 0003's design and by the signatures, not by a right.
- **It cannot add a member offline.** A member added on a machine that is offline becomes usable only once that change reaches the relay and the new member has synced. Until then the new session can do nothing in the space.
- **A lost session key loses that session's unsynced writes.** The key and the session's own local view die with the container. Writes it made but did not push are gone; that is why every write starts a sync at once.
- **A lost device key needs another admin.** If the only admin device of a space is lost, nobody can add or remove members. Recovery is NoCentralDB's account recovery (NoCentralDB RFC 0010), where it exists. `nxs init` says so and recommends a second admin device.
- **A dead session's member stays until removed.** Until R4 exists, a session that dies without `nxs session end` remains a member until its commissioner removes it. Nobody can sign as it, because its key died with it.
- **A relay can withhold or delay changes.** The relay cannot read or forge content, but it can hold changes back. A session can then act on an old state, for example take a ticket that another session already took. `nxs sync --status` shows how old the local view is; nothing detects a relay that withholds on purpose.
- **Island mode reaches no one else.** Without a relay, a space lives on one machine: cloud sessions cannot use it, and a coordination space has no point. Until R5 lands, agent sessions on that machine cannot use it either, because each would need a local view of its own pulled from a relay.
- **"On main" is as fresh as the last `git fetch`,** and is not known (`null`) to a caller without a checkout.
- **Derived state is computed in memory** until R7 lands, so it costs time in proportion to the space's tickets and links, on every read that needs it, unless the cache is fresh.
- **A ticket cannot link to a goal or a request.** They live in another space; a ticket names them by id, and that reference can dangle.
- **A repository belongs to at most one coordination space** in v2.

## Migration

This RFC moves no data. A v1 workspace (`.nxs/`) is left as it is: v2 neither opens nor changes it, and v1 keeps working from `main`. RFC 0008 designs the move of a v1 workspace and its data into the spaces described here, including workspaces that v1 syncs across several devices; until it is decided, `nxs init` on v2 creates an empty repository space beside an existing `.nxs/` and says that the v1 data has not been moved.

## Open questions

Each of these is a public surface or a stored format, which GENESIS.md reserves for the owner.

1. **Where the space id is committed.** Options: (a) a file `nexus-flow.toml` at the repository's root, as designed above; (b) a file inside a committed directory, such as `.nxs/space.toml`, next to where v1 kept its workspace; (c) not committed, told to every session. Costs: (a) adds a file at the root; (b) mixes a committed file into a directory v1 ignores in git, which RFC 0008 would have to untangle; (c) every clone and every cloud container needs the id by hand. Recommendation: (a).
2. **Where the local copy lives.** Options: (a) the platform's data directory, outside the repository, as designed above; (b) inside the repository's `.nxs/`, as in v1. Costs: (a) a workspace is no longer visible in the repository, and deleting the repository leaves the local copy behind; (b) a fresh clone or a second worktree starts empty and syncs again, and the branch-switch guarantee depends on `.nxs/` being ignored by git. Recommendation: (a).
3. **The new commands' names.** `nxs session start|end`, `nxs member add|remove|list`, `nxs device join`, `nxs space join-coordination`, `nxs sync [--watch|--status|--relay]`. Options: these names; or the same verbs under one noun (`nxs space member add`, `nxs space session start`). Costs: the nested form is longer to type and to read in a prompt; the flat form adds three top-level nouns. Recommendation: the flat names above.
4. **How a session is admitted.** Options: (a) the join request with a public key, added by the commissioner, as designed above; (b) the commissioner mints a single-use invitation and hands it to the session. Costs: (a) needs a channel back from the new session to its commissioner, which a Claude Code cloud session does not reliably have today (GENESIS.md, "Background"), so pm may have to read the request from the ticket; (b) the invitation is a secret, and it would travel through a session message or a ticket, which GENESIS.md forbids for secrets. Recommendation: (a).
5. **Whether role sessions read the coordination space.** Options: (a) not members, as designed above; (b) readers, so a coder sees the goals its ticket serves. Costs: (a) a coder sees a goal only as the id its ticket names; (b) a member to add and remove per session in a second space. Recommendation: (a), revisited by RFC 0005 if a goal's text turns out to matter to the work.
6. **The seam's crate name.** Options: (a) keep `nexus-flow-facade` with `Engine`, in a new major version; (b) a new crate name for v2. Costs: (a) the same name means two incompatible lines, v1's on `main` for nexflow.it and v2's here; (b) manufakt.io changes its dependency's name once. Recommendation: (a), since nexflow.it pins v1 and the major version says the rest.

## History

- 2026-11-15: published as a draft in #<pull request>.
