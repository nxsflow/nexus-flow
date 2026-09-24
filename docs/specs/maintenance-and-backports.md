# Maintenance lines & security backports — runbook

> Implements the security-backport policy of `release-management.md` §4.5 (N-2, grace ~6 weeks)
> and the consumer patch-lane of §4.4. This is the operational mechanics; the policy and the
> SemVer contract it rests on live in `release-management.md` §4.3–§4.5.

## 1. What this enables

Today's release model is fix-forward on a single line (tags on `main`, "stable skips the broken
version", §10/§11). That cannot serve a consumer who pins a minor and wants "security daily, no
unplanned minor jumps" (§4.4 patch lane). This runbook adds **maintenance lines**: a security fix
ships as a **patch `x.Y.z+1` on its own `release/x.Y` branch**, so a pinned consumer adopts it on
the patch lane without a breaking minor jump.

## 2. Active lines (N-2) and the grace window

- **N-2:** the **current + 2 previous** minor lines are security-supported — up to **3 active
  lines** at once (§4.5).
- **Grace window: ~6 weeks, calendar-based** after a successor minor is promoted to stable. When
  `x.(Y+1).0` is promoted, line `x.(Y-2)` falls out of the N-2 window; keep it security-supported
  for ~6 more weeks, then **retire** it (§5 below). No open-ended LTS in 0.x.

Example with current `0.8`: active lines are `0.8` (current), `0.7`, `0.6`. When `0.9.0` is
promoted, `0.6` enters its ~6-week grace, then retires; `0.9`/`0.8`/`0.7` become the N-2 set.

## 3. Branch-cut timing (decision)

**A `release/x.Y` branch is cut lazily — at the moment the line first needs a backport** (or when
`x.(Y+1).0` is promoted and `x.Y` becomes a *previous* line), from the line's highest released tag
`vx.Y.z`. Rationale: the current line releases from `main` exactly as today (no branch, no change
to the proven main-tag path); a `release/x.Y` branch exists only for lines that are no longer
`main`'s tip and may receive a backport. Cutting from the tag (not a floating point) keeps the
line's build reproducible.

```bash
# Cut the maintenance branch for the 0.6 line from its latest tag:
git fetch --tags
git branch release/0.6 v0.6.2
git push origin release/0.6
```

**The branch name is load-bearing: exactly `release/<major>.<minor>`, digits only.** Since
6j6v.31rs the three gates below trigger on `release/[0-9]+.[0-9]+`, not on `release/**` — the wide
glob also caught the `release/v<X.Y.Z>` RELEASE-PREPARATION branches and ran the whole gate twice on
every version-bump PR. `promote.yml` already validated the same shape (`^release/[0-9]+\.[0-9]+$`),
so the two now agree on what a maintenance line is; a typo (`release/v0.6`, `release/0.6.0`) is a
branch the gates ignore. **After the first push, confirm the runs actually appeared** — a name that
misses the pattern fails silently, by producing nothing:

```bash
gh run list --branch release/0.6 --limit 10
```

Once pushed, `ci.yml`, `version-check.yml`, and `facade-semver.yml` run on the line, so it gets the
**same gates as main**, including the derivation-parity gate (`cargo test --all` + the
postgres-parity job) — the §4.5 requirement that parity stays green **per line**. That includes the
**release profile**: the `(release)` test steps are skipped on the pull-request event only
(6j6v.6e0e), so every push to a maintenance line runs both profiles, exactly as `main` does. And
that push run is what the release's own `ci-green` gate reads when the line is tagged (spec §9).

## 4. Backporting a security fix to all active lines

For each active line `x.Y` (current first, then the previous ones):

1. **Land the fix on `main`** first (normal PR + changelog fragment with `security: true`, and
   `facade: …` if it touches the facade API — `changes/README.md`). `main` is always fixed
   forward.
2. **Forward-port to the line.** Branch off `release/x.Y`, cherry-pick the fix
   (`git cherry-pick -x <sha>`), resolve any drift, open a PR **targeting `release/x.Y`**. The
   per-line gates run on the PR.
3. **Bump + fold the notes on the line.** On the line branch:
   ```bash
   cargo xtask version set x.Y.<z+1>   # bumps the line's workspace version + folds changes/ → feed (beta ring)
   ```
   `version set` is line-agnostic — it operates on the checked-out branch's `Cargo.toml` +
   `release-notes.json`, so it bumps **the line**, not main. The facade-axis gate still applies:
   a `facade: breaking` fragment on a patch bump is rejected (§4.3) — a backport is a patch, so it
   must not break the facade (which N-2 guarantees it never needs to).
4. **Tag the line.** `git tag -a vx.Y.<z+1> -m "vx.Y.<z+1>" && git push origin vx.Y.<z+1>`.
   `release.yml` triggers on `v*` **from any branch**, checks out the tag, builds the 4 targets,
   and publishes the **beta ring** for that version — no change needed for maintenance lines.
   `version-check` enforces `tag == workspace version` on the line.
5. **Promote beta → stable.** Flip the GitHub pre-release to "latest". `promote.yml` reads the
   release's `target_commitish`; because it is `release/x.Y`, it checks out **that line**,
   aggregates within the line (`prev-stable` returns the line predecessor `vx.Y.z`, not a
   higher-line stable — proven by the `backport_promotion_is_line_scoped_across_parallel_lines`
   test), and copies the same bytes beta→stable (build-once-promote, per line).
6. **Forward-port the line's feed entry to `main`.** `promote.yml` opens a PR carrying the
   regenerated feed against the **line** branch (release-management.md §5.5 step 5: close and
   re-open it so its checks run, then merge). To surface the backport in the canonical feed + the public
   changelog (which build from `main`'s `release-notes.json`), open a small PR cherry-picking that
   `release-notes.json` change onto `main`. Merging it triggers `site.yml` and refreshes the
   public site. (The canonical feed on `main` is the **union** of all lines' entries; backport
   entries reach it via this step.)

Repeat 2–6 for each active line that is exposed to the vulnerability.

## 5. Retiring a line (grace-window expiry)

~6 weeks after a line leaves the N-2 window (§2), retire it:

- Stop backporting to it; announce the line as end-of-life in the release notes.
- Leave `release/x.Y` in place (immutable history) but expect no further tags on it.
- Already-published artifacts stay available (immutable, versioned, §11); only **new** security
  fixes stop landing on the retired line. A consumer still pinned there must move to an active
  minor (the human-gated minor lane, §4.4).

## 6. Open design decisions (resolved here)

| Question | Decision |
|---|---|
| **Branch-cut timing** | Lazy: cut `release/x.Y` from `vx.Y.z` when the line first needs a backport / drops off `main`'s tip. The current line keeps releasing from `main` unchanged (§3). |
| **`release-notes.json` shape with parallel lines** | The feed on `main` is the **union** of all lines' entries (the channel arrays already hold any versions). Each `release/x.Y` carries its line's feed; `promote.yml` is line-aware (commits to the line); backport entries are forward-ported to `main` (§4.6). `prev-stable`/`aggregate`/`promote` are line-correct by construction (strictly-below picks the per-line predecessor; monotonic per-line versions) — locked by the parallel-line test. |
| **`cargo-semver-checks` gate on backport patches** | Works per line: on `release/x.Y` the baseline is the highest tag ≤ the line version = the line predecessor `vx.Y.z` (newer-line tags are above it and ignored — `select_baseline`/`baseline_on_a_backport_line_ignores_newer_lines`). A backport is a patch, so the gate is **fail-closed**: the facade may not break on a backport. |
| **GHCR image retag per line** | The container image (spec §5.6, `:beta`→`:stable` retag) is **not yet wired into the live pipeline** — when it lands, it follows the same per-line pattern: a backport tag `vx.Y.z+1` produces `:x.Y.z+1` + `:beta`, and promotion retags `:stable` for that line (no `:latest` move for a non-current line). Tracked separately; no change here. |

## 7. No regress on the main-tag path

Every pipeline change is guarded so the **main** release/promotion path is byte-identical to
before this runbook:

- `release.yml` already triggered on `v*` from any branch — unchanged.
- `promote.yml` redirects **only** when `target_commitish` starts with `release/`; a `main`
  release (`target_commitish: main`) takes the exact prior path (`ref: main`, push `HEAD:main`).
- The CI/version/facade-semver triggers **add** `release/<major>.<minor>` (narrowed from
  `release/**` by 6j6v.31rs — see §3); the `main` and PR triggers are unchanged.

Exercise maintenance-line changes via the `release.yml` `workflow_dispatch` staging rehearsal
before the first real backport (spec §5.1, §12.8). On a branch that is not `main` or a
`release/<major>.<minor>` line, that rehearsal takes **two** dispatches — `ci.yml` first, then
`release.yml` — see the runbook, spec §10.1.
