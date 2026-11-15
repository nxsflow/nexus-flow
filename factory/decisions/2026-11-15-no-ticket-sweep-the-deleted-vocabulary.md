# 2026-11-15 · no ticket · After a removal you cannot grep for what is gone - run bash xtask/sweep-retired-vocabulary.sh <base-ref>, search the DELETED vocabulary, and compare against the branch base, not HEAD.

- Decided by: recorded in the project memory (`nxm`, key `sweep-the-deleted-vocabulary`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: removal, documentation, verification

## The record

Whoever REMOVES something cannot search for the identifier that no longer exists. The text that
survives is the text describing what USED to be there. So: **search the vocabulary of what was
DELETED, not of what remains.**

During the nexus-chat surface work (August 2026) stale text showed five distinguishable shapes,
and each needed its own search:

1. Field enumerations in brace form — `{thread_id, acted, reason, ...}`.
2. THE SAME enumerations in PROSE — "the four fields `ReplyReceipt` reports".
3. COMPLETENESS CLAIMS — "the ONLY place an app can learn this", "the WHOLE result of the
   forwarding". This shape is found only by searching for the claim, never for the field.
4. A statement that is TRUE but points at a renamed or non-existent test. Neither a search over
   changed identifiers nor one over deleted vocabulary finds it — the text is right, only the
   pointer is dead.
5. THE WORK ORDER ITSELF. A brief to an implementer contradicted itself because one step had been
   updated and the matching "not your job" list had not. The document that says what to do is under
   the same staleness discipline as the code.

THREE BLIND SPOTS WHERE SEARCHES FAIL — they produced the heaviest findings:
- **Files the search never opened.** Mechanisable as `vocabulary ∖ changed files`.
- **Lines DIRECTLY BESIDE edited lines.** A hunk moves line 4; line 3 carries the same untruth on.
- **An addendum that scopes itself too narrowly.** A note saying "read `roles/` as the new path
  below" makes the spot LOOK HANDLED while the actual removal beside it stays undocumented.

AND A TRAP IN THE CLEANUP ITSELF: a pass that compares each file against `HEAD` sees its own
earlier round as "unchanged" and skips it. Compare against the BRANCH BASE, not HEAD — otherwise
you quietly declare your own work to be somebody else's.

A tool for this lives at `xtask/sweep-retired-vocabulary.sh` — explicitly A REVIEW AID AND NOT A
GATE (it exits 0 either way and is not in CI). On its first run it found three places that two
careful manual passes had missed, including a removed flag in LIVE source.

## How to run it (migrated from CLAUDE.md, 2026-11-02)

After a removal of a verb, a flag or a path, run:

    bash xtask/sweep-retired-vocabulary.sh [<base-ref>]

It lists files that still name a retired verb or path and that your change never opened. Every line it prints is for you to judge: a supersession note or a changelog fragment names a retired term correctly. Add your own removal to its vocabulary list; that list, and the exclusions with their reasons, are the whole content of the file. Its header also records the two shapes it cannot see (lines ADJACENT to lines you edited, and files INSIDE your own change set) and how to sweep for those by hand. Why it matters: prose is the surface an agent reads, so a help page or a system prompt naming a verb that no longer exists is a false map, not a cosmetic defect.
