# 2026-11-15 · no ticket · If you hand out work and pass on the reports, you become the author of every claim you let through — a claim you cannot re-run is not evidence.

- Decided by: recorded in the project memory (`nxm`, key `a-relayed-claim-becomes-yours`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: process, verification

## The record

Whoever hands work to implementers and passes their reports on becomes the AUTHOR of every claim
they let through. **A claim you cannot re-run is not evidence — and one you pass on becomes your
own.**

Three times during the nexus-chat surface work (August 2026), every time from the coordinating
side:

- An implementer reported that a cleanup sweep was "mechanised and checks itself". The
  coordinator declared that the best result of the round and reported it onward — without looking.
  It did not exist: a one-off shell pipeline described in prose, with no recorded vocabulary list,
  not repeatable. (In the following round the tool was actually built, and it immediately found
  three missed places.)
- A reviewer wrote that a certain scenario was "never built". Passed on without checking. A
  pre-existing test did build it; the finding was real, but considerably NARROWER than described.
- A number from an older log entry ("the known five lines") was carried into a work order without
  re-running it against the current state. It was one.

THE SECOND HALF OF THE SAME DISCIPLINE: A PRESCRIBED FIX IS A HYPOTHESIS, NOT AN INSTRUCTION.
Twice a solution dictated by the coordinating side was wrong, and only mutation showed it:
- An assertion pinned the STATE a predicate reads instead of the predicate FIRING.
- A proposed discriminator would have opened exactly the bug the implementer then covered on their
  own — their simpler anchor was STRONGER, not merely equivalent.

So prescribed corrections go out as a hypothesis, with an explicit invitation to test them and
report counter-findings rather than route around them. An implementer who refutes the coordinator
is the procedure working.
