# Running a relay

You have been working locally for a while. Now you want your board somewhere other than this
laptop — as a backup, or so a colleague (or another of your machines) can see the same work.

That needs a **relay**: a small server that holds the op log and hands it to every replica that
asks. There is no hosted one; you run it. The good news is that you already have it.

> **Read this first: the relay has no authentication yet.** Anyone who can reach its address can
> read *and* write the whole stream. Run it on a private network, a VPN, or behind something that
> does the authenticating — not on a public address. This is a known gap, not a configuration you
> can fix from the outside. What it can no longer do is **forge an instruction**: every op is signed
> by the machine that wrote it, and an agent acts only on ops signed by a key you trust — see
> *Whom a machine believes* below.

## You already have the binary

Every release tarball ships two programs: `nxs` (the CLI you already use) and `nxf-relay`. The
installer keeps the relay out of your `PATH` by default, because most people never run one:

```bash
NXF_INSTALL_RELAY=1 curl -fsSL https://nxsflow.com/nxs/install.sh | sh
```

If `nxs` is already installed, re-running the installer that way adds the relay beside it.

## The smallest thing that works

```bash
NXF_RELAY_ADDR=127.0.0.1:8787 NXF_RELAY_DB=/var/lib/nxf/relay.sqlite nxf-relay
```

That is the whole server. It stores everything in **one SQLite file** — which is also the whole
backup story: stop it (or snapshot the volume), copy the file, done. For one person with a few
machines, or a small team on one box, this is not a compromise; it is the right answer.

Point your workspaces at it and bind a stream:

```bash
nxs sync endpoint http://relay.internal:8787   # once per machine
nxs sync bind                                  # once per workspace
nxs sync run                                   # push + pull
```

In a git repo, `bind` needs no arguments: the stream id is derived from the `origin` remote, so
every clone lands on the same stream. See [migration](nxf-migration) for the other binding modes.

A relay behind a gateway that wants a key takes it in the URL,
`https://<user>:<password>@relay.example`. nxs sends it as a Basic `Authorization` header and never
repeats it: a failed pass names the relay by host, port and path only — in the service's log, its
status and `nxs sync machines` alike — and `nxs sync bind` and `nxs sync endpoint` show the URL
masked. The files that store it (`.nxs/sync.toml`, `~/.nexusflow/config.toml`) are readable by you
alone.

## Which machines sync a workspace

Once more than one machine syncs a workspace, the relay can tell you which ones do, and which of
them are online right now:

```text
$ nxs sync machines
stream stream-01j8… on http://relay.internal:8787 — 2 machines, 1 online
  online   Mac mini   last seen 42s ago  (this machine)
  offline  MacBook    last seen 3h 5m ago
```

Each machine's **background service** tells the relay "I am here" after every pass that synced. A
manual `nxs sync run` does not: "online" promises that something on that machine is attending the
workspace, and only the service keeps that promise. So a machine appears in the list once its
service (`nxs sync daemon install`) has synced the workspace.

**Online** means the machine's last announcement is at most twice the cadence its service promised,
plus one minute. The cadence is the service's interval, but never less than its 5-minute retry
backoff — so it is **11 minutes** for the default interval and for anything shorter. The second
cadence absorbs one failed pass, so a live machine does not flicker; the price is that a machine you
shut down still reads as online for up to 11 minutes. `--json` gives each machine's `last_seen_at`,
`age_secs` and the window it was judged by (`online_within_secs`).

**A machine is one service home.** On an ordinary computer that is the computer itself. A
development instance (`~/.nexusflow-dev`) is a machine of its own — it is a separate service — and
its default name says so: `studio (dev)`.

**The name** defaults to the computer's host name, and `nxs sync bind` tells you which name that is
before the service first announces it. See and change it with:

```bash
nxs sync machine               # this machine's name and id
nxs sync machine "Mac mini"    # rename it; the id stays
```

A name has to read as what it is: up to 64 characters, no invisible or direction-changing
characters, no whitespace but spaces.

The relay has no authentication, and that shapes three things:

- **Anyone who can read the stream can read the machine names**, along with when each was last
  seen. If your host name carries your own name, rename the machine. Nothing else is sent: an id
  minted on the machine (a ULID — its first characters say when it was minted), the name, and the
  service's cadence. No paths, no user, no hardware id.
- **A listing is a claim, not proof.** Anyone who can reach the relay can announce a machine, or
  keep a stopped one looking online — for up to about two hours, the longest cadence a relay
  accepts. Treat the list as a guide to which machines exist and are awake, never as permission.
- **It is bounded.** A machine nobody has heard from for 30 days is forgotten, and reappears the
  moment its service announces again. One answer lists at most 100 machines; if the relay holds more
  (a flood of invented ones, typically), `nxs sync machines` says the list is not complete.

Presence is **not part of the op log**. The relay keeps one entry per machine per stream and
overwrites it with every announcement: SQLite and Postgres in a `machine_presence` table the relay
creates on boot, DynamoDB as a third kind of item (`machine#<id>`) in the registry table you already
have — no new table. On DynamoDB the relay's role now also needs **`dynamodb:Query`** on that table,
and you may enable TTL on its `expires_at` attribute to have forgotten machines removed (without it
they are only hidden). A relay built before this existed simply does not record presence:
`nxs sync machines` says so, and syncing carries on unaffected. A relay behind a gateway has to
route `GET` and `POST` on `/streams/{id}/machines` as well.

## Whom a machine believes

Every op a workspace writes is **signed** by it, and every op it receives is **checked**. The
check decides one thing only: whether an **agent action** may follow the op here. The board itself
never asks — every op is kept and shown whoever signed it, so all machines still end up with the
same board.

An action follows an op when this workspace wrote it, or when its signature checks out against a
key on this workspace's **trust list**. Everything else — an op from a key you have not trusted,
one whose signature does not check out, one with no signature at all — is shown and acts nowhere:
a reply in it answers no round, names no session to wake, is collected as nobody's answer.

The key belongs to the workspace on that machine: its private half is `.nxs/signing.key`, readable
by you alone, created the first time the workspace is opened. To let your desk act on what your
laptop starts, in the same workspace on each:

```bash
nxs sync key                                        # on the laptop: prints ed25519:…
nxs sync trust add ed25519:… --name laptop          # on the desk
```

**Compare that string over a channel you trust** — read it out, paste it into a chat you know is
yours — never through the relay: the relay is exactly the party the check exists to distrust.
`nxs sync trust list` helps you find the right key: it shows the keys you trust and the keys that
signed ops here without being trusted, with the names their ops claim. The names are claims; the
key is what you compare.

```bash
nxs sync trust list                                 # whom this workspace believes
nxs sync verify <op-or-message-id>                  # who signed one op, and whether it may act
nxs sync trust remove ed25519:…                     # stop believing a key, at once
```

The list is **local**: it is never synced, a snapshot does not carry it, and each machine decides
for itself whom it believes. Removing a key takes effect immediately, for that key's past ops as
well. There is no MCP tool for any of this, on purpose: an agent's instruction must not be able to
widen whom your machine believes.

What you see when something is not trusted:

- **A message** in `nxc threads show` reads `(unvouched — no agent action follows it)`, and carries
  `"unvouched": true` in `--json`.
- **A thread** whose opening or whose "who owes an answer, until when" came from an op you do not
  trust is **held**: it says so in `nxc threads show`, and `nxc tick` answers `held` instead of acting.
- **A sync pass warns** when signatures went missing on the way — ops this machine holds signed that
  came back without their signature, or unsigned ops from machines that otherwise sign. A relay older
  than nxs 0.58 drops them: upgrade it. The ops are kept either way; they just act nowhere. And
  `nxs sync trust list` shows how many ops each trusted key has signed here: a machine you trust
  whose changes arrive while that count stays at zero is behind such a relay.

Workspaces written before signing existed keep working: what this machine wrote before is its own,
and ops from machines that do not sign yet are shown and act nowhere.

## Which machine runs a chat

A chat with a persona runs on exactly **one** machine; every other machine that syncs the workspace
shows it and starts nothing. Which one is decided when the chat starts — `nxc send --to <persona>
--machine <m>`, else the persona's `machine:`, else the machine that starts it — and written into the
chat, so a chat started on your phone can run on the Mac mini, and a reply from anywhere reaches the
persona there. The details are in the chat guide ([personas](nxc-personas)).

What makes it work across machines is what this page already sets up:

- **The designated machine's service picks the chat up.** It asks the relay every 15 seconds
  whether anything is new — one small request per workspace, a full pass only when the answer is
  yes — so a chat written elsewhere starts there within about half a minute.
- **Only from a machine it trusts.** The order and the designation both have to come from a key on
  its trust list (see above). An order from anywhere else is shown in the thread and starts nothing.
- **Once.** Each message is taken once per machine, even when that machine holds two clones of the
  same repository.
- **"Online" decides whether you are asked.** A chat for a machine that is not online is not sent:
  you get the machines that are, and choose. The list is the one `nxs sync machines` shows — a guide,
  never a permission.

## A new machine from a snapshot

A new machine folds the whole history the first time it syncs: every op the board ever had, one at
a time. For a board a few thousand ops long that is a second or two; it grows with the board.
Instead, a machine that already syncs the board can hand over a **snapshot** — its op log, the board
folded from it, and the relay position the two reach — and the new machine starts from that and
pulls only what came after:

```bash
nxs sync run                              # on a machine that syncs the board: push first
nxs sync snapshot board.snapshot          # write the snapshot

nxs init                                  # on the new machine, in the new clone
nxs sync bind --snapshot board.snapshot --endpoint http://relay.internal:8787
```

`bind --snapshot` joins the stream the snapshot was taken from, takes the snapshot, and only then
registers the workspace with the background service — whose first pass pulls just the rest. The new
machine then shows exactly the board the source shows: the promise is that a snapshot plus the rest
folds to the same state as the whole history, late edits written into the past and ops the relay
stores twice included.

What it checks, and refuses by name:

- **The source has pushed everything.** `nxs sync snapshot` refuses while this machine holds ops the
  relay has not seen: a snapshot promises that everything in it is reachable from the relay.
- **The new workspace is empty.** A snapshot starts a fresh replica; a workspace that already holds
  ops binds without `--snapshot` and syncs the ordinary way. A refused bind leaves it unbound and
  empty; one interrupted midway can leave the log loaded but unbound — bind it again without
  `--snapshot`, and it syncs from there.
- **The machine's id prefix is its own.** If another replica on the stream already holds the prefix
  the new workspace was given, the relay hands it a fresh one before the import, so the ids that
  arrive stay as they are.
- **The position is this relay's.** Taking a snapshot asks the relay which op it holds at the
  position the machine has synced to, refuses if the machine does not have that op, and writes its
  id into the snapshot. Before anything is written, the new machine asks the relay for the op at the
  same position and requires the same one. A snapshot taken against another relay, or before a
  relay's log was rebuilt or reordered, fails that check — and the machine then pulls from the start
  instead, skipping every op it already has. Slower, never short. (It checks the numbering at one
  position, which a different log almost never matches by accident; it is not a proof that the
  whole history before it is the same list.)
- **The version.** A snapshot another nxs version wrote is folded again from the log it carries,
  and `bind` says so; one from an nxs whose database this one cannot read is refused. `--json`
  reports which (`snapshot.views`: `taken` or `refolded`, with the reason).

**Where a snapshot lives is up to you** — a file you copy, a bucket an app keeps. The relay does not
hold snapshots: it cannot fold, and while it authenticates nobody, a snapshot left there would be a
folded board anyone could plant and nobody could check against its log. A snapshot carries the op
log and the board folded from it, so it reveals what the stream itself reveals to anyone who can
read it — and none of what nxs keeps only for this machine (agent transcripts, sessions, leases).
nxs writes it readable by you alone. And a snapshot's **board is taken as given**: it is as
trustworthy as the machine it came from, so hand over only files from machines you control. Its log
is checked like any op that arrives: every signature is verified again on the new machine, which
starts with a trust list of its own — so a snapshot cannot make anything act there. One the import could not read back — a log with a value of the wrong type, a view cell that
does not fit its column — is refused whole, before anything is written.

An app that embeds nxs — a hosted board that starts cold — does the same through
`nxs_sync::snapshot::{export, import}`: export from a replica that is in sync (the export confirms
the position with the relay), store the bytes where the app trusts them, and on the next cold start
import them into an empty store and resume syncing from the position `import` returns. A replica that
will write registers its id prefix first, as it would before any first sync.

## Sync to a Postgres, for example Supabase

When you would rather have a managed database keep the data — someone else's backups, someone
else's disks — point the relay at Postgres instead. Any Postgres works: Supabase, Neon, RDS, or
your own.

```bash
export NXF_RELAY_BACKEND=postgres
export NXF_RELAY_PG_URL='postgres://user:pass@db.example.com:5432/nxf?sslmode=require'
nxf-relay
```

The relay creates its own tables on first boot, so there is no migration step to run.

A few things worth knowing:

- **`sslmode=require`** is what you want for anything reachable over the internet; every managed
  provider requires it. The default (`prefer`) negotiates TLS when the server offers it and falls
  back to plaintext when it does not, which is right for a database on your own private network
  and wrong for one on the public internet — so say `require` and mean it. If you point the relay
  at a non-local host and leave it on `prefer`, it says so at startup rather than quietly sending
  your op log in the clear.
- **A private or self-signed CA** is trusted by pointing `NXF_RELAY_PG_CA_FILE` at its PEM file.
  Public certificate authorities are built in and need no setup — but do not assume a managed
  provider uses one. **Supabase signs its Postgres and pooler certificates with its own root**, so
  a relay pointed there fails the handshake with `invalid peer certificate: UnknownIssuer` until
  you download that certificate (Project Settings → Database → SSL Configuration) and set
  `NXF_RELAY_PG_CA_FILE=/path/to/prod-ca-2021.crt`. This is the one extra step Supabase needs, and
  it applies to any provider with its own CA.
- **The pool** is `NXF_RELAY_PG_POOL_MAX_SIZE` (default 16) and
  `NXF_RELAY_PG_ACQUIRE_TIMEOUT_SECS` (default 30).

Switching backends changes nothing a client can see: same HTTP surface, same convergence, same
stream ids. You can move from SQLite to Postgres by standing up a second relay and re-binding.

## DynamoDB: build it yourself

The relay also has a DynamoDB backend, for serverless deployments where there is no disk to own.
It is **not** in the released binary, on purpose — it drags a native cryptography toolchain
(`cmake`, a C compiler) into every build to add ~19 MB to every download, for something almost
nobody runs. It is a supported configuration, just a source-built one:

```bash
git clone https://github.com/nxsflow/nexus-flow.git && cd nexus-flow
git checkout v0.55.0                                     # pin an exact release
cargo build --release -p nxs-server --bin nxf-relay --features dynamodb
```

You need `cmake` and a C toolchain on the build host, and you must create the two tables
yourself — the relay never creates or migrates them. Their required shape is documented at the
top of `crates/server/src/store_ddb.rs`.

If you point a **released** binary at `NXF_RELAY_BACKEND=dynamodb`, it will refuse to start and
tell you exactly this: the backend is a build-time feature, and this build does not carry it.

## What is not here yet

- **Authentication.** See the warning at the top. Until it ships, treat relay reachability as the
  access control, because it is. Signatures stop a relay from forging an instruction; they do not
  stop anyone who can reach it from reading the stream, writing ops that act nowhere, or holding
  ops back.
- **A container image.** `docker run …/nxf-relay` is the obvious way to run this, and it is
  specified — but publishing a one-command way to stand up an *unauthenticated* shared board is
  the wrong order to do things in. It follows authentication.
