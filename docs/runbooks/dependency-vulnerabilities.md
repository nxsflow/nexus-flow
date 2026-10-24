# Runbook: a vulnerability in a dependency

nexus-flow ships software that links other people's code: the Rust crates in `Cargo.lock`, and
the npm packages bundled into the agent sidecar, which is compiled into every `nxs` binary. When
an advisory is published against one of them, this runbook says who learns of it, how fast it is
answered, how a decision not to fix it yet is recorded, and what is kept as evidence
(6j6v.2dn0, part B-2 of the product liability epic 6j6v.t8we).

**The rule in one line: every advisory against a shipped dependency is either fixed or recorded
as a dated exception. Nothing is silenced in bulk, and nothing is silenced for good.**

## What the gate checks

The gate is one command, `cargo xtask deps audit` (`xtask/src/deps.rs`), and it runs in two
places:

- as `ci.yml`'s `dependency-audit` job, on every push, pull request and dispatch;
- in `.github/workflows/dependency-audit.yml`, **daily at 05:23 UTC**, because an advisory is
  published without anybody touching the tree.

It runs every check below, even after one has failed, and is red when any of them fails:

| Check | What it refuses |
|---|---|
| `cargo deny check` (config: `deny.toml`) | A RustSec advisory (vulnerability, unsound, unmaintained) or a yanked version, against any crate a shipped artifact links, under every feature. Dev-dependencies are excluded: they build tests, not artifacts. Also: a license outside the allow-list, `openssl-sys` in the graph, a crate from an unknown registry or git source. |
| deny.toml exceptions (`cargo xtask deps exceptions` on its own) | An ignore entry in `deny.toml` whose reason does not start with `expires YYYY-MM-DD:`, names no board item, or whose date has passed. |
| npm audit `agent-sidecar` (`cargo xtask deps npm-audit agent-sidecar` on its own) | Any `npm audit --omit=dev` advisory against the sidecar's production dependencies that `agent-sidecar/npm-audit.toml` does not record, or an invalid or expired entry there. An audit that could not reach the registry is red too. |
| npm audit `npm/mcp` | The `@nexus-flow/mcp` runner shim gaining a runtime dependency without a lockfile to audit. Today it has none. |

Out of scope, with the reason: `content/` (the website's content provider, never shipped to a
user) and the npm dev-dependencies (esbuild builds the bundle but is not in it).

Because `require-green-ci.sh` refuses to tag a commit whose `ci.yml` run is not green, **an open
advisory also blocks a release** until it is fixed or recorded. That is intended: a release never
ships an advisory nobody decided on.

Run the same gate locally from the repository root (it needs `cargo-deny` and `npm`):

```bash
cargo xtask deps audit       # brew install cargo-deny, or cargo install cargo-deny
```

## Who is notified

> **Proposed, owner to confirm.**

1. **The owner**, through GitHub. A failed scheduled run notifies the account that last edited the
   `cron:` line of `dependency-audit.yml`; a failed `ci.yml` run notifies whoever pushed. The
   owner's GitHub notification setting for Actions must be "failed workflows only" or wider, by
   e-mail, so the daily run reaches a person and not only a tab.
2. **The PM** of this repository, by the owner or by whoever reads the red run first. The PM
   opens the board item (below) and assigns the work.

CI does not open a GitHub issue by itself. The repository is public, and an issue that says "the
shipped binary is affected by X" before the fixing release exists would announce the hole.
Discussion of an advisory that affects users goes into a private GitHub security advisory on the
repository until the fixing release is out.

## Response time per severity

> **Owner-confirmed (2026-10-24).** The clock starts when the gate first goes red on the advisory
> (the run's timestamp is the evidence).

| Severity | Response |
|---|---|
| **Critical** | Within **72 hours**: a fixed release, or a documented mitigation (what is affected, why users are not exposed, or what they must do) recorded on the board and in the exception. |
| **High** | Within **14 days**: a fixed release. |
| **Medium** | With the **next regular release**. |
| **Low** | **Recorded**, no deadline: fixed when convenient, or a dated exception. |

The severity is the advisory's own (GHSA severity for npm, the CVSS score of a RustSec
advisory). A RustSec **unmaintained** or **unsound** notice without a CVSS score counts as
**low** unless the reading below finds the unsound path reachable in our use; then it is
raised. Severity may be **raised** for our use, never lowered, and the reason goes on the item.

## What to do when the gate is red

1. **Open a board item**, one per advisory (or per crate when several advisories land on one
   crate at once): `nxf create --type bug --title "<advisory id>: <crate> <what it allows>"
   --priority <P0 critical, P1 high, P2 medium, P3 low>`. Paste the advisory id, its URL and the
   red run's URL into the description.
2. **Read whether we are exposed.** Which artifact links the crate, under which feature
   (`cargo tree -i <crate> --all-features -e features`), and whether our code reaches the affected
   path. Write the answer as a note on the item.
3. **Fix it, if a fixed version exists.** In this order of preference:
   - `cargo update -p <crate>@<version>` (stays inside the declared semver range; no manifest
     change);
   - a manifest change that drops the path (for example a feature that pulls in a legacy stack:
     2dn0 removed rustls 0.21 and h2 0.3 by turning off `aws-sdk-dynamodb`'s deprecated `rustls`
     feature);
   - a version bump of the direct dependency, with its tests;
   - for npm: `npm audit fix` (in-range only, never `--force` without reading what it bumps),
     then `npm run build` and the bundle smoke step of `ci.yml`.
   The fix goes through the normal chain like any other ticket. A fix for a critical or high
   advisory ships as a release marked as a security release (6j6v.qs7j).
4. **Otherwise, record an exception.** Only when no fixed version exists, or the fix needs work
   that cannot land within the response time. See the next section.

## How an exception is recorded

An exception is one entry, in the file the gate reads, in this exact shape:

```toml
# deny.toml, under [advisories]
ignore = [
    { id = "RUSTSEC-2024-0449", reason = "expires 2027-01-31: <why it is accepted, and whether we are exposed> (6j6v.xxxx)" },
]
```

```toml
# agent-sidecar/npm-audit.toml (create it when the first npm exception is needed)
ignore = [
    { id = "GHSA-xxxx-xxxx-xxxx", reason = "expires 2026-12-31: <why> (6j6v.xxxx)" },
]
```

- **`id`**: the advisory id exactly as the gate prints it (`RUSTSEC-…` or `GHSA-…`).
- **`reason`** starts with **`expires YYYY-MM-DD:`**. The gate is red again on the day after that
  date, without anybody touching the tree, so the decision gets taken again rather than
  forgotten. Pick the date by severity: no further out than the response time for critical and
  high, the next regular release for medium, at most three months for low.
- The reason **names the board item** that carries the decision (`6j6v.xxxx`). The item holds
  the reading of whether we are exposed, and the follow-up that removes the exception.
- One entry per advisory, never a pattern, never a whole crate.
- A bare id without a reason is refused, and so is any entry without an expiry or a board item.
- An exception whose advisory no longer fires is reported. For npm, `deps npm-audit` warns; for
  Rust, `cargo deny` warns `advisory-not-detected`. Remove it in the next change.

Renewing an exception is a new decision: change the date and add a note to the board item saying
why it is still accepted.

## What evidence is kept

For every advisory the gate has gone red on:

| Evidence | Where it is kept |
|---|---|
| **The advisory**: id, URL, severity, affected crate and versions | The board item's description. |
| **The first red run** (the clock's start) and the run that went green again | Their URLs, as notes on the board item. |
| **The decision**: fixed, or accepted until when and why | The board item's notes, plus the exception entry in git (`git log -p deny.toml` shows who recorded it and when). |
| **The fixing release** | The board item's close reason names the version and the commit or PR. |
| **The audit result of each release** | The `dependency-audit` job of the `ci.yml` run on the tagged commit. It goes into each release's evidence record (B-4, 6j6v.tqd9), which keeps it for 10 years. |

Until B-4 archives the run logs, GitHub keeps them for 90 days only. The board item, the git
history and the release notes are what lasts until then.

## Current exceptions

Recorded by 2dn0 on 2026-10-24, when the gate was introduced:

| Advisory | Crate | Kind | Expires | Item |
|---|---|---|---|---|
| RUSTSEC-2024-0449 | unsafe-libyaml 0.2.11, via serde_yaml 0.9 (nexus-chat) | unmaintained | 2027-01-31 | 6j6v.spqq |

Every other advisory found on that day was fixed in the same change. The list is in the
session report on 6j6v.2dn0.
