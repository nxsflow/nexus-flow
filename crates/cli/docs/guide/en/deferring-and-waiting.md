# Deferring and waiting

Two things look similar — "not now" and "not until something else happens" — but nexus-flow models
them very differently. Getting the distinction right keeps the `deferred` and `next` lanes honest,
so the derived work list stays trustworthy.

## Defer is for real calendar dates only

`defer_until` hides an item until a **date** arrives. Use it only when there is a genuine calendar
day before which the work cannot sensibly start — a filing deadline opens, a quarter begins, an
embargo lifts:

```bash
# a real date: don't surface this until the quarter starts
nxf create --type chore --title "Pay quarterly taxes" --priority P3 --defer 2026-07-01
nxf update <id> --set defer=2026-07-01
```

Before the date the item sits in `nxf deferred` and is kept out of `nxf next`; on the date it
becomes ready automatically. That behaviour is only meaningful when the date is real.

**Anti-pattern: a placeholder defer date.** Do *not* pick an arbitrary future date to mean "someday,
once X ships". You do not know when X ships, so any date you invent is a lie: too early and the item
snaps back into `next` before it is actionable; too late and it stays hidden after it was ready. A
defer date you would have to keep nudging is the tell — that is not a calendar event, it is a
*dependency*, and it wants the pattern below.

## Waiting on a delivery: the WAIT chore

nexus-flow has **no cross-workspace dependencies** — an item in this workspace cannot `dep` onto an
item in another repo's board. So when your work is waiting on an external **delivery** (a release in
another repo, a partner's API, an upstream fix), you model that delivery as a first-class item *in
your own workspace*: an open **WAIT chore**.

The chore stands in for the thing you are waiting on. Title it with a `WAIT:` prefix so it is
unmistakable, have every dependent item depend on it, and let its lifecycle drive the chain:

```bash
# 1. an anchor for the delivery you're waiting on
nxf create --type chore --title "WAIT: acme-api v2 ships (need the /batch endpoint)" --priority P2 -q
# → 6j6v.w8t2

# 2. the work that needs it depends on the anchor (so it shows as blocked, not ready)
nxf dep add 6j6v.k1a9 6j6v.w8t2
nxf dep add 6j6v.p3f0 6j6v.w8t2

# 3. when the delivery lands, CLOSE the anchor — put the delivered version in the reason
nxf close 6j6v.w8t2 --reason "acme-api v2.3.0 shipped with /batch; verified against staging"
```

Closing the chore is the **event** that releases the chain: the moment it closes, everything that
depended on it stops being blocked and returns to `nxf next`. The closing reason — with the concrete
version — is the durable record of *what* arrived and *when*, right where the next agent will look.

Each workspace keeps **its own** anchor for the same upstream delivery; there is no shared
cross-repo ticket. That is deliberate — every board stays self-contained and converges on its own.

## Why a WAIT chore is not "just work"

A WAIT chore is technically **ready** (it is open and unblocked, so it can appear in `nxf next`), but
its action is not "build it" — it is "check whether the delivery has arrived, and if so, close it
with the version". Read a `WAIT:` item that way: it is a recurring *poll*, not a task to sit down and
do. Keeping the prefix consistent is what lets you (and future tooling) tell the two apart at a
glance.

## In one line

- **Defer** → a real calendar date, nothing else.
- **Wait on a delivery** → an open `WAIT:` chore your items `dep` onto, closed (with the version)
  when it lands.
