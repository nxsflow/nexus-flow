# A Reply Across Two Machines

[The journey of one operation](develop-the-journey-of-one-op) stays on one machine on purpose: one
verb, one log, one refold, so the four claims underneath it are visible without a second story
running beside them. This is the same trip with the axis that one leaves out — two replicas, and an
op that arrives from somewhere else.

**The scene.** An agent ran on the laptop and is owed an answer. You are at the desktop, so you
answer there. Nothing joins the two machines but a relay and an append-only log.

![A reply crossing from one machine to another: nxc reply appends ops on the desktop, the relay carries them, the laptop refolds its messages view — and the wake stays behind, on the one table that cannot travel](/nxs/docs/assets/a-reply-across-two-machines.webp)

## You reply on whichever machine you are at

```
nxc reply --thread m-7f2 "ship it — default the flag to off"
```

`reply` appends message ops to **this** machine's log, exactly as `nxf close` appended field ops in
the first journey, and the receipt comes back the moment they are durable. The CLI holds no model of
its own here either: it calls `Engine::reply_thread` on `crates/chat`, the same seam an embedding app
calls. Nothing has left the desktop yet.

## The desktop cannot tell that anybody is waiting

The receipt says `woke: null`, and it is **not** reporting an attempt that failed — it reports that
no wake was ever owed.

A thread's return address names a *session*, and sessions live in `session_map`: a plain table the
reducer never touches and sync never carries. The reason is in that module's own first paragraph — a
runtime session id "names a live session on THIS machine", so it is meaningless to a peer. Asking the
desktop's map who is behind that address answers *nobody*, and the resume path reads that as the
ordinary case rather than as a skip.

So the boundary this page exists for is already here, one step in:

> **The log carries what was said. It does not carry who is standing by to hear it.**

## The relay moves ops, never views

The desktop's background service pushes on its next pass; the laptop's pulls on its own. That
exchange is anti-entropy, not delivery: each side asks what the other holds that it lacks, and takes
the ops themselves. No op is addressed to a machine and no server keeps a queue per recipient — the
relay would have nowhere to put one, never having been told that a laptop is interested in this
thread.

This is the first journey's property seen from the other side. The view is a cache of the log, so
sync ships the log; and because it ships the log, a replica that was switched off for a week
converges by asking rather than by having been remembered.

## The laptop refolds, and says so

The pulled ops land in the laptop's `ops` table. A pass moves the log for **every** domain but folds
only flow's views, so the message sits there with the `messages` view not yet materialized; opening a
chat store runs the existing refold-when-behind, and the view catches up. Nothing in the daemon knows
how a message folds — the trigger is generic, the folding belongs to the reducer.

Each pulled op is **verified on the way in**: the laptop checks its signature against the key it
names and records the verdict beside it. The fold ignores that verdict — the laptop's board ends up
the same whatever it is — but the decision reads do not. Your reply discharges the laptop's thread
only if the desktop's key is on the laptop's trust list (`nxs sync trust add` there, once per
workspace); otherwise it sits in the thread marked `unvouched` and the round waits. That is the
point of signing: the relay in the middle cannot answer for you.

That commit bumps `PRAGMA data_version`, and that pragma is the whole of the change-notification
mechanism: a background thread polls it on its own read connection, and `Engine::subscribe` hands out
a coalesced *"someone wrote — re-read"* tick. No daemon of its own, no socket, no push.

## What continues, and what does not

An app that is **already alive** — a desktop client, a TUI, anything holding the engine open with a
`subscribe()` — gets that tick, re-reads its inbox, and shows the answer. That is the mechanism
working exactly as designed, across two machines, with no network call between the human and the app.

An agent session that has ended its turn is resumed by none of it. The laptop's service knows exactly
one chat job, and it is a deadline the laptop itself armed; an op arriving from elsewhere arms
nothing. The wake in the second section did not fail to cross — there was never a wake to cross,
because the fact that a session is waiting is not in the shared log and by construction cannot be.

If you are building on this, that is the sentence to keep:

> **Sync delivers your data, not your control flow.**

Resuming the work on the machine where it was left is a decision your app makes, out of the message
it has just read.
