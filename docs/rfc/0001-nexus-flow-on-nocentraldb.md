# RFC 0001: nexus-flow on NoCentralDB

Status: Decided (2026-11-17)

This RFC sets the frame every NoCentralDB-backed module of v2 stores its data in: the NoCentralDB spaces nexus-flow uses, who is a member of them and with which role, how nexus-flow's NoCentralDB model is written and grown, where each member's local database lives and how `nxs` reads and writes it, how it syncs, how derived state such as readiness is computed, and which library seam manufakt.io calls. It follows `factory/GENESIS.md`, "Architecture and stack": tickets, decisions, goals and status notes live in NoCentralDB, "a ticket space per repository, and a coordination space per group of repositories", every read is local, and `nxs` is the only entry point. Once it is decided, the module RFCs (0002 to 0007) design their collections inside this frame, RFC 0008 designs the move from v1 into it, and a coding ticket can build the shared layer: the pin, the model build, opening a member's database, members, sync and the seam.

## What it solves

v1 keeps its state in a `.nxs/` workspace of its own: an op-log CRDT on SQLite (`crates/core`), its own signing keys and trust list (`crates/foundation`), its own sync protocol, background service and relay (`crates/sync`, `crates/service`, `crates/server`). v2 moves all of that onto NoCentralDB, so that nexus-flow keeps only what is its own: the modules' rules, the derived state and the `nxs` surface (GENESIS.md, "What we build"). The move is priority 1 (decision of 2026-11-15, "Priorities: migrate to NoCentralDB, then build the new factory, then the rest").

The owner and the agents need from this frame:

- **One place per repository** where a session finds that repository's tickets, decisions and status notes, and one shared place per group of repositories for goals and requests (GENESIS.md, "Work across repositories").
- **A signature on every change**, and a record of which role made it and which person commissioned that role (GENESIS.md, "What has to come back through me": "a ticket always shows who decided").
- **Reads that never wait for the network** (GENESIS.md, principle 3, and "Limits"), with deterministic JSON output.
- **Nothing lost on a branch switch or when a session ends** (principle 4): the local data outlives both.
- **One rule per derived state**, computed the same way by `nxs` and by manufakt.io (GENESIS.md, "Done is not yet on main").

The owner decided the frame's main choices before this RFC was written, and the text below carries them: the dependency and stop-gaps (NEX-13), the space layout (NEX-15), who a member is (NEX-16), the sync (NEX-17), the library seam (NEX-18), the ticket model (NEX-23), and "on main" against a configurable base branch (NEX-33). The owner decided the choices this RFC left open on 2026-11-17 (NEX-38): where the space id is committed, where the member databases live, the command names, how a session's invitation reaches its commissioner, that role sessions stay out of the coordination space, and the seam's crate name. On the same day the owner decided how a session is tied to the person who commissioned it: pm is an agent paired into that person's account, and every session pm starts is pm's sub-agent.

Out of scope, each in its own RFC: the collections and fields of each module (tickets 0002, decisions 0003, goals 0005, status 0007), human-readable ticket ids and what the ticket model is generated from (0002, from NEX-16, NEX-23 and NEX-24), and the move of v1 workspaces and their data (0008). GENESIS and the factory's files stay in git (NEX-14, NEX-33), so this RFC gives them no space; RFC 0004 and RFC 0006 design those modules.

NoCentralDB's own design is in nocentraldb/nocentraldb, `docs/rfc/`, all decided: 0003 keys and certificates, 0004 references, 0005 the schema and `model.lock.json`, 0006 the local view, 0007 the relay, 0008 membership and roles, 0009 the SDK, 0010 accounts. This RFC cites them as "NoCentralDB RFC", by number and section, and does not restate them; an RFC number without that prefix is one of nexus-flow's own. Where nexus-flow needs something NoCentralDB does not offer, it is a request on NoCentralDB under "What nexus-flow needs from NoCentralDB", not a workaround here.

## The design

### The dependency on NoCentralDB

nexus-flow depends on NoCentralDB's Rust SDK (`crates/nocentraldb` in nocentraldb/nocentraldb, NoCentralDB RFC 0009) through a pinned git revision in the workspace `Cargo.toml`, the only way in, since the crate is not published (requirement R10): `rev = "<full commit hash>"`, never a branch or a tag. The pin moves deliberately, one pull request per move, and that pull request carries nothing else. Because `Cargo.toml` and `Cargo.lock` are code-owned, every move waits for the owner (decided in NEX-13).

Only one crate of nexus-flow depends on the SDK: the storage crate under the seam (see "The library seam"). Every other crate reaches NoCentralDB through it, so that a change in NoCentralDB's API at a pin move touches one crate.

When nexus-flow needs something NoCentralDB lacks, pm files it as a request on NoCentralDB's tracker. nexus-flow may build a stop-gap meanwhile, only behind its own seam, named as temporary in the RFC that brings it, with a ticket to remove it (NEX-13). This RFC brings two stop-gaps: the derived state (see "Derived state") and the author field a record carries until NoCentralDB can say who signed a change (see "Who decided"). When the RFC is decided, pm opens a ticket to remove each, blocked by the NoCentralDB request it waits for.

*Not taken:* waiting for a crates.io release (priority 1 would stall on another repository's date) and vendoring a copy (two copies drift).

### Spaces

nexus-flow uses two kinds of NoCentralDB space. Rights apply to a whole space, and a link points only to an object in the same space, so the layout follows what must link to what.

| Kind | One per | Holds | Designed in |
| --- | --- | --- | --- |
| Repository space | Repository | The repository's projects and tickets, decisions, and status notes | RFC 0002, 0003, 0007 |
| Coordination space | Group of repositories | The group's goals, and the requests between its repositories | RFC 0005, and the RFC on work across repositories |

**One repository space per repository**, not one per module (NEX-15). Decisions and status notes point at tickets all the time, and only one space lets them use real links. The cost is that a member who may read tickets may also read decisions. When a team needs finer rights, a small extra space can be added later, the way NoCentralDB's sub-agent spaces work; this RFC designs none.

**A coordination space** joins a group of repositories. A repository belongs to at most one coordination space in v2. An object id is unique only within its space (NoCentralDB RFC 0009), so a record in a repository space refers to a goal or a request through an `nc.ref`, which carries the space id and the object id (NoCentralDB RFC 0005, RFC 0004). NoCentralDB defines the states of a reference it cannot resolve, for example when the reader is not a member of the other space (NoCentralDB RFC 0004); the module RFCs say how each is shown.

#### Creating and finding a space

`nxs init` in a repository creates the repository space when the repository has none, with the developer's device as its first admin (NoCentralDB RFC 0008), and writes the space's id into a committed file at the repository's root:

```toml
# nexus-flow.toml, committed. Holds no secret: a space id grants nothing.
format = 1

[space]
repository = "<NoCentralDB space id>"
coordination = "<NoCentralDB space id>"   # optional

[git]
base = "main"                             # the team's base branch, which "on main" is checked against
```

A session in a fresh clone or a cloud container finds its space by reading this file; it does not ask anyone. The file is committed so that every clone, worktree and branch names the same space. A space id is not a secret: holding it gives no access without a member key (requirement R1, met).

A relay creates a space only for an allowlisted creator (NoCentralDB RFC 0007). `nxs init` against a relay that does not allow the developer therefore creates the space locally and reports that the relay refused it; on manufakt.io's relay, the developer's account must be allowlisted first. Without a relay, the space is created locally and stays in island mode (see "Sync").

`nxs space join-coordination <space id>` records a coordination space in the file once an admin of that space has added the repository's pm (see "Members"). `nxs init` never overwrites a `nexus-flow.toml` that already names a space (GENESIS.md: never overwrite a team's configuration).

*Not taken* (decided by the owner, NEX-38): a committed file inside `.nxs/`, next to where v1 kept its workspace (it mixes a committed file into a directory v1 ignores in git, which RFC 0008 would have to untangle); keeping the space id only in the local data directory (every clone and every cloud container would have to be told it by hand); and deriving the id from the git remote URL (a renamed or forked repository would silently get a new space).

### Members

A NoCentralDB member is a device, an agent or a sub-agent, each with its own keys (NoCentralDB `MemberKind`: `device`, `agent`, `subAgent`; NoCentralDB RFC 0003, RFC 0009). An account is a key pair that signs the certificates of its devices and agents; it is not a login, and no server holds it (NoCentralDB RFC 0010, "Accounts"). A device or an agent joins an account by pairing, which runs on a device that already belongs to the account; the agent receives a certificate, never the account key. A sub-agent holds no account: its parent agent signs its certificate when it adds it, so the chain runs sub-agent, parent agent, account (NoCentralDB RFC 0010, rule 4; RFC 0009, "Sub-agents and ad-hoc spaces"). nexus-flow uses four kinds of member:

| Member | NoCentralDB kind | Created | Keys live | Removed |
| --- | --- | --- | --- | --- |
| A developer's device | `device` | When the developer first runs `nxs init` or `nxs device join` on it, paired into the developer's account (NoCentralDB RFC 0010) | In the device's key store (NoCentralDB RFC 0003, RFC 0009) | By the developer, when the device is retired or lost |
| pm | `agent` | When the pm session starts, paired once into the account of the developer who commissions it | In pm's own member database, created in the session (see "The local data on disk"); never written to the repository, the environment, a ticket or a log | When pm ends, or by the developer when a new pm takes over |
| A role session that pm starts (coder, legal, …) | `subAgent` of pm | When the session starts, added by pm | In the session's own member database, as for pm | When the session ends, or by pm when it archives the session |
| manufakt.io | `device` of the person's account | When the person pairs manufakt.io into their account and adds it to their spaces | Where manufakt.io keeps its device keys | By the person |

**One member per agent session** (NEX-16). Its role is the role on its trigger, for example `coder on NEX-42` or `pm`, and the chain of certificates says which person commissioned it: a coder is pm's sub-agent, and pm is an agent of the developer's account, so every change shows the chain person, pm, coder (decided by the owner on 2026-11-17). The person acts once per pm, when pm is paired, and never per session. The design names no particular person: nexus-flow is an open-source tool for any developer or team. How a ticket is written in that role (`NEX-42` or otherwise) follows RFC 0002.

**manufakt.io** joins as a device of the person's account, not as an agent: it is the person's own view onto their work and acts for them. How the person's account and their manufakt.io sign-in become one account is described under "Sync", with manufakt.io's relay.

#### The role record

NoCentralDB has no member names: an invitation carries a member id, its public keys and, for a device or an agent, a certificate and an account fingerprint, nothing else (NoCentralDB RFC 0003, RFC 0009). The role therefore lives in nexus-flow's own model. Each space nexus-flow uses holds a collection `spaceMember`, owned by this RFC, with one record per member nexus-flow added:

| Field | Holds |
| --- | --- |
| `member` | The NoCentralDB member id |
| `role` | `owner`, `developer`, `pm`, `app`, or the role of a session (`coder`, `legal`, …) |
| `ticket` | For a session: an `nc.ref` to the ticket it was started on, if any |

The member that adds another writes its record in the same step. Who commissioned a member is not part of the record: it is the member's certificate chain, which NoCentralDB signs and checks.

#### Roles in a space

NoCentralDB's roles are reader, writer and admin, per space (NoCentralDB RFC 0008).

| Member | Repository space | Coordination space |
| --- | --- | --- |
| The owner's devices | admin | admin |
| Other developers' devices | writer; admin when the owner makes them one | writer |
| pm | admin, so it can add and remove the sessions it starts | writer |
| Every other role session (coder, legal, …) | writer | not a member |
| manufakt.io | writer, so the person can read and act through it | writer |

pm is admin of its repository space because it starts sessions, and starting one means adding its member. Other role sessions are writers: they change tickets and decisions but cannot add members. manufakt.io reads and writes in both kinds of space, as the person would from any of their devices. A role session is not a member of the coordination space: pm ranks the work against the goals and turns requests into tickets (GENESIS.md, "Goals set the focus", "Work across repositories"), so a coder never needs it; it sees a goal only as the unresolved `nc.ref` its ticket holds (decided by the owner, NEX-38). Making role sessions readers would add a second membership to add and remove per session. A later version may give a coder the access to the coordination space that pm names (NEX-40); since a coder is already pm's sub-agent, pm can add it there itself. RFC 0005 can revisit this.

#### Starting pm

pm is paired once into the account of the developer who commissions it (NoCentralDB RFC 0010, "Accounts"):

1. The pm session runs `nxs session start --role pm --pairing`. It opens its member database for pairing from its first open (`OpenOptions::pairing()`; NoCentralDB RFC 0009) and prints a pairing request. The request holds nothing secret and may travel through any channel the developer trusts.
2. On one of their devices, the developer runs `nxs member pair <request>`, which pairs pm into their account as an agent (`account.pair` with kind `agent`) and prints the response.
3. pm runs `nxs session complete-pairing <response>` and holds a certificate from the developer's account, never the account key.
4. The developer's device adds pm to the repository space as admin and to the coordination space as writer, with `nxs member add <invitation> --role pm`.

A new pm, when the developer replaces one, is paired the same way.

#### Starting a role session

No secret travels between sessions. A starting session creates its own keys and hands pm an invitation; pm adds it.

1. The new session runs `nxs session start`. It opens its member database as a sub-agent (`OpenOptions::sub_agent()`; NoCentralDB RFC 0009, "Sub-agents and ad-hoc spaces") and prints its invitation (`ncdb-invite-…`): its member id and public keys, without a certificate. The invitation holds nothing secret. It reaches pm as a session message through the agent framework, and where the framework cannot send one, as a comment on the session's ticket (decided by the owner, NEX-38). The fallback is needed today: a Claude Code cloud session cannot reliably message back ([how sessions reach each other](../research/2026-11-14-how-sessions-reach-each-other.md)), and the session cannot write to the repository space before it is a member, so the comment goes through the tracker in use until v2's own tickets carry it.
2. pm runs `nxs member add <invitation> --role <role> [--ticket <ref>]`. pm's `members.add` signs the sub-agent's certificate with pm's member key, adds it to the repository space as writer, writes its `spaceMember` record, and syncs. A sub-agent creates no space of its own; pm adds it to the spaces it works in.
3. The new session's next sync finds itself a member and pulls the space.

In a cloud container this works the same: the keys are created in the container and die with it. A cloud environment holds secrets only as proxy-injected headers, which cannot carry a signing key, and a key stored in the environment would be shared by every session started there, which defeats one member per session.

*Not taken:* a session as an agent of its own unconfirmed account, tied to the person only by a `spaceMember` record its commissioner signs (answer 7a in NEX-38, which the owner replaced on 2026-11-17 once NoCentralDB had built pairing and sub-agents); and pairing every session into the person's account, which would ask the person to act for every session.

#### Ending a session's member

`nxs session end` removes the session's own member, syncs (see "Sync"), and deletes its member database. A session that dies without ending leaves a member behind; pm removes it with `nxs member remove <member>` when it archives the session, and does so for every session it archives. Until NoCentralDB offers members that lapse on their own (requirement R5), a dead session's member stays a member until someone removes it. Its keys are gone with the container, so nobody can sign as it.

A removed member's past changes stay valid: NoCentralDB checks a change against the member's rights at the time it was made, up to the removal's cutoff (NoCentralDB RFC 0008, "The cutoff").

#### Who decided

Every change is signed by the member who made it, and every member's certificate chain leads to a person's account. A ticket's history should show, for every change, the role (`coder on NEX-42`), the person who commissioned it, and whether the owner signed it with one of their own devices; GENESIS.md's "a ticket always shows who decided" is then met without a field anyone can set by hand.

NoCentralDB checks every signature but offers no way yet to read who signed a given change or an object's history; `members.list()` returns the current members only (NoCentralDB RFC 0009). Reading a change's author is therefore a request (requirement R6). Until it lands, each module's records carry the member id of the member that wrote them, written by that member, which NoCentralDB does not check against the signature. That field is the second stop-gap of this RFC: it is temporary, and once R6 lands the modules read the signer instead and the field goes. How each module reads this into its records (for example the "Decided by" of a decision) is designed in its own RFC.

### The model

nexus-flow has one NoCentralDB model for the whole app, written and built the way NoCentralDB RFC 0005 describes, and committed in the repository:

```
model/
  model.ts           # the source, which `ncdb` reads by default; split into one file per module as it grows
  model.lock.json    # the built model, committed; the Rust code reads only this
```

**One model, both kinds of space.** A NoCentralDB model belongs to an app, not to a space. nexus-flow's model declares every collection of every module; the storage crate refuses to write a collection into a space of the wrong kind (a goal into a repository space, a ticket into a coordination space). That rule is nexus-flow's own, and the module RFCs say which kind of space each collection belongs to.

**Each module owns its collections.** A collection's name must match `^[a-z][A-Za-z0-9]{0,62}$` (NoCentralDB RFC 0005), so it has no separator. A collection's name is its module's name in lower case followed by the collection's own name in upper camel case: `ticketsTicket`, `decisionsDecision`, `goalsGoal`, `requestsRequest`, `statusNote`. The frame's own collections use the prefix `space`, as `spaceMember` does. Two module RFCs therefore never declare the same name.

**One fixed ticket model** for every repository (NEX-23). What it is generated from, and what NEX-24 carries over from Linear, is RFC 0002's. A team's own types and fields are not designed now; a later RFC can add them additively.

**The model only grows.** Schema changes in NoCentralDB are additive (NoCentralDB RFC 0005), and nexus-flow keeps to that: a field or collection, once in a decided RFC and on `v2`, is never renamed or removed. A change that would need one goes through a new collection and a migration, decided by the owner (GENESIS.md: a change to a stored format comes back to the owner).

**Building it.** NoCentralDB's `ncdb` tool builds the model and checks that a new `model.lock.json` is an additive change of an old one (`ncdb model check`; requirement R4, met). It runs in CI at the version the pin names, which needs Node 24 in the CI job: a change to `.github/workflows/`, which is code-owned and waits for the owner. A CI step rebuilds `model.lock.json` from the source and fails when it differs from the committed file, and runs `ncdb model check` against the lock on the base branch. The Rust build reads only the committed lock (`include_str!`), so building `nxs` needs neither Node nor `ncdb`.

*Not taken:* a model per module (NoCentralDB has one model per app), and a generic field-value collection for team fields (NEX-23 decided one fixed model instead).

### The local data on disk

What replaces v1's `.nxs/`: each member nexus-flow runs as has one NoCentralDB database, which holds that member's view of every space it belongs to. That is NoCentralDB's design: each member on a device has one SQLite database, and a device with two members has two (NoCentralDB RFC 0006, "Where the view lives"); one SDK database is opened with one member's keys (NoCentralDB RFC 0009). The databases live outside the repository, in nexus-flow's data directory:

| Platform | Data directory |
| --- | --- |
| macOS | `~/Library/Application Support/nexus-flow/` |
| Linux | `$XDG_DATA_HOME/nexus-flow/`, else `~/.local/share/nexus-flow/` |
| Windows | `%LOCALAPPDATA%\nexus-flow\` |

```
<data dir>/
  device/                 # the device member's NoCentralDB database, with all its spaces (NoCentralDB RFC 0006); its format is NoCentralDB's
  sessions/<session id>/  # one session member's database and keys; created by `nxs session start`, removed by `nxs session end`
  derived/<member id>/    # nexus-flow's rebuildable cache, if any (see "Derived state"); deletable at any time
```

`NXS_DATA_DIR` overrides the directory, so that tests, cloud containers and several instances on one machine stay apart.

The data lives outside the working tree so that a branch switch, a second worktree or a fresh clone of the same repository opens the same database and loses nothing (GENESIS.md, principle 4). Every worktree of a repository names the same space in `nexus-flow.toml`, and the device member reads it from its one database.

Each session on a machine has a database of its own, which syncs like any other member's. Several sessions on one device therefore hold several copies of the same space and each pulls it once; that is the cost of one member per session.

*Not taken* (decided by the owner, NEX-38): the data inside the repository's `.nxs/`, as in v1 (lost on a fresh clone, duplicated per worktree, and easy to commit by mistake). The cost of the data directory is that a workspace is no longer visible in the repository, and deleting the repository leaves the databases behind. Also not taken: one shared database for every member on a device (NoCentralDB opens a database with one member's keys).

### Reading and writing

`nxs` finds the repository's root from the working directory, reads `nexus-flow.toml`, opens the database of the member it runs as, and answers from the spaces that file names.

- **No read touches the network.** Every read runs against the local database, through an index the model declares (every query in NoCentralDB runs through one). A read that would need data the database does not hold answers from what it holds; it never fetches.
- **A write commits locally.** It is signed by the member the command runs as and is visible to every later local read at once. Reaching the relay is sync's job, below.
- **The member a command runs as** is the session's member when `NXS_SESSION` names a running session, otherwise the device's member.
- **Output stays deterministic.** JSON by default, Markdown with `--md`, with the same ordering and field rules as v1 (the decisions on golden JSON determinism stay in force). The JSON fields of each module are designed in its RFC.

### Sync

v2 retires v1's own sync for NoCentralDB's relay (NEX-17). The relay cannot read content (NoCentralDB RFC 0007). A team chooses its relay:

- **One of NoCentralDB's example relays**, as NoCentralDB ships them.
- **A relay the team runs itself.** nexus-flow ships no relay software of its own, since `nxf-relay` retires; its documentation says how to run NoCentralDB's relay for a team.
- **manufakt.io's hosted relay.**

A member with no relay works in island mode.

**Using manufakt.io's relay.** The person signs in to manufakt.io, and their local NoCentralDB account and their manufakt.io account become one: in NoCentralDB terms the two accounts are linked (NoCentralDB RFC 0010, "Linking accounts drawn per database"). The local database pairs with a device of the manufakt.io account, both account keys sign the link, and the memberships already written keep their certificates. To the person it is one account, and nexus-flow asks nothing of them beyond signing in. NoCentralDB cannot link an agent that holds an account of its own (NoCentralDB RFC 0010), but no session here does: pm is paired into the person's account, and every other session is pm's sub-agent. How manufakt.io hosts the relay and runs its sign-in is manufakt.io's side, and nexus-flow needs nothing from it beyond the library seam.

**One relay per member database.** The SDK connects one database to one relay, and a relay per space was not taken (NoCentralDB RFC 0009, "The relay connection"). A member's repository space and coordination space therefore sync through the same relay, and the repositories of one coordination space must all use it. `nxs sync --relay <url>` sets the relay of the member it runs as, in that member's own configuration in the data directory, not in `nexus-flow.toml`: which relay a machine syncs through is the machine's choice, and a URL may carry a host a team keeps private.

**Sync runs while a database is open.** The SDK has no one-shot sync call: it syncs while the database is open, and offers pause and resume (NoCentralDB RFC 0009). nexus-flow uses that as follows:

- **A command** opens the database, does its work and closes it without waiting. A read never waits for sync; a write that the command closes before it is confirmed stays queued in the database and goes out the next time the member's database is open.
- **`nxs sync`** opens the database, waits until the initial sync is complete and nothing is behind or unconfirmed, then closes it. `--timeout <seconds>` bounds the wait; on timeout it reports what is still behind and exits non-zero. `nxs sync --watch` keeps the database open, and so syncing, until stopped, for a framework that runs a background task or a developer who keeps a terminal open.
- **`nxs session end`** runs the same wait before it removes the member, so a session's last writes leave before its database is deleted.
- **`nxs sync --status`** reports, from the local database and without the network, whether anything is unconfirmed and when the member last synced.

**Island mode.** With no relay, a member's database lives on its own: it keeps working, and its copies converge when they reconnect (GENESIS.md, "Limits"). NoCentralDB designs peer sync between members (NoCentralDB RFC 0007, "Island mode"), but how two members find each other without a relay is left to a later NoCentralDB RFC (NoCentralDB RFC 0007, "What this RFC does not cover"). Until then, island mode serves one member per space in practice: the device's. Once members can find each other (requirement R11), several repositories on one device, each with its agents working in parallel, sync peer to peer without a relay, the coordination space they share included. A cloud session, on a machine of its own, needs a relay either way.

**What happens to v1's sync.** `crates/sync`, `crates/service` (the background sync service) and the `nxf-relay` in `crates/server` are not part of v2. They keep shipping from `main` as long as v1 does; on `v2` they are removed once RFC 0008's migration no longer needs them, and that removal is its own ticket.

*Not taken:* syncing at the start of every read (puts the network on the read path), and a background sync service as in v1 (one more process to install, supervise and update on every platform, which NEX-17 retired).

### Derived state

Readiness, blocked and "on main" are never stored (GENESIS.md, "Done is not yet on main"). NoCentralDB reserves computed fields and views (`nc.reach`, `nc.view`) but has not designed them yet (requirement R7), so nexus-flow computes them itself, behind the seam.

- **Readiness and blocked** are computed by the storage crate from the dependency links in the repository space: it reads the tickets and their links through the model's indexes and walks the graph in memory. The rules themselves (what counts as done, as blocked) are RFC 0002's.
- **"On main"** is computed from the repository's local git: a ticket is on main when a commit reachable from the base branch `nexus-flow.toml` names (`[git] base`, `main` unless the team configures another) names the ticket in its title, squash merges included. A title may name tickets as a range: `NEX-13..NEX-23` names every ticket from NEX-13 to NEX-23 (decided in NEX-33). It reads the local git and never fetches, so it is as fresh as the last `git fetch` of the base branch. A caller with no checkout (manufakt.io) gets `null`, meaning not known, instead of a guess.
- **A cache** under `derived/<member id>/` may hold what the computation found, keyed by the database's version and the git commit it read. It is rebuilt whenever either moves, never synced, and deleting it changes no answer.

This is a **stop-gap** under NEX-13: it is temporary, it lives only behind the storage crate's functions that return derived state, and once NoCentralDB's computed fields can express reach (R7), those functions read them instead and the in-memory walk goes.

### The library seam

v2 keeps one library seam, designed for manufakt.io, the owner's view onto nexus-flow (NEX-18). nexflow.it is not a v2 consumer: it stays on v1 from `main` until it decides its own move, and nothing in v2 serves its proprietary plugin.

The seam is the crate `nexus-flow-facade` in a new major version, with `Engine` as its entry point, as in v1 (decided by the owner, NEX-38). The same name then carries two incompatible lines, v1's on `main` for nexflow.it and v2's here; nexflow.it pins v1, and the major version says the rest. A new crate name was not taken: manufakt.io would rename its dependency for nothing. It is the only crate an embedder links. `nxs` is built on the same seam, and every `nxs` verb has a counterpart on it (the decision on CLI and library parity stays in force). The seam this RFC brings:

| Symbol | What it does |
| --- | --- |
| `Engine::open(repository: &Path)` | Opens the device member's database and the spaces `nexus-flow.toml` names |
| `Engine::open_space(space: SpaceId, git: Option<&Path>)` | Opens a space by id, for a caller without a checkout (manufakt.io); "on main" is then `null` |
| `Engine::as_session(session: SessionId)` | The same engine, opened on a running session's member database |
| `Engine::session_start`, `Engine::session_end` | Start and end a session's member, as `nxs session start` and `end` |
| `Engine::member_add`, `Engine::member_remove`, `Engine::members` | Add a member from its invitation with its role record, remove one, and list the current members with their role records |
| `Engine::sync`, `Engine::sync_status` | Wait until nothing is behind or unconfirmed, and report the sync state |
| `Engine::subscribe` | A stream of changes to the local database, for a view that redraws (requirement R3, met) |

Each module RFC adds its own reads and writes to `Engine` (tickets, decisions, goals, requests, status notes), with derived state computed inside, so manufakt.io never re-implements a rule.

What becomes of v1's crates on `v2`:

| Crate | In v2 |
| --- | --- |
| `foundation` | Retired. Its workspace, schema, signing and trust layers are NoCentralDB's job now. Repository discovery and `nexus-flow.toml` move into the new storage crate. |
| `core` | Retired. The op-log CRDT is replaced by NoCentralDB. RFC 0008's migration reads v1 data through it, and it is removed once the migration no longer needs it. |
| `facade` | Rewritten as the seam above, in a new major version. v1's facade keeps shipping from `main` for nexflow.it. |
| `sync`, `service`, `server` | Retired (see "Sync"). |
| new: `store` | The only crate that depends on the NoCentralDB SDK: the model, opening member databases, members and role records, sync, derived state. Not public; the facade re-exports what an embedder needs. |

*Not taken:* embedders reading NoCentralDB directly (every embedder re-implements the derived state, and a model change breaks them), and the CLI as the only interface (a process per call, no types); both were options in NEX-18.

### Public surfaces this RFC brings

GENESIS.md reserves these for the owner:

- **Commands**, flat rather than nested under one noun such as `nxs space member add`, which is longer to type and to read in a prompt (decided by the owner, NEX-38): `nxs init` (creates the repository space and `nexus-flow.toml`), `nxs device join`, `nxs space join-coordination <space id>`, `nxs session start [--role pm --pairing]`, `nxs session complete-pairing <response>`, `nxs session end`, `nxs member pair <request>`, `nxs member add <invitation> --role <role> [--ticket <ref>]`, `nxs member remove <member>`, `nxs member list`, `nxs sync [--watch] [--status] [--timeout <seconds>] [--relay <url>]`.
- **JSON output:** `nxs session start` (`session`, `member`, and `invitation` or, with `--pairing`, `pairing_request`), `nxs member pair` (`response`), `nxs member list` (`member`, `kind`: `device`, `agent` or `subAgent`, `role`, `ticket`, `parent`: the member that signed a sub-agent's certificate, or `null`, `account`: the account fingerprint its chain leads to), `nxs sync --status` (`member`, `relay`, `unconfirmed`, `synced_at`).
- **Environment variables:** `NXS_SESSION`, `NXS_DATA_DIR`.
- **Library seam:** the `Engine` symbols in the table above, and `SpaceId`, `SessionId`, `Member` as the types they take and return.
- **Stored formats:** `nexus-flow.toml` (`format = 1`), `model/model.ts` and `model/model.lock.json` as nexus-flow's model, the collection-naming rule, the `spaceMember` collection, the author field each module record carries until R6, and the data directory's layout.

### What nexus-flow needs from NoCentralDB

Each requirement this design rests on is either met, with the NoCentralDB RFC that meets it, or a request pm files on NoCentralDB's tracker.

**Met:**

- **R1. A space id grants nothing.** A relay connection proves only that a member key is held, and whether that member may read or write a space is checked per request (NoCentralDB RFC 0007, "Opening a connection"); content is encrypted, and the relay cannot read it (NoCentralDB RFC 0001, "The design in one page"). Knowing a space's id therefore reads nothing, and the id can be committed.
- **R2. Join by invitation.** A new member creates its own keys and hands over an invitation, which an admin adds, so no secret travels (NoCentralDB RFC 0009, RFC 0003). A role name is not part of it, and nexus-flow keeps the role in its own `spaceMember` record.
- **R8. An agent in the person's account, and its sub-agents.** An agent is paired into an account and receives a certificate, never the account key (NoCentralDB RFC 0010, "Accounts"; built in Rust and TypeScript). A sub-agent opens with `OpenOptions::sub_agent()`, holds no account, and its parent's `members.add` signs its certificate (NoCentralDB RFC 0009, "Sub-agents and ad-hoc spaces"; RFC 0010, rule 4). The chain person, pm, coder is therefore NoCentralDB's own.
- **R3. Change notification.** A subscription re-runs its query after every fold, from a local write or a sync, without the network (NoCentralDB RFC 0006, "Reads never touch the network"; RFC 0009, "Subscribe"), for `Engine::subscribe`.
- **R4. An additive check.** `ncdb model check` says whether one `model.lock.json` is an additive change of another (NoCentralDB RFC 0005); it needs Node 24.
- **R9. A first sync for a new member.** A member that joins a space pulls the whole log from the relay (NoCentralDB RFC 0007, "Pull: read from a cursor"), so a new member with an empty database, such as a cloud container, gets the whole space.
- **R10. A git-pinnable SDK.** The Rust SDK opens a database from the committed lock, `Db::open(OpenOptions::new(include_str!(…)))`, with no build script, so it builds from a git revision without `ncdb` (NoCentralDB RFC 0009, "The Rust SDK"). The crate is not published (`publish = false`), so a git pin is the only way to depend on it.
- Past changes of a removed member stay valid, up to the removal's cutoff (NoCentralDB RFC 0008, "The cutoff").

**Requests:**

- **R5. Members that lapse.** A member can be added with an expiry or a lease, so a session that dies without ending stops being a member on its own.
- **R6. Who signed a change.** An API that returns a change's author, and an object's history of changes with their authors, including members removed since. Today only the current members can be listed (NoCentralDB RFC 0009).
- **R7. Computed reach.** Computed fields or views (`nc.reach`) that express "every ticket this one depends on is done", so readiness need not be computed outside NoCentralDB.
- **R11. Finding a peer without a relay.** How two members on one device or one network find each other for peer sync, which NoCentralDB RFC 0007 leaves to a later RFC; until then island mode serves the device's member only, and several repositories with their agents on one device cannot share a space without a relay.

## What it does not do

- **It does not protect one module from another.** Rights apply to a whole space: a writer of the repository space can change its tickets, its decisions and its status notes alike. A decision record is protected from being edited away by the design of nexus-flow's RFC 0003 and by the signatures, not by a right.
- **It cannot add a member offline.** A member added on a machine that is offline becomes usable only once that change reaches the relay and the new member has synced. Until then the new session can do nothing in the space.
- **A lost session loses its unsynced writes.** The keys and the database die with the container. Writes a session made but did not sync are gone; that is why `nxs session end` waits until nothing is unconfirmed, and why a session that dies without ending can lose its last writes.
- **A lost only-admin device cannot be replaced.** Account recovery does not restore a device's space rights: a recovered account's spaces come back only when one of their members adds the new device (NoCentralDB RFC 0009, `AccountRecovered`). If the lost device was a space's only admin, nobody can add or remove members of that space again. `nxs init` says so and recommends a second admin device.
- **A dead session's member stays until removed.** Until R5 exists, a session that dies without `nxs session end` remains a member until its commissioner removes it. Nobody can sign as it, because its keys died with it.
- **pm must be paired by a person, and from its first open.** NoCentralDB cannot link an agent that already holds an account of its own (NoCentralDB RFC 0010), so a pm session opens for pairing from its first open, and a developer pairs every new pm on one of their devices.
- **A session is pm's sub-agent, so pm's member matters to it.** pm removes its sessions when it archives them; a pm that is replaced while its sessions still run stays a member until they have ended.
- **"Who wrote this" is a field, not a checked signature,** until R6: a module record's author field is written by the writer and not checked against the change's signature.
- **A relay can withhold or delay changes.** The relay cannot read or forge content, but it can hold changes back. A session can then act on an old state, for example take a ticket that another session already took. `nxs sync --status` shows what is unconfirmed; nothing detects a relay that withholds on purpose.
- **One relay per member.** A member syncs all its spaces through one relay, so the repositories of one coordination space must use the same relay.
- **Every read is as fresh as the last sync** (GENESIS.md, "Limits"). A command does not wait for sync, so a read can miss what another member wrote since.
- **Island mode reaches no one else, yet.** Without a relay, a space is the device member's alone until R11: until members on one device can find each other, the sessions there and the repositories that share a coordination space cannot sync with each other. A cloud session needs a relay either way.
- **"On main" is as fresh as the last `git fetch`** of the base branch, and is not known (`null`) to a caller without a checkout.
- **Derived state is computed in memory** until R7 lands, so it costs time in proportion to the space's tickets and links, on every read that needs it, unless the cache is fresh.
- **A ticket cannot link to a goal or a request.** They live in another space; a ticket names them through an `nc.ref`, which stays unresolved for a reader who is not a member there.
- **A repository belongs to at most one coordination space** in v2.

## Migration

This RFC moves no data. A v1 workspace (`.nxs/`) is left as it is: v2 neither opens nor changes it, and v1 keeps working from `main`. RFC 0008 designs the move of a v1 workspace and its data into the spaces described here, including workspaces that v1 syncs across several devices; until it is decided, `nxs init` on v2 creates an empty repository space beside an existing `.nxs/` and says that the v1 data has not been moved.

## Open questions

None. The owner answered them on 2026-11-17 (NEX-38), and the answers are written into the text above.

## History

- 2026-11-15: published as a draft in #142.
- 2026-11-17: decided by the owner (NEX-38), in #142.
