# The Journey of One Operation

[Architecture](develop-architecture) is the map. This is one trip across it.

**The scene.** `bl0k` is a ticket you just finished; `fr33` is waiting for it. You close `bl0k` —
and `fr33` becomes the next thing to do, without anyone writing that down.

![One operation from the verb to a changed answer: nxf close, three appended rows, the refold, and the item that becomes next](/nxs/docs/assets/the-journey-of-one-op.webp)

## You close one ticket

```
nxf close bl0k --reason "shipped in #412"
```

The CLI holds no model of its own. It calls `Engine::close(now, actor, id, reason)` on
`crates/facade` — the same seam an embedding app calls. There is no private path the CLI can take
and an app cannot, which is why CLI coverage never counts as proof for the seam.

## The log grows by three rows

`write::close` appends rather than writes: `set_field(status)`, `set_field(closing_comment)`,
`set_field(closed_at)`. Each append becomes one row in the `ops` table.

```
12  bl0k  status           in_progress        ← still there
13  bl0k  status           closed
14  bl0k  closing_comment  shipped in #412
15  bl0k  closed_at        2026-09-02T09:14Z
```

Nothing was overwritten — and that holds for every verb before this one. `nxf create` appended five
rows per item, `nxf dep add` one edge row, `nxf claim` one more. The log *is* the history: every op
carries its own `author` and `wall_clock`, so who changed what is derivable from it without a
second audit table to keep in step. Each is also **signed** by the replica that appended it — the
key id and the signature ride with the op, and a receiving replica records whether they check out —
so "who changed what" is provable, not only readable. The `author` is the name for display; the key
is what an agent action is decided on.

## The views are refolded

A reducer folds each new op into the `items` view under keep-if-beats LWW — an op wins only if its
`(lamport, site)` beats what the view already holds, so two machines folding the same ops in
different orders land on the same row.

The view is a **cache** of the log. Delete it and a refold rebuilds it, which is why sync ships the
log and never the view.

## The answer changes

```console
$ nxf next
showing 1 of 1
bl0k  P2  in progress  [feature]  Ship the export path

$ nxf blocked
fr33  P2  open         [feature]  Publish the diagram
    ↳ blocked by: bl0k (in progress)
```

```console
$ nxf next
showing 1 of 1
fr33  P2  open         [feature]  Publish the diagram

$ nxf blocked
```

The second `nxf blocked` prints nothing at all — the lane is empty, so there is no line to show.
Between the two runs there was nothing but those three appended rows. `blocked_gating`
(`crates/core/src/derive.rs`) holds an item back only while a dep is not yet closed, so `fr33`
stops matching: the predicate ran again, nothing wrote to it.

**No row anywhere says which item comes next.** `nxf next` and `nxf blocked` are questions, asked
fresh each time — not columns someone keeps up to date. That is what makes the board consistent
without anyone maintaining it, and it is what the schema pays for the privilege: whatever you want
to ask has to be *derivable* from the log.
