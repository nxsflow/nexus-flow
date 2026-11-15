# 2026-11-15 · no ticket · A single CI runtime here is noise (259-744 s on full cache hits) — never tune save-if; the cache sweeper deletes on ABSENCE in its branch arm.

- Decided by: recorded in the project memory (`nxm`, key `ci-cache-and-runtime-noise`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: ci, cache

## The record

MEASURED 2026-09-04 over the logs of 32 `ci.yml` runs and the cache API, while fixing a repository
Actions cache that was evicting itself; sharpened afterwards in the review of that pull request.
Three things one otherwise gets wrong.

## 1. A SINGLE CI RUNTIME IS NOT A SIGNAL HERE

`Tests (debug)` in `quality-gates`, 30 successful runs in the same window:

    min 259 s   median 285 s   max 744 s   factor 2.9

and ALL THIRTY had a full cache hit. So the spread is NOT the cache; it is runner noise on shared
`ubuntu-latest` machines. Comparing two runs and deriving a decision from them is comparing noise.
This matters because this project decides an unusual amount on measured runtimes (dropping nextest
over 17 s, the macOS arrangement, the rule that CI is the gate). Anyone deciding such a thing anew
needs SEVERAL runs and the median, not one run.

## 2. THE CACHE HIT RATE IS HIGH — DO NOT TOUCH save-if

209 hits, 1 miss, every hit "full match: true" (so no re-saving). Anyone wanting to shrink the
Actions cache must NOT put `save-if:` or `cache-targets: false` on the CI lanes: there is no hit
rate to repair, and it costs. Measured, `changelog-check` would go from 40 s to 93 s per subsequent
run.

This count is kept CANONICALLY in the header comment of `.github/workflows/cache-sweep.yml`. It
stands here as well because a memory has to stand on its own — at session start that file is not
open. Whoever re-measures changes BOTH places.

The space comes from the sweeper instead: `.github/workflows/cache-sweep.yml` runs every six hours
and deletes what NO future run can read any more. The rule lives as a pure, unit-tested function in
`cargo xtask cache sweep` (`xtask/src/cache.rs`), not in the YAML. It depends solely on an entry's
`ref`: a deleted branch, the merge ref of a closed PR, a superseded release tag. `refs/heads/main`
and the newest tag always stay.

Effect on the first run: 32 entries / 10.67 GB -> 15 entries / 5.17 GB, against a hard limit of
10 GB per repository.

IF OCCUPANCY CLIMBS AGAIN, the first question is NOT "save less" but whether the sweeper is running
(`gh run list --workflow=cache-sweep.yml`).

And the second question needs a precision the review forced: the sweeper does NOT fall back to KEEP
throughout. Unknown ref shapes, an unreadable PR number and a missing newest tag — yes, those stay.
But the BRANCH arm DELETES ON ABSENCE: it removes `refs/heads/<b>` because `<b>` is not in the
branch list, and absence has two causes ("gone" and "not delivered"). That is why the list comes
from `git ls-remote --heads` (an unpaginated ref advertisement in which no page can be missing and
no namespace can hide) and NOT from `gh api .../branches`. Reverting that source turns it into a
deletion rule with an unknown basis.

Likewise a bet and not a guarantee: "the newest release tag stays, older ones do not". A tag is not
deleted when it is superseded, so it stays checkoutable; a re-run of an old release is possible,
just unlikely. Taken deliberately, because being wrong costs one cold rebuild — but it is not a
reachability fact like the other arms.

## 3. `actions/cache/usage` LIES IN THE SHORT TERM

The aggregate endpoint is eventually consistent. After a run that had demonstrably deleted 17 of 32
entries, it kept answering "32 entries, 10.67 GB" for FOUR MINUTES; the entry list `actions/caches`
said 15 / 5.17 GB immediately, and the aggregate only agreed from the sixth minute. To prove a
deletion, sum the LIST, not the aggregate — otherwise you quote exactly the number that proves
nothing happened.
