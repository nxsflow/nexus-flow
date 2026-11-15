# 2026-11-15 · no ticket · TDD for all behavior, --json everywhere, and every new CLI verb needs a counterpart on the library seam — a gate enforces that parity.

- Decided by: recorded in the project memory (`nxm`, key `conventions-patterns`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: conventions, testing, cli, changelog

## The record

- **TDD for all behavior** (test-first); scaffolding and config (Cargo.toml, release profile) are
  exempt.
- **CLI contract:** `--json` everywhere, deterministic output (agent ergonomics is the measuring
  stick).
- **A new CLI verb needs a counterpart on the library seam.** One shared core, with the CLI and an
  app over it as equals, is specified per module (the embedding-host API specs for flow, memory and
  chat) and now *enforced*: the verb-seam gate (`crates/{cli,memory,chat}/tests/verb_seam.rs`,
  engine in `crates/test-support`) walks each clap tree and fails on any verb with no matching
  public symbol on that module's `Engine`/facade. Both lists are derived, so the gate cannot drift;
  what you declare is only the difference, and every entry carries a reason — `CliOnly` (host-own:
  `init`, `agent-manifest`, `self-update`, import/onboarding), `Alias` (on the seam under another
  name — the gate checks the target exists), or `KnownGap`, which names a board item. This is
  precisely what the three parity differentials structurally cannot see: they compare what exists on
  both seams and are blind to a verb that exists on only one.
- **WHAT `KnownGap` ACTUALLY MEANS TODAY, measured 2026-09-18** — this entry used to say "a real
  omission, with the board item that tracks it. A gap without a ticket is not a waiver", and that
  is not what the one live entry is. There is exactly ONE `KnownGap` in the repository: chat's
  `tick`, naming `6j6v.dvyq`, which is CLOSED — closed because it recorded a DECISION ("NO SEAM
  TWIN BY DECISION, not by omission"), not because anybody forgot it. So the word carries two
  states, an omission somebody owes and a settled question, and only opening the item tells them
  apart. Do not read a closed item behind a `KnownGap` as rot, and do not tighten the rule to "the
  item must be open" — it would fail the only correct entry. 6j6v.pqww carries the fix (a
  `ByDecision` variant, so the gate can enforce what this rule claims). Until it lands: when you
  declare a `KnownGap`, name an OPEN item, and say in one line whether you mean an omission or a
  decision.
- **Every PR with user impact needs a changelog fragment**: a `changes/<slug>.md` with EN **and** DE
  body sections and frontmatter `type: added|changed|fixed|removed`. The `changelog-check` CI gate
  enforces it on the PR and is fail-closed; validate locally with
  `cargo xtask changelog check origin/main`. Only PRs with **no** user impact (internal infra/CI,
  docs, tooling) opt out: one that touches only never-shipped paths passes by itself (6j6v.c37e),
  anything else gets the `skip-changelog` label instead of a faked fragment. See
  `docs/specs/release-management.md` §4.2, `changes/README.md`, and [changelog-scope](2026-11-15-no-ticket-changelog-scope.md) for what
  counts as impact.
- **The `crates/facade` public API is a SemVer contract** (§4.3-§4.5): a **patch never breaks it**
  (no removed/renamed/signature-changed `pub` items) — a break belongs on a **minor** (the break
  axis in 0.x). The `facade-semver` CI gate (`cargo xtask facade semver-check`, cargo-semver-checks)
  enforces it fail-closed on a patch bump; flag a facade-touching fragment with
  `facade: changed|breaking` (+ `security: true` for a security fix). Security fixes backport to
  the current + 2 previous minor lines — see `docs/specs/maintenance-and-backports.md`.
