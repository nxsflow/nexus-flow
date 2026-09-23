# 07a — Parent↔child status coupling (effective-lane derivation) (Plan-Spec, #07a.6)

Decision record + decomposition spec for coupling a child's **effective lane** to its parents'
lifecycle. This is the **spec gate** for epic #07a: it writes down the agreed model and formally
**inverts sp6.7 ②c** (`crates/core/src/derive.rs`, the "parenthood is containment, not blocking"
invariant) before any implementation slice starts. The implementation children (#07a.1–#07a.5) are
`blocked-by` this node.

## 0. Goal & non-goals

**Goal:** make the `parent` edge a **gating containment** edge — a child's *effective* (displayed,
derived) lane depends on its parents' lifecycle — while `contributes_to` (#vf4, n:m, never blocks)
stays the **non-gating** escape hatch for "belongs here, contributes there". The child's **stored
status is never rewritten** by this coupling (except the one deliberate write in §6); the lane is a
pure derivation, convergent SQL over the materialized views.

This **inverts** the deliberately parent-agnostic derivation shipped as sp6.7 ②c (#sp6.7), where
"Readiness is computed from `dep` edges, status, defer date, and dep-cycles ALONE — it never reads
the `parent` edge." That invariant — "parenthood is *containment*, not a blocking relationship" —
becomes "**parent is gating containment; `contributes_to` is the non-gating edge**".

**Non-goals (the implementation slices that this gate unblocks):**

- **#07a.1** — claim-up propagation (the only *stored* write; ancestors actually become `in_progress`).
- **#07a.2** — suppress children of a `blocked`/`deferred` ancestor from `ready`/`next`.
- **#07a.3** — effective *display* status from the ancestor lane (ready-mask, closed-mask,
  `parent_closed_reason`).
- **#07a.4** — `prime`/guidance copy: `parent` (gating) vs `contributes_to` (non-gating).
- **#07a.5** — `close(parent)`: warn in the receipt about silently-swept children.

This document changes **no derivation behavior**. It authors the spec, rewrites the ②c module doc
to point at this target model (flagging that the gating derivation rolls out across #07a.1–.5, so the
code is still parent-agnostic until then), and reframes — but **keeps green** — the two ②c pinning
tests as the *pre-gating baseline* the implementation slices will invert (see §9).

## 1. Effective lane of a child (DERIVED)

The child's **stored** `status` is never rewritten by this table (claim-up §6 is the one exception,
and it writes `in_progress` *up*, never down). The effective lane is what `ready`/`next`/`blocked`
selection and the display layer compute from the child's own state **plus** its ancestors' lanes:

| Ancestor situation | Effective lane / display of the child |
| --- | --- |
| no parents | own lane (today's behavior, unchanged) |
| **all** parents `closed` | `closed` (masked) + `parent_closed_reason`, when the child itself is still `open` |
| an open parent `blocked`/`deferred` | **suppressed** from `ready`/`next` |
| an open parent `ready` (open, not in_progress) | shown as `ready`, even if the child is `in_progress` |
| an open parent `in_progress` | child shows its **own** status |

"Ancestor" means the full transitive `parent` chain, walked up the **`parent` edge** (not the
single-parent `belongs_to` projection) — see §7.

## 2. Multi-parent precedence — most-restrictive-wins (PROVISIONAL)

A child may have n parents (the OR-set `parent` edge is n:m, #sp6.3). When a child's parents sit in
different lanes, the **most restrictive** ancestor lane wins:

```
blocked / deferred  (suppress)   >   ready-mask   >   in_progress / own status
```

This is **provisional** — we expect to tune it. The sanctioned escape hatch for "this item belongs
here but should NOT be gated by that other container" is **`contributes_to`, never a second
`parent`** (§3). Precedence is evaluated over the child's **full ancestor set**, so a single blocked
grandparent anywhere up the chain suppresses the child.

**The ladder ranks OPEN ancestors; the closed-mask is the no-open-ancestor case.** The three rungs
above all concern *open* parents. The **all-parents-closed → closed-mask** rule (§4) applies precisely
when there is **no open ancestor left** to rank — so the ladder and the closed-mask partition the
space without overlap: if any parent is still open, the most-restrictive *open* ancestor decides;
only when *every* parent is closed does the closed-mask take over.

**Why `ready-mask` outranks `in_progress`/own:** a not-yet-started container (an open, `ready`
parent) means the work hasn't truly begun at the container level, so a child shown as `in_progress`
under it would over-state progress; surfacing the child as `ready` keeps the displayed lane honest to
the container's gate. (This settles the two-parent `ready`+`in_progress` case: `ready`-mask wins.)

## 3. `parent` (gating) vs `contributes_to` (non-gating) is now load-bearing

The two edges were interchangeable to the derivation under ②c; they no longer are:

- A child under a **`parent`** *rests when its container rests* — it is gated by the ancestor lane
  (§1).
- A loose association that must **not** gate uses **`contributes_to`** (#vf4) — it never appears in
  the ancestor walk, never suppresses, never masks.

**Worked example (the model's acceptance intuition):**

- "Perfume" sits under **both** *birthday* and *shopping* as a `parent`. When *birthday* is deferred,
  Perfume is correctly **hidden** (suppressed by a deferred ancestor).
- "Cream" really belongs to *shopping* and only **contributes to** *birthday* → modeled as
  `parent=shopping`, `contributes_to=birthday`. Deferring *birthday* does **not** hide Cream.

Choosing `parent` vs `contributes_to` is therefore now a semantic decision the author makes, not a
cosmetic one. #07a.4 surfaces this distinction in `prime`/guidance.

## 4. Closed-parent mask → `parent_closed_reason`

When **all** of a still-`open` child's parents are `closed`, the child's effective display lane is
`closed` (masked), and the record carries **`parent_closed_reason`** listing **every** closed parent
as `[{parent_id, reason}]`:

- issue-tracker (single `parent`, `max=1`): one entry.
- personal-todo (`max="many"`): all closed parents, in id order (deterministic).

The `reason` is the parent's `closing_comment`. A child with *some* open parents is not closed-masked
(it is gated by the most-restrictive *open* ancestor per §2); the closed-mask requires **all** parents
closed.

**Resolution — the closed-mask keys on DIRECT parents (implementation, 07a.3).** §2 phrases the
open-ancestor ladder over the full *ancestor* chain, while this section phrases the closed-mask over the
child's *parents*. These are deliberately different scopes, and the implementation makes the split
explicit: **suppression** (a blocked/deferred ancestor) walks the **full transitive `parent` chain**
(§7), but the **closed-mask** keys on the child's **direct parents only** — a child is closed-masked iff
**every direct, live parent is `closed`**. The two never conflict because they partition the space (§2):
if any direct parent is still open, the open-ancestor ladder decides; only when *every* direct parent is
closed does the closed-mask take over. (A blocked grand-ancestor reached *through* a closed parent does
not re-suppress a closed-masked child — the closed-mask, evaluated first, wins; `derive::actionable`
excludes the child either way.) `derive::actionable`'s closed-mask clause and the read-layer
`parent_closed_reason` join both read the direct `parent` edges, so they agree by construction.

**`show` keeps the child's OWN status; only the lane views mask it.** The closed-mask is an
*effective-lane* projection for the enumerating/ranking views (`list`/`next`/`ready`/`closed`) — it
decides which lane a child *groups under*, not what its record says. `show` MUST still render the
child's **own stored status** (e.g. `open`) and surface `parent_closed_reason` alongside it, so the
"this was never actually completed — here is why it now looks closed" signal is preserved. So #07a.3
must NOT emit `status:"closed"` for a closed-masked child in `show`; it emits the real status plus the
reason join, and only the lane views treat the child as effectively closed.

## 5. `parent_closed_reason` is a READ-LAYER JOIN, not a core field

The canonical record is contractually **"exactly the core fields"** — `CanonicalItem`
(`crates/facade/src/record.rs:16-34`), pinned by `canonical_record_has_exactly_the_core_fields`.
`parent_closed_reason` must therefore be surfaced **exactly like the existing `parent` join**
(#916.7, `crates/facade/src/read.rs:288`): appended to the `show` / `list` / `next` **envelope** by
the read layer, **never** added to `CanonicalItem`. This keeps `<cmd> --json`'s canonical item shape
byte-stable and plugin-independent; the coupling lives entirely in the read/derivation layer.

## 6. Stored vs derived split

Only **one** thing in this epic writes state:

- **Claim-up propagation (#07a.1):** claiming a child propagates `in_progress` **up** the full
  parent chain — ancestors *actually become* `in_progress` (a real op-log write, attributed to the
  actor, convergent like any other status set).

Everything else is **pure derivation**, nothing written into the children:

- suppression by a blocked/deferred ancestor (#07a.2),
- the ready-mask and closed-mask + `parent_closed_reason` (#07a.3).

So a child's stored `status` is only ever changed by the user/agent or by claim-up — never silently
"downgraded" to match a parent.

## 7. Determinism

All derivation stays **pure SQL over the materialized views** — convergent, just costlier than
today. The ancestor lane is computed with a **recursive ancestor CTE up the `parent` edges**. The
walk MUST follow the **full `parents_of` set** (every present `parent` edge), **not** the
single-parent `belongs_to` projection (`present_parent`), so a multi-parent child is gated by *all*
its containers. As with every other derived query, the result is a deterministic function of the
op-log; concurrent multi-parent edits converge before the derivation reads them.

**The walk terminates.** The recursive ancestor CTE cannot loop: parent cycles are rejected at the
write seam (`reaches_via_parent`, #sp6.8), and the chain is additionally bounded by the plugin's
hierarchy `max_depth`. A convergence-delivered transient loop is still defended against with a
visited-set/depth bound in the CTE, so the walk is total even on a momentarily inconsistent merge.

## 8. C3 lane partition, restated over EFFECTIVE lanes

C3 (#916.4) established that the lanes partition the live item space as disjoint sets
(`crates/facade/src/read.rs:158`): `archived ⊎ closed ⊎ blocked ⊎ deferred ⊎ ready ⊎ in_progress =
all live`. Under this epic that partition is restated over **effective** lanes: a child's lane is its
**ancestor-coupled** lane (§1/§2), not its raw own-status lane. The partition still holds — every
live item lands in exactly one *effective* lane — but membership is now computed through the ancestor
walk. The lane-ranked `search` grouping (#916.6) and the `prime` snapshot consume the effective
lanes, so a child suppressed/masked by an ancestor groups by its effective lane, not its raw status.

## 9. Replacing sp6.7 ②c

**Module doc (`crates/core/src/derive.rs`, the "## Parenthood is containment, not blocking (sp6.7,
②c)" section).** Rewritten to describe **this** target model — `parent` is gating containment,
`contributes_to` is the non-gating edge — and to point at this spec. Because the gating derivation is
implemented by #07a.1–#07a.5 (not by this spec gate), the rewritten doc **explicitly states** that
the derivation is *still parent-agnostic today* and that the inversion rolls out across those slices.
This keeps the doc honest about the current code while recording the agreed direction.

**The two ②c pinning tests** — `readiness_is_independent_of_parent_count` and
`a_multi_parent_item_blocks_and_defers_on_its_own_state_not_its_parents` — pin the *current*
parent-agnostic derivation, which is **still correct until #07a.2 lands**. This spec gate therefore
**keeps them green** and only reframes their comments as the **pre-gating baseline** that the
implementation slices will invert (e.g. #07a.2 replaces
`readiness_is_independent_of_parent_count` with a suppression assertion). Deleting or inverting them
*here* would either drop coverage of the live behavior or assert gating that is not yet implemented;
both are deferred to the slice that actually changes the behavior, so each behavioral inversion
travels with its own test in the same PR.

## 10. Retroactive semantics

Existing `parent` edges **gain gating meaning the moment the implementation slices ship** — there is
no migration and no opt-in. A board that today has children parented under deferred/blocked
containers will see those children suppressed/masked from that point on. This is **acceptable now**
(boards are small and early), and is called out here so the behavior change is not a surprise. If a
later board needs the old "non-gating containment" for a specific edge, the answer is to model it as
`contributes_to` (§3), not to special-case the derivation.

## 11. Acceptance (the spec gate, #07a.6)

- This document exists under `docs/specs/` and captures: the effective-lane table (§1),
  most-restrictive-wins multi-parent precedence (§2), the `parent`/`contributes_to` distinction
  (§3), the closed-parent mask + `parent_closed_reason` over **all** closed parents (§4), the
  read-layer-join (not core-field) rule (§5), the stored-vs-derived split (§6), determinism over a
  recursive ancestor CTE on the full `parents_of` set (§7), the C3 partition restated over effective
  lanes (§8), and the retroactive note (§10).
- The sp6.7 ②c module doc points at this spec and records the inversion as in-flight across
  #07a.1–.5 (§9), with the two pinning tests kept green as the pre-gating baseline.
- The implementation children #07a.1–#07a.5 are `blocked-by` this node and carry their own
  behavioral tests (each behavioral inversion travels with its test).
