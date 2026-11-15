# CONFIG.md

The values of this repository's factory: how work moves here today. `factory/GENESIS.md` describes the product's target state; this file names the tools that serve until nexus-flow can do their job itself.

| Setting                            | Value |
| ---------------------------------- | ----- |
| Owner (the source)                 | Carsten (GitHub `@cabcookie`) |
| Product file                       | `factory/GENESIS.md`, the target state. The work moves towards it on the base branch, and the base branch merges into `main` once it is reached. |
| Designs                            | RFCs in `docs/rfc/`, one per area of GENESIS.md; `docs/rfc/README.md` says how one is written and decided. The owner decides each RFC. |
| Decision log                       | `factory/decisions/`, one file per decision; `factory/DECISIONS.md` says how to read and write one. Read it with `cargo xtask decisions` (`--current --index` for the decisions in force; `--help` lists every option). |
| Tracker (the sink)                 | Linear, workspace **nexus-flow**, through its GraphQL API at `https://api.linear.app/graphql`; the cloud environment's proxy adds the key. Never the claude.ai Linear connector, which points at another workspace. |
| Team                               | `NEX` (Nexus-flow) |
| Ticket id in commits and PR titles | `NEX-<number>` |
| Initiative for the daily note      | `nexus-flow` |
| Label for questions to the owner   | `needs-owner` |
| Coding agents in parallel          | 4 |
| A lane is free again               | when pm has reviewed the PR and hands it to the owner; a PR that waits only for the owner holds no lane |
| Expedite lanes                     | 1 |
| Base branch                        | `v2`. Every ticket branch starts from `v2`, and every PR targets it. `main` keeps shipping v1 and takes no factory PR. |
| Keeping the base branch current    | pm merges `main` into `v2` at least once a week, and after every release cut on `main`, through a PR of its own. A conflict in that PR goes to the owner as a question. |
| Branch per ticket                  | Linear's branch name for the ticket, started from `v2` |
| Gate command                       | The CI workflow `.github/workflows/ci.yml`, green on the PR's head commit before a PR is marked ready; the ruleset "main and v2: every change through a pull request" names the required checks, among them `lint`, `quality-gates` and `commit-roles`. Locally, before every push, run only: `cargo fmt --all -- --check`; the test files the change touched (`cargo test -p <crate> --test <file>`); and clippy on the touched targets (`cargo clippy -p <crate> --lib --test <file> -- -D warnings`). Never a local `cargo test --all`, `--workspace` or `--all-targets`. Before a merge, pm dispatches the full gate once: `ci.yml` with `full=true` on the branch. |
| Change fragments                   | A PR with product impact adds one under `changes/` (see `changes/README.md`); `changelog-check` enforces it. A PR without product impact outside the paths `changelog-check` already exempts gets the `skip-changelog` label. |
| Releases                           | None from `v2`: no tag, package or binary is cut there. Releases come from `main`, as `docs/specs/release-management.md` describes. |
| Largest code or test file          | No line limit is enforced; the crates of the nxc rebuild have the size budget `cargo xtask layering check` enforces. Split a file by concern before it grows into several. |
| Merge rule                         | Every PR opens as a draft. pm marks it ready and squash-merges it once the owner has approved it on GitHub. Exception: a PR that only implements a decided RFC, as `roles/pm.md` defines it, pm checks and merges without the owner's review, says so in a PR comment, and lists it in the daily note. No exception covers a PR that touches a path in `.github/CODEOWNERS`: it always needs the owner's approving review. Branch protection on `main` and `v2` enforces the pull request, the code owners' review, dismissal of stale approvals, the required checks, and no bypass for any agent account. |
| Daily note                         | Every day at 06:00 Europe/Berlin |
| Runtime                            | Claude Code cloud environment `nexus-flow`. Its setup script installs Rust 1.99.1, fetches the crates, runs `npm ci` in `agent-sidecar/`, moves a session that starts on `main` to `v2`, and appends a cloud-only block to AGENTS.md (marked skip-worktree, never committed). |
| Agent account on GitHub            | `nexusflow-ug`, with the Write role on the repository only. Its verified email, `github@nxsflow.com`, is the author email of every agent commit; the author name is the session's role, such as `coder on NEX-123` or `pm`. |
| Language                           | English for everything durable; chat follows the owner's language |
