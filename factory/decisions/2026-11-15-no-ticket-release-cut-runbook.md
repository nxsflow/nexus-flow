# 2026-11-15 · no ticket · The release runbook: set skip-changelog BEFORE the check finishes — adding it afterwards needs a toggle, a re-run replays the old state.

- Decided by: recorded in the project memory (`nxm`, key `release-cut-runbook`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: release, changelog

## The record

CUTTING A RELEASE, end to end (`docs/specs/release-management.md` §10). Verified against the
workflows and xtask on 2026-09-18.

    cargo xtask version set <v>    # bump + fold changes/*.md into release-notes.json + regen docs
    # commit `chore(release): v<v>`, PR to main, ADD the skip-changelog LABEL, merge
    # WAIT for ci.yml to go green on the MERGE COMMIT, then:
    git tag -a v<v> -m "v<v>" && git push origin v<v>

`version set` is a chain in `xtask/src/main.rs`, not one function: validate fragments ->
enforce the major-bump migration -> enforce the facade-breaking axis -> bump Cargo.toml + lock ->
`changelog::consume` (folds the fragments into `release-notes.json` and DELETES them) ->
`docs::gen`.

WHICH AXIS: a pending fragment carrying `facade: breaking` forces a MINOR.
`enforce_facade_breaking_axis` fail-closes a PATCH bump carrying one, because by the §4.3 contract
a break ships only on the minor axis. Check `changes/*.md` for the marker before choosing the
number.

THE LABEL, AND WHY ITS TIMING COSTS TIME. A release PR DELETES fragments instead of adding one, and
the gate's diff is `git diff --name-only --diff-filter=ACMR {base}...HEAD`
(`xtask/src/changelog.rs::git_changed_paths`) — `ACMR` excludes Deletions, so the PR "touches no
fragment" and `changelog-check` is red BY CONSTRUCTION. Every release PR therefore carries
`skip-changelog`; set it right after `gh pr create`, before the check finishes. Adding it AFTERWARDS
does not fix the run, and RE-RUNNING does not either — a re-run replays the state the run started
with. The label has to be TOGGLED (remove, re-add). That cost the v0.92.0 session about ten minutes.

DO NOT PLAN A SECOND `docs gen`. `version set` regenerates `docs/generated` CORRECTLY, the man
page's version stamp included. This entry said the opposite until 2026-08-31, when v0.77.0 was cut
and the grep showed the new version already in both places; the old advice had been verified back at
0.2.1 and the tool had changed since. The second run is idempotent but costs a full xtask rebuild.
`cargo xtask docs check` afterwards is the evidence you actually need, and it is cheap.

THE WAIT BEFORE THE TAG IS NOT POLITENESS. Since 6j6v.31rs `release.yml` no longer runs the suite a
second time: `.github/scripts/require-green-ci.sh` READS the verdict of the `ci.yml` run for that
exact commit, counting only `push` and `workflow_dispatch` events, and is fail-closed — no green run
for the commit, red release, never a fallback test run. Merging to main produces exactly such a
`push` run, so no dispatch is needed there.

PROMOTION TO STABLE IS A MANUAL OWNER STEP: open the GitHub release, deselect "Set as pre-release",
set "Set as latest". That fires `promote.yml` (`release: types: [released]`), which SERVER-SIDE
COPIES the same bytes and signatures to stable — nothing is rebuilt or re-signed. Since 6j6v.exga
it then opens a FEED PR (branch release-notes/promote-v<v>): main's ruleset allows no direct push.
A workflow-opened PR starts no checks, so CLOSE AND RE-OPEN it, wait for the checks, MERGE it. Only
that merge puts the stable entry on main and refreshes the public changelog (publish-content.yml).
NPM IS STAGED, NOT PUBLISHED (6j6v.c7dd): release.yml's publish-npm runs `npm stage publish`, so each
tag release leaves @nexus-flow/mcp@<v> WAITING on npm. The owner approves it with 2FA (npmjs.com, or
`npm stage list @nexus-flow/mcp` + `npm stage approve <id>`); the job summary says so. Do NOT re-run
publish-npm for a staged-but-unapproved version: it fails, because npm refuses a second copy.

AND IF PROMOTE EVER COMPLAINS "no beta entry for <v> to promote": do NOT flip the GitHub release
first. Add the entry, re-run `version set`, then fire the promotion. That failure is now structurally
prevented — `consume()` ALWAYS writes the beta entry, upserting an empty `items: []` one when there
are no fragments, so "every released version is in the feed" holds from `version set` time, and
`assert_beta_entry()` (read-only, wired into promote.yml as `changelog assert-beta`) refuses early
instead of half-way. It is recorded because the half-promoted state it produced was expensive: on
v0.23.1 the operator had already flipped the release to stable when the first AWS step was skipped,
so GitHub said stable while S3 stable was empty.

A release with no user impact (a re-ship, a pure infra/CI patch) is a legitimate case and needs no
faked fragment.

A NEIGHBOURING TRAP ON THE SAME SURFACE, worth reading before the first push:
[changelog-check-needs-a-commit](2026-11-15-no-ticket-changelog-check-needs-a-commit.md) — the check needs a COMMIT, so a correct fragment sitting in the
working tree reports as missing.

(Absorbed on 2026-09-18: `release-pr-skip-changelog`, `release-no-fragment-promote` and
`release-version-set-docs-gotcha`, which each carried one paragraph of the above and cost three
more lines of the session-start index than the facts needed.)
