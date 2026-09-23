# E4 — Identity, Relay Auth, Authorization, Signed Ops (Design Spec, the map)

> Board: `6j6v.6aza` (the auth slice), first slice `6j6v.pzkb` (signed ops + trust list, no
> cloud), with `6j6v.m19v` (a foreign Lamport number cannot tip the local clock) in the same
> move. Written 2026-09-22.
>
> This is the MAP of the auth work, kept short on purpose: it names every seam, says which one
> this slice builds and how the others attach later. It is not the construction plan of the
> later slices.

## 1. The question

Whom does a machine believe an instruction that another device triggered? Today nobody: the
relay authenticates no one, `op.author` is self-declared, and a reader cannot tell a message a
colleague wrote from one a stranger with the stream id planted. Four separate things answer four
separate parts of that question. Keeping them apart is the whole design:

| Seam | Answers | This slice |
|---|---|---|
| **Identity** | Which key wrote this op? | **built** — an Ed25519 key per replica |
| **Signature** | Did that key really write exactly these bytes, and do I trust it? | **built** — sign at append, verify at receipt, a local trust list, the check before an action |
| **Relay authentication** | May this caller push to / pull from this stream at all? | later — `6j6v.ax54`, `6j6v.1rgq` |
| **Authorization** | Which channels / kinds may this identity read or write? | later — 6aza point 2, the filtered sync |
| Confidentiality | Can the relay read the payload? | later — 6aza point 5, nexflow.it only |

The signature is independent of the other three: it proves ORIGIN, whatever carried the op and
whoever was allowed to carry it. That is why it can come first — and why a relay that
authenticates nobody stays a denial-of-service risk after this slice, but no longer a forgery
risk.

## 2. Decisions of this slice

### 2.1 The key hangs on the replica, not on the machine

Each replica (one `.nxs/` workspace on one machine) holds one Ed25519 key in
`.nxs/signing.key`, created on the first open that finds none — on Unix readable by its owner
only (`0600`, and narrowed back to it when found wider); on Windows as private as the folder it
sits in, which under the user profile is the owner's alone (an owner-only ACL is `6j6v.8f70`). The public key IS the key id (`ed25519:<base64url>`), so every receiver can check every
signature without a directory; whether it BELIEVES the key is a separate question (2.3).

Why the replica and not the machine (`6j6v.f0b5`'s service home):

1. **Scope is where replay dies.** Trust is per workspace. A replica key exists in exactly one
   workspace, so a validly signed op copied from workspace A into workspace B's stream arrives in
   B as *verified, from a key nobody here trusts* — no action. A machine key would be trusted in
   every workspace the machine syncs, and the same copy would act. Binding the stream into the
   signed bytes was the alternative; it breaks the day a workspace moves to another stream
   (`sync bind --rebind` re-pushes old ops, whose signatures would then name the wrong stream).
2. **No write path can forget to sign.** The substrate store opens the key beside its own db
   file, so the CLI, the MCP server, the service and every embedding app sign through the same
   `Store` without anybody handing a key around. A machine key would have to be plumbed into
   every store opener, and a missed one fails silent: unsigned local ops.
3. **One replica, one site, one key.** A replica already owns a coordinate space (`site`); the
   key names the same unit. A development and a production service instance writing the same
   workspace sign alike.
4. It is where the 6aza direction anchored it (point 1, "am replica_uuid verankert").

The cost, named: a second machine is trusted once PER WORKSPACE (the trust list is per workspace
anyway), and two separate clones of one repository on one machine are two replicas that trust
each other explicitly. `.nxs/` is git-ignored by its own `.gitignore`, and the repository
`.gitignore` blocks `*.key` besides. A snapshot never carries the key (it carries the db).

An in-memory store gets a key of its own for its lifetime, so tests and ephemeral embedders sign
too; nothing it signs can be trusted by anybody else, which is right.

### 2.2 The envelope

- **Signed bytes:** the op's canonical bytes (`Op::canonical_bytes`, spelled a second time as
  `WireOp::canonical_bytes` for the relay, which does not link the foundation; a test holds the
  two equal). Tag `nxs-op/2`: the `nxs-op/1` form of `6j6v.xsf3` also covered the envelope
  version and never signed anything — the envelope version is transport metadata a later build
  rewrites when it re-wraps an op, so a signature over it would break on the first push after an
  upgrade. The signature covers every field of the op, the domain included.
- **`key_id` and `sig` ride in the envelope's passthrough** (`WireOp::extra`, `6j6v.5crb`),
  under those two names. They are not part of the signed bytes (a signature cannot cover
  itself; the key id is a lookup hint — pointing it at another key only makes the signature
  fail). Every relay since 5crb (nxs 0.58) carries them with no change to its storage; a relay
  OLDER than that strips them, which 2.6 makes loud.
- **On the receiving side both are columns of the op log** (`ops.key_id`, `ops.sig`), beside a
  third, local one: `ops.provenance`, the receiver's verdict — `own`, `verified`, `invalid` or
  `unsigned` (2.4). The log becomes a signed audit log: anyone can re-verify it, and "who
  ordered this" is provable, not only traceable (owner, 2026-08-10).

### 2.3 The trust list

- **Local, per replica, never synced.** A table in the workspace db (`trusted_keys`) that no op
  writes, so no relay and no peer can extend whom this machine believes. It is not in a snapshot
  either: a new replica decides its own trust.
- **Its own key is in it from the start** and cannot be removed.
- **Only the local user changes it** — `nxs sync trust add|remove` and the embedding seam. There
  is deliberately no MCP tool for it: an agent's instruction must not be able to widen whom the
  machine believes.
- **First trust is explicit, out of band.** `nxs sync key` on machine A prints its key id;
  `nxs sync trust add <key-id> --name A` on machine B. No trust-on-first-use: the first op of a
  new key reaches B through the relay, the one party signatures exist to distrust.
  `nxs sync trust list` shows the keys that signed ops in this log but are not trusted, with the
  authors they signed as — a pointer to what to compare, not a shortcut past comparing it.
- **Revocation is removal**, effective at once: trust is evaluated when an action is decided,
  never stored on the op, so a removed key's past ops stop carrying actions immediately.

### 2.4 Verify, record, and the check before an action

- **At append** every local op is signed with the replica key; its provenance is `own`.
- **At receipt** (`Store::apply`, and `Store::load_image` for a snapshot) every op is verified
  against the key its `key_id` names; the verdict lands in `ops.provenance`. A verdict is never
  taken from outside — not from the wire, not from an image.
- **Convergence does not depend on trust.** Every op is stored and folded exactly as before,
  whatever its verdict. Only actions look at it.
- **The check:** an op carries an agent action iff its provenance is `own`, or `verified` by a
  key in the trust list. One definition, exposed twice: `Store::acts_on(op_id)` for code, and the
  SQL view `acting_ops` for the decision reads that are SQL.
- **Where it is applied** — every place a received op can steer an action today (chat's local
  verbs `tick`/`reply`/`deliver`/`resume` act on views that include foreign rows):
  - a reply discharges an obligation (the quorum), names the session a reply resumes (the return
    address), is collected as a member's answer, and moves the park decision — only when its op
    acts;
  - a thread whose `open` op or whose winning `expects_reply_from` / `deadline` register does not
    act is **held**: it never completes and never goes stale, so no consolidation, next step or
    wake follows from it.
  `6j6v.1c6k` (the executing machine picks work up after a pull) uses `Store::acts_on`.
- **Old local ops:** ops written before this version carry no signature. At the migration, the
  ones on this replica's own site are taken as `own` — the log cannot tell them apart, and
  nothing was signed before. Everything received afterwards is judged by its signature.

### 2.5 Replay — answered

- **The same op again** (a relay re-delivering, an attacker replaying): `op_id` is the log's
  primary key, so it is the idempotent union's no-op — no row, no fold, no new verdict. Actions
  are decisions over current state, not reactions to arrival, and a duplicate changes no state.
- **An old op to a replica that never had it** is not a replay, it is sync: it is the original
  op, byte for byte, and convergence needs it. The check proves ORIGIN, not FRESHNESS — so
  `1c6k` must execute an order once per op id and under a claim, never on arrival.
- **Into another workspace:** verified, untrusted there (2.1.1).
- **Altered** (same op id, a changed field): the signature fails, the verdict is `invalid`, no
  action. If the original is already held, the altered copy is a re-delivery of a known op id
  and ignored. The one thing a hostile relay can do is serve its altered copy FIRST: then that
  replica keeps the altered, invalid op and never acts on the genuine one — a denial, never a
  forgery. Named here as the known limit of a relay that authenticates nobody.

### 2.6 Silent downgrade — answered

- **An op without a signature never acts**, whatever else its origin signed. A stripped
  signature therefore costs an action, never grants one.
- **It is never silent.** A sync pass reports two counts: `signatures_stripped` — ops this
  replica already held WITH a signature that the relay delivered WITHOUT one, its own coming back
  from its own push included (the relay predates 5crb, or tampers) — and `unsigned_from_signers`
  — ops new to this replica, delivered unsigned, from a site whose other ops are signed. `nxs sync
  run` warns with both and says what to do; the service writes the same sentence to its log. A relay
  that has stripped EVERY signature from the start leaves the receiver nothing to compare against,
  so the trust list says it instead: each trusted key shows how many ops it verified here, and a
  trusted key with none while that machine's changes arrive is named as a relay dropping signatures.
  The receiver's own pushes coming back unsigned are counted as `signatures_stripped` too.
- **Old clients** keep syncing: their ops are unsigned, stored and folded, and act nowhere.
- **An old binary on the same machine** (a long-running process from before an update) writes
  local ops without a signature, which therefore carry no action until the process is restarted,
  and it drops the signatures of what it pulls. The order that avoids both is the one 6aza
  already states: the relay first, then the clients.

### 2.7 A foreign Lamport number cannot tip the clock (`6j6v.m19v`)

An op may carry any Lamport number, and the relay authenticates nobody. Two bounds:

- **`MAX_LAMPORT = 2^62`** — an op above it is kept (§7 forbids dropping it) and **inert**: it
  folds nowhere and moves no clock. A pure function of the op, so every replica agrees and the
  board still converges.
- **`CLOCK_CEILING = 2^61`** — the highest number a RECEIVED op can move a clock to. Without it
  the attack only moves to the bound: an op at exactly `MAX_LAMPORT` would park every clock there,
  and every later local op would land above the bound and be inert — a frozen board instead of a
  crash. Between the two bounds lie `2^61` local writes of headroom.
- The cost, named: an op between the two bounds folds but does not move the clock, so it keeps
  winning its one field against later writes — a relay-level writer can PIN a field (it can
  already overwrite any field), but it can no longer crash a replica, wrap its clock or freeze the
  board. Which ops a replica ACTS on is the signature's question, not the clock's.
- The clock is seeded from the log's highest in-bound number, capped at the ceiling, and from this
  replica's OWN ops above it — `provenance = own`, which no received op can claim. A received op
  squatting a coordinate on this replica's site is stepped over, not collided with.

## 3. How the later seams attach

- **Relay authentication** (`ax54`, `1rgq`): the `Authenticator` default is a keypair challenge —
  and the replica key of 2.1 is that keypair; the relay's allowlist holds public keys in the same
  `ed25519:` form. OIDC (the central broker `id.nxsflow.com`, Cognito behind it) is one more
  `Authenticator`, which maps a token to an identity; it does not replace op signatures, it
  decides who may talk to the relay.
- **Authorization / filtered sync** (6aza point 2): an `Authorizer` filter in
  `OpStore::read_since`, keyed on the authenticated identity; the pull loop already terminates on
  the cursor, not on page shape (`6j6v.xsf3` invariant 4), so it stays a server change. With
  E2EE the filter can only read what stays in the clear: stream, domain, target kind and channel.
- **Confidentiality** (nexflow.it): the payload is encrypted, the signature still proves origin.
  Whether it covers the plaintext or the ciphertext is that slice's decision; the canonical form
  is tagged (`nxs-op/2`), so a later `nxs-op/3` cannot be confused with a signature over today's
  bytes.
- **The hosted engine** (manufakt.io's web board) is a replica like any other: it has its own key,
  and a machine that should act on work started in the web board trusts that key.

## 4. Not in this slice

Relay authentication, authorization and the filtered sync, E2E encryption, and the pickup after
a pull itself (`6j6v.1c6k` — built since, see [E4-executing-machine.md](E4-executing-machine.md)).
