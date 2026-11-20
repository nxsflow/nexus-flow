# 2026-11-20 · NEX-72 · The owner-rules list in GENESIS must follow its marker directly, with only blank lines between

- Decided by: Carsten
- Why: a deleted list then leaves the gate with no readable rules, so every pull request goes to the owner, instead of some later list in GENESIS being read as the rules; this narrows RFC 0004's "the first list after the mark" (RFC 0010, "The rules from GENESIS"). Answer (a) in NEX-77.
- Instead of: the first list anywhere after the marker.
- Topics: merge-gate, genesis, owner
