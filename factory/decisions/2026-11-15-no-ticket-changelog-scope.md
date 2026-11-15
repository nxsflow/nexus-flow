# 2026-11-15 · no ticket · A changelog fragment is required by PRODUCT IMPACT, not by 'touches the CLI' — and none is written for a fix to something that never shipped publicly.

- Decided by: recorded in the project memory (`nxm`, key `changelog-scope`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: changelog, release

## The record

Changelog fragments (`changes/*.md`) feed `release-notes.json` and the self-update notes.

SCOPE (set 2026-06-18 by the owner, superseding an earlier CLI-only rule): the measuring stick is
**PRODUCT IMPACT**, not "touches the CLI". A fragment is required for

  (a) user-facing CLI changes, AND
  (b) notable engine/library capabilities of the product — for example the embedding-host API in
      the `nexus-flow-facade` crate.

Rationale: a deliberately shipped feature (a whole epic) must not vanish from the release feed just
because it is not CLI surface. The earlier rule — "the CLI is the only thing that needs a
changelog" — held until an entire embedding-host epic closed with no headline entry;
`docs/specs/release-management.md` §4.3 now states the product-impact scope explicitly.

TWO EXEMPTIONS, and neither is served by a faked fragment. Where the PR touches ONLY paths that never
ship (the `NO_PRODUCT_IMPACT` allowlist in `xtask/src/changelog.rs`: docs/ minus
docs/architecture and docs/generated, CI workflows minus every workflow that can publish (a test
keeps that list in step), tests, xtask minus the gate's own changelog.rs/facade.rs, repo prose incl.
README.md, which rides in the tarball but is prose), the gate passes BY ITSELF since 6j6v.c37e — needed because a fork contributor
cannot set a label. Everything else takes the `skip-changelog` LABEL, set by a maintainer:

1. NO PRODUCT IMPACT: the landing site (`site/**`: robots/sitemap/SEO/copy/styling), infra and CI,
   docs, internal refactors, and purely internal performance work with zero consumer-visible
   behaviour change.
2. NEVER PUBLICLY SHIPPED (owner, 2026-06-21): a fix that only affects a version which never
   shipped publicly or as stable — a behaviour that only ever ran under a beta or pre-release tag.
   The user never saw the change, so there is nothing to tell them. Worked example: a fix to the
   keyed-map form of the priorities config, whose array predecessor had only ever run in one 0.5.0
   pre-release and never reached the stable feed.

See [conventions-patterns](2026-11-15-no-ticket-conventions-patterns.md) for where this sits among the other PR rules, and
[release-cut-runbook](2026-11-15-no-ticket-release-cut-runbook.md) for why a RELEASE PR is red by construction and needs the same label for a
third, purely mechanical reason.

(Absorbed `changelog-skip-unreleased` on 2026-09-18 — it was exemption 2 above, in a memory of its
own, linked from here as "the second exemption".)
