# E4 — The Executing Machine (Design Spec)

> Board: `6j6v.1c6k` (second slice of `6j6v.nvbz`). First slice `6j6v.f0b5` (machine names and
> presence), built on `6j6v.pzkb` (signed ops, trust list, the check before an action — see
> [E4-auth-identity-and-signed-ops.md](E4-auth-identity-and-signed-ops.md)). Written 2026-09-22.

## 1. The question

A chat started on the phone has to run on exactly one machine, and everybody has to know which.
The owner fixed the shape in three sentences:

- 2026-08-02 (`6j6v.gcnq`): a run triggered on my machine runs only there. Every machine sees all
  messages, and none starts an agent session because of that. Every machine can take over.
- 2026-09-17 (`nvbz`): when the executing machine is not there, **ask**. Choosing another machine is
  allowed, and the choice offered is every machine that is online.
- 2026-09-21: the machine can be set **for the chat** or **on the persona**.

And one condition (2026-09-21): a machine executes an order another device wrote only when that
device's origin is proven — the signed ops of `pzkb`.

## 2. Decisions

### 2.1 What is designated, and where it is written

**A persona chat carries its machine as a thread register**, `machine`, holding a machine id
(`ServiceHome::machine()`, `machine.toml`). It is an LWW register beside `expects_reply_from`,
`deadline` and `name`, so it syncs with the thread, every replica reads the same answer, and taking
over is simply writing it again.

Why the thread and not the declaration or the registry: the persona file travels through git, and
the catalogue an operation reads is frozen per machine (`declaration_freeze`), so two machines can
hold different versions of it. The register is the one place both machines read from the same
source — the synced log. The persona's `machine:` is consulted once, at the start of a chat, and its
answer is written into the thread.

**Precedence, highest first:**

1. **the chat** — `nxc send --to <persona> --machine <id|name>`, and later
   `nxc reply --thread <id> --machine <id|name>` to hand the chat to another machine;
2. **the persona** — `machine: <id|name>` in `.nxs-personas/<handle>.yaml`;
3. **the machine that starts the chat** — the owner's default.

A name is resolved to an id when the chat starts (against this machine and the presence list); the
register always holds the id. The name is for display and may change at any time (`f0b5`, note 1).

**A host that names no machine** (an embedding host that passes no `Machines`, the standalone chat
CLI) records nothing and spawns exactly as before. That keeps every existing embedding unchanged;
a host that wants the designation passes a `Machines` (2.6).

**Only a workspace that syncs is designated** (`Machines::applies_to`; `nxs`: bound to a stream).
One that syncs nowhere has a single machine, a designation there would decide nothing, and every
local-only workspace — every test suite and the parity differentials included — keeps its op log and
its routing exactly as before. The cost, named: a chat started before the workspace was bound has no
machine, so a reply from another device reaches nobody until somebody hands it over with
`reply --machine`.

**Only a chat a PERSON starts is designated.** A chat a running session starts (a persona's
sub-chat) always runs on that session's machine — its answer returns into the session, which lives
there — so it records no machine and keeps the routing a session's conversation always had; a
`--machine` naming another machine is refused.

**In this slice only a persona chat is designated** — `send --to <persona>` and the replies into
that chat. A declared channel runs where it is started, as before, and a persona's `machine:` is not
consulted when it is commissioned as a channel member: a channel's supervisor state is local to the
machine that runs it, so a member on another machine would answer into a supervisor that is not
there. Channels are their own slice (`6j6v.2d5r` belongs to it, 2.8).

### 2.2 Who starts, and when

| Situation | The machine that wrote the message | The designated machine |
|---|---|---|
| designated = this machine | starts now, as today | — (it is this one) |
| designated = another machine, online | posts, writes the register, starts **nothing**; the receipt names the machine (`handed_to`) | picks the order up after its next pull |
| designated = another machine, not online | **asks**: nothing is written, the refusal lists the online machines with "last seen" | — |
| chosen for this chat explicitly, not online | posts and hands over anyway: a person chose it knowingly; the order waits in the log | picks it up when it is back ("late, not lost") |

"Nothing is written" when asking is the owner's "nicht stillschweigend anstellen, nicht ablehnen":
the question comes before any write, so answering it with `--machine` is the same call again, not a
repair. An embedding app renders the same question from a read, `Engine::machine`
(`nxc machine --to <persona>` / `--thread <id>`), which returns the designation, where it came from,
whether it is online, and the online machines to choose from.

### 2.3 The pickup after the pull

The service runs `nxs chat pick-up` for a workspace after a pass when it has something to pick up.
The pickup is a **decision over current state, never a reaction to arrival** (E4 2.5):

A chat is **owed a turn on this machine** when

- its `machine` register names this machine and the op that wrote it **acts** (`acting_ops`: own, or
  verified by a trusted key);
- it lives in a direct conversation with a declared persona (the persona its `expects_reply_from`
  named when the chat started);
- its newest acting message is not the persona's own — somebody wrote to the persona after its
  last answer;
- and those messages are not yet claimed on this machine.

It then **resumes** the persona's session, when this replica holds one for the chat (a busy session
gets the messages held and delivered when its turn ends, the ordinary `reply` behaviour), or
**starts** one — the first turn of a chat, and a chat just handed over from another machine. A
message whose op does not act is never picked up: an order from an untrusted or unsigned origin
stays in the log, visible, and starts nothing.

### 2.4 Exactly once, per machine

The check proves origin, not freshness — so every order is executed once per message op id, under a
claim. **The claim is per MACHINE, not per workspace**: one machine can hold two replicas of one
stream (two clones of a repository), both pull the same order, and a per-workspace guard would start
it twice (`f0b5`, note 3). The claims live in the service home, `<home>/orders/`:

- `msg-<message id>` — created with `create_new`, so exactly one process on the machine wins it.
  The local path claims too, which is what stops the service picking up what `nxc send` or
  `nxc reply` just started here.
- `thread-<thread id>` — which replica of this machine serves the chat. The first one to claim it
  keeps it, so every later turn goes to the replica that holds the persona's session.

A spawn that fails gives its message claims back, so the next pickup tries again.

**Claims are recorded only for a workspace bound to a stream.** The service picks up only in a
workspace it syncs, and nothing reaches an unbound one from another replica, so a claim there would
guard against nothing — and every `nxc send` in a local-only workspace (every test suite included)
would leave a file in the service home for no reader. The cost, named: a workspace bound AFTER a
chat started in it leaves that chat's messages unclaimed, and a pickup then finds the persona's own
session and resumes it with them once more (held while it works).

**A reply written on the chat's own machine hands over that reply alone** when the persona has a
session there — the ordinary `reply` behaviour; everything since the persona's last answer goes only
to a FRESH session and to the service's pickup, whose claims say what is new. And the thread
return-address fallback of `reply --thread` does not run for a chat with a machine: it would be a
second route around both the hand-over and the claims.

### 2.5 Latency: a cheap question between passes

An order from the phone reaches the designated machine only on its next pass — up to the 300 s
safety-net interval. That is too slow for a chat. **Decision: the service asks the relay every
15 s whether there is anything new** — one `GET /ops?since=<cursor>&limit=1` per bound workspace —
and runs a full pass only when the answer is not empty. A chat then starts in about 15 to 25 seconds.

Cost, named: 4 small requests per minute per bound workspace per running service, each an indexed
range read that returns nothing almost every time. The full pass (register, push, pull, announce) is
still what runs when something moved, and at the safety-net interval. The peek is skipped for a
workspace inside its failure backoff, so an unreachable relay still costs one attempt per backoff
window, not one per 15 s. The realtime channel (`6j6v.2xff`) would bring this to about one second;
it is not needed for a chat to feel started, and stays its own slice.

The SENDING side had the same gap, found by the end-to-end run: the service treated the first
sighting of a workspace's `last-write` marker as no write at all, so in a freshly bound workspace
whose marker did not exist yet the first chat waited for the 300 s pass before it even reached the
relay. A marker the service has seen MISSING and then finds is now a write.

### 2.6 Where presence comes from: the host reads it

`nexus-chat` has no HTTP client, and the question needs the online machines at the moment a chat
starts. **Decision: the host reads presence**, through one injected trait:

```rust
pub trait Machines: Send + Sync {
    fn here(&self) -> Option<MachineRef>;                         // this machine, if it executes
    fn presence(&self, db_path: &str) -> Result<Vec<MachineSeen>, String>;  // judged, live
    fn claim_order(&self, message_id: &str) -> Result<bool, String>;
    fn release_order(&self, message_id: &str);
    fn claim_thread(&self, thread_id: &str, holder: &str) -> Result<String, String>;
}
```

`nxs` implements it with the one presence read (`nxs_sync::engine::machines`, judged by
`presence::judge`) and the service home's order claims; manufakt.io implements `presence` from its
own relay tables. Why not a cached list in the service: a cache is only true "as of" its pass, and
judging a cached relay age plus the time since the pass compounds two intervals and flips live
machines offline (`f0b5`, note 5). A live read at the moment of asking is exact and costs one
request per chat start.

**Presence is a claim, never an authorization** (`f0b5`, note 6). It decides only what the question
offers. What lets a machine act on an order is the signature check of 2.3.

### 2.7 What `pzkb` gives and what this adds

`pzkb` answers "may an agent act on this op?". This slice is its first consumer outside chat's own
verbs: the register and the order must both act. What it adds is freshness — once per op id, under
a per-machine claim — which a signature cannot give.

### 2.8 `6j6v.2d5r` — checked, not taken

The double synthesizer across devices comes from a CHANNEL's consolidation, which is started by
whichever replica sees the completing reply. Once channels are designated (the channel slice), the
consolidation runs only on the channel thread's machine, and `2d5r` shrinks to one check before
`claim_consolidation`: "is this thread's machine this one". It is not small today, because channels
are not designated in this slice. Recorded on `2d5r`.

## 3. Surfaces

| | CLI | Engine |
|---|---|---|
| designate for the chat | `nxc send --to <p> --machine <m>` | `SendToRequest::machine` |
| hand a chat over | `nxc reply --thread <t> --machine <m>` | `ReplyThreadRequest::machine` |
| designate on the persona | `machine: <m>` in the persona YAML | same file |
| the question, as data | `nxc machine --to <p>` / `--thread <t>` | `Engine::machine` |
| the pickup | `nxc pick-up` (run by the service) | `Engine::pick_up` |

## 4. Not in this slice

Channels (2.1), relay authentication and the filtered sync (`6aza`), the realtime channel (`2xff`),
pruning the order claims (a file per picked-up message; small, and named as its own chore).
