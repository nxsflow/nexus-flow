# 2026-11-15 · no ticket · A measurement pasted into a ticket is the TICKET'S evidence, not yours — re-run it before any removal, and hold when it comes out otherwise.

- Decided by: recorded in the project memory (`nxm`, key `a-tickets-own-measurement-is-unverified`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: verification, removal

## The record

A MEASUREMENT WRITTEN INTO A TICKET IS EVIDENCE THE TICKET CARRIES, NOT EVIDENCE YOU HAVE. Re-run
it before you act on it — and before a REMOVAL, always, because a removal has no identifier left to
grep for afterwards.

PROVEN CASE (2026-09-05). A chat item had been re-cut from a decision into a chore on the strength
of a measurement taken on 2026-08-25 and pasted into its description, layer by layer:

    read_cursors   only consumer: ChatStore::inbox (store.rs:853)

That line is why the item said mark_read "advances a cursor nothing evaluates any more" and why
removing the whole unread apparatus looked free. It was wrong. ChatStore::read_cursor_seen was a
second reader, and it fed ChatStore::opener_wake -> PrimeReport::wake -> prime --json's
threads_you_opened — the app-seam record that a completed board left EXACTLY when the cursor passed
its completing reply. Strike the table and no board could ever leave that list again.

WHY THE ORIGINAL MEASUREMENT MISSED IT, because the lesson is in the mechanism and not in "be more
careful": a grep for the TABLE name finds both readers. What the measurement did was stop at the
first consumer it could explain. inbox was the obvious one, it accounted for the symptom the item
was about, and the trace ended there instead of following EVERY accessor's own callers one level
further. A named getter beside the table is the shape that hides: it reads like a helper, not like
a consumer.

THE CHEAP CHECK THAT WOULD HAVE CAUGHT IT, and it needs no judgement:

  1. For each thing to be removed, list its ACCESSORS (functions that touch it), not its textual
     hits. Then list each accessor's own callers, transitively, until you reach a public surface.
  2. Read the ticket AGAINST ITSELF. A removal ticket usually carries two lists — "what is removed"
     and "what stays untouched". This one had both, and its own "stays untouched" list named
     facade::opener_wake, which read something on the removal list. A ticket that contradicts
     itself has not finished being designed, and that is visible without opening the code.

WHAT TO DO WHEN THE RE-RUN COMES OUT OTHERWISE: hold, report, and leave the finding as a RUN rather
than a paragraph — a test that goes red the moment somebody attempts the removal, so the next
attempt is told what it costs instead of discovering it in a consumer. Then correct the claim
wherever the repository repeats it: the false sentence had also been copied into the gate
read_surface.rs, where it read as settled fact.

HOW IT ENDED, and the ending is the other half of the lesson (2026-09-08, nxf 6j6v.4d2z). The
removal landed three days later, and NOT because anyone re-argued the measurement: the consumer the
gate found was REPLACED first. nxf 6j6v.2hx9 derived the session-start notice from the OPERATION —
what finished since the caller's previous session ended — so the cursor lost its second reader by
being made unnecessary rather than by being declared unimportant. Only then did read_cursors,
mark_read, facade::inbox and the PrimeReport catch-up fields go, in one change, with the four-surface
gate re-run first and green. So the holding pattern is: hold, pin the finding as a run, build the
replacement, re-measure, then remove. The pinning file was retired WITH the apparatus it guarded,
which is correct — a gate whose subject is gone is itself a false map — and the account of what it
held now lives in read_surface.rs's module doc.

THE DISTINCTION THAT MAKES THE FINDING USABLE: that measurement was right about one thing and wrong
about the other, and the two are easy to say in one breath. NOBODY CALLS mark_read (true, measured
over five layers). NOTHING READS THE CURSOR (false at the time). Unused is not unread. Only the
second would have made the removal free; the first is an argument for giving the write a caller —
and when nobody wanted to write that caller either, it became the argument for removing the write.

RELATED, AND NOT THE SAME: [handover-lists-are-themselves-unverified](2026-11-15-no-ticket-handover-lists-are-themselves-unverified.md) is about a list handed over
at CLOSING time ("these N artefacts must move together"). This one is about a measurement embedded
in an OPEN ticket as its justification. Same family — an inherited claim becomes yours the moment
you act on it, see [a-relayed-claim-becomes-yours](2026-11-15-no-ticket-a-relayed-claim-becomes-yours.md) — different artefact, and this one is more
dangerous because it reads as data rather than as a reminder.
