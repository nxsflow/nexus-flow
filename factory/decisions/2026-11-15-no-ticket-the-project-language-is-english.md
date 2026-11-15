# 2026-11-15 · no ticket · Write everything down in English — tickets, memories, commits, PRs, code, docs; speak to a human in their language, but never write it down.

- Decided by: recorded in the project memory (`nxm`, key `the-project-language-is-english`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: language, conventions

## The record

The project language is English. Everything this repository writes down and keeps is English:
ticket titles and descriptions, memories, commit subjects and bodies, PR titles and descriptions,
issues, code, comments and documentation.

THE ONE EXCEPTION IS THE LIVE CONVERSATION. An agent talks to a human in that human's own language
— the owner works in German, and answering him in English would be worse, not more correct. The
exception ends the moment something is written down: what goes onto the board, into a memory, into
a commit or a PR is English, no matter which language produced it. That is the boundary that used
to blur — German conversations produced German tickets by drift, never by decision.

VERBATIM STAYS VERBATIM. Code identifiers, file paths, CLI flags and commands, proper names and
established technical terms are never translated. Without this exception a language rule makes
things worse, because it invites translating the very names that have to match the code.

TWO SURFACES ARE BILINGUAL ON PURPOSE and stay that way: the user guide (`docs/guide/de` and `/en`
in every guide-carrying crate) and the changelog fragments under `changes/` (EN and DE per
fragment, gated by `changelog-check`). That is a product decision for German-speaking users, not
sloppiness. Nobody "cleans it up" by deleting the German half.

THE HISTORY STAYS GERMAN, AND THAT WAS DECIDED. Commits and PRs written before this rule are German
and remain so. Keeping the history was decided on 2026-07-12, after weighing a full history rewrite
before going public: rewriting messages changes every commit id, and consumers pin git tags —
engine-bridge and agent-runtime in app-foundations, `Cargo.lock` in three repos, over a hundred
tags in this repo including the first stable, plus the changelog's PR references. Translating the
history retroactively is the same operation as discarding it, at the same cost. English applies
from this rule forward, and CONTRIBUTING says so in one sentence.

OPEN TICKETS AND MEMORIES ARE ENGLISH FROM HERE ON. Anything created from now on is English. An
open item you touch — editing a field, adding a note — moves over while you are there. Closed items
are historical record and stay as they are. (The durable memories were migrated wholesale on
2026-09-05: every surviving entry is English, body and introduction.)

WHY IT MATTERS NOW: this repository goes public. A newcomer reads the board, the memories and the
log; German artifacts either turn them away or hide the reasoning that makes this project worth
reading in the first place.

THE ENFORCEMENT IS MEANT TO LIVE IN CONFIG, NOT IN WILLPOWER: an open board item designs a
`ticket_language` setting — an imperative directive in prime, the same directive at the write
point, and a corrective post-op check, with the verbatim exception mandatory. Until that ships,
this memory carries it.
