# 2026-11-15 · no ticket · A gate result proves EXACTLY the tree it ran against — HEAD plus the working copy; a run that raced anything proves nothing, green or red.

- Decided by: recorded in the project memory (`nxm`, key `gate-evidence-is-per-commit`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: gates, verification

## The record

A gate result (tests, lint, changelog check) proves EXACTLY the tree it ran against — and "the
tree" is HEAD **plus the working copy**, not HEAD alone.

During the nexus-chat surface work (August 2026) SIX independent gate runs were lost, and each
loss had its own shape:

- One run was raced by a concurrent second `cargo test` (the documented env-var race class).
- One ran while an implementer was editing sources — the binary-freshness guard tore it down.
- One was overtaken by a commit: HEAD moved in the middle of the suite.
- One reported 24 failures that were pure artefacts.
- One `changelog check ok` was TRUE AND WORTHLESS: the gate reads `git diff <base>...HEAD`, so only
  committed state, and the fragment under test was in the second-to-last commit of the range. The
  green run happened BEFORE the fragment was committed and never saw it.
- One run had a guard that compared only HEAD — the implementer edited the working tree without
  committing, so HEAD stayed equal and the guard said nothing.

PRACTICAL CONSEQUENCE, in both directions:
- Before starting a gate run, check that no implementer is holding the tree.
- Before commissioning work that changes the tree, check that no gate run is in flight.
- The run logs HEAD AND the number of dirty files at the start and at the end, and PRINTS ITS OWN
  VERDICT (VALID / VOID), rather than leaving two numbers for somebody to compare who never will.
- A gate runs ONCE, at the end, against the state you are reporting — not behind every commit.
  That is both the expensive form and the only probative one.

A red run out of a race is NOT evidence of anything — reading it as a finding is the same mistake
as believing a green that checked nothing.
