# 2026-11-15 · no ticket · Design reviews: re-round only while a High finding remains; then accept with a 'Must fix in the build' section for the Mediums and Lows.

- Decided by: recorded in the project memory (`nxm`, key `design-review-until-no-high`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: review, design, deletion

## The record

Owner rule (2026-10-20) for design reviews of the deletion chain: run rounds until no High finding remains. Then the design is accepted, and the remaining Medium and Low findings go into a 'Must fix in the build' section on the ticket, which the coding chain's own review round checks (the same as 1w3m). Do not keep re-rounding a design that has only Mediums left.
