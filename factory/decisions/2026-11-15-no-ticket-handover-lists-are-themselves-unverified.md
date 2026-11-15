# 2026-11-15 · no ticket · A list handed over at closing time is itself unverified — derive it AGAIN from the vocabulary of the gap instead of working through it.

- Decided by: recorded in the project memory (`nxm`, key `handover-lists-are-themselves-unverified`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: process, verification

## The record

When an item hands over a list as it closes ("these N artefacts must move together when X is
closed"), THE LIST ITSELF is unverified — and in this repository it has already been wrong twice
in a row.

PROVEN CASE (2026-08-14/15, during the nexus-chat surface recut): the item that introduced
`reply --escalate` closed by announcing that THREE artefacts would have to move together when the
`summarize` escalation gap was closed — the characterisation test, the doc comment on
`ChatStore::last_reply_escalated`, and the EN+DE clause in `changes/reply-escalate.md`. The
successor item moved all three and reported truthfully that it had moved all three.

THERE WERE FIVE, and the number grew at EVERY inspection:
- The review found a FOURTH: `set_escalated`'s private doc comment, in the very module that
  performed the closure.
- The post-review found a FIFTH: the PUBLIC rustdoc on `route_declared_channel_completion`, whose
  predicate had described a bug ever since.
The corrected list was thus itself still one short — and this memory said "four" in its first
version, under-reporting the very incident it documents. Caught by the branch review on
2026-08-15.

WHY THIS IS ITS OWN CLASS OF ERROR: the implementer was EXACT with respect to the list they were
given. Their report was true. An artefact still survived that tells readers the opposite of the
code. No test, no gate and no careful report catches that — the check "did I work through the
list" cannot, by construction, answer "was the list complete". And as the corrections show, even
"I derived the list once more" does not answer it either.

WHAT TO DO WHEN YOU INHERIT SUCH A LIST: do not work through it — DERIVE IT AGAIN. The cheap route
is a search for the TERM of the gap across `src`, `tests`, `changes/` and `docs/`, then compare the
result with the inherited list. The difference is exactly what would otherwise have stayed behind.
PUBLIC doc comments first: they are the most expensive place to be wrong, because they are a
contract, and in this case they were at the end of the chain.

AND IF YOU WRITE SUCH A LIST: ship the SEARCH you derived it with, not just its result. A list with
its derivation is checkable; one without is an assertion. Better still: hand over no list at all,
but the SEARCH TERMS — that is what the third item in this chain did, and the review confirmed it
explicitly as the right application of the lesson rather than an omission.

Related but not the same: the rule against lifetime- rather than turn-scoped predicates (same
session, four occurrences) concerns the code; this one concerns the HANDOVER. Its code-side twin is
`RESERVED_HANDLE_PREFIX`, where a list was replaced by a CLASS RULE after being wrong for a round —
the same lesson, cast in code.
