# CONFIG.md

The values of this repository's factory. FACTORY.md is copied unchanged between repositories; this file is the one that changes.

| Setting                            | Value |
| ---------------------------------- | ----- |
| Owner (the source)                 | Carsten (GitHub `@cabcookie`) |
| Product file                       | `factory/GENESIS.md` |
| Decision log                       | `factory/decisions/`, one file per decision; read it with `cargo xtask decisions` (`--help` lists the options). Where FACTORY.md or `roles/` say `pnpm decisions`, run `cargo xtask decisions` with the same options (`--index`, `--current`, `--ticket`, `--topic`, `--since`). `factory/DECISIONS.md` says how to write one. |
| Tracker (the sink)                 | Linear, workspace **nexus-flow**, through its GraphQL API at `https://api.linear.app/graphql`; the cloud environment's proxy injects the key. Never the claude.ai Linear connector, which points at another workspace. |
| Team                               | To be filled in: the team key of the workspace nexus-flow, read from Linear once the API key works. |
| Ticket id in commits and PR titles | `<team key>-<number>` |
| Initiative for the daily note      | `nexus-flow` |
| Label for questions to the owner   | `needs-owner` |
| Coding agents in parallel          | 4 |
| A lane is free again               | when pm has reviewed the PR and hands it to the owner; a PR that waits only for the owner holds no lane |
| Expedite lanes                     | 1 |
| Base branch                        | `main` |
| Branch per ticket                  | Linear's branch name for the ticket |
| Gate command                       | The CI workflow `.github/workflows/ci.yml` is the gate, green on the PR's head commit before a PR is marked ready. Locally, before every push, run only: `cargo fmt --all -- --check`; the test files the change touched (`cargo test -p <crate> --test <file>`); and clippy scoped to the touched targets (`cargo clippy -p <crate> --lib --test <file> -- -D warnings`). Never a local `cargo test --all`, `--workspace` or `--all-targets` (memory `gates-run-in-ci-not-locally`). CI runs, among others: `cargo fmt --all --check`, `cargo xtask layering check`, `cargo xtask decisions --check`, `cargo clippy --all-targets -- -D warnings`, `node --test` in `agent-sidecar/`, the `cargo xtask` drift gates, the lean builds and `cargo test --all`. Before the merge, pm dispatches the full gate once: `ci.yml` with `full=true` on the branch. |
| Change fragments                   | A PR with product impact adds one under `changes/` (see `changes/README.md`); `changelog-check` enforces it. A PR without one gets the `skip-changelog` label from the owner. |
| Largest code or test file          | No line limit is enforced in this repository yet; the nxc rebuild's crates have the size budget `cargo xtask layering check` enforces. Split a file by concern before it grows into several (see FACTORY.md). |
| Merge rule                         | Every PR opens as a draft. The owner ticks its Authorship box (`.github/AI_POLICY.md`); only then pm marks it ready and squash-merges it. Exception: a PR that only implements a decided RFC, as AI_POLICY.md defines it, pm checks, ticks "Agent, checked against a decided design", and merges without the owner. A second exception: the PR that carries a beta's release notes, curated by an agent, which pm checks, ticks "Agent, beta release notes" and merges without the owner. The notes of a stable release stay with the owner. A third exception: a documentation PR that carries out a decision already made and decides nothing new, which pm checks, ticks "Agent, carries out a decision" and merges. A fourth: a PR that changes only developer and agent tooling, which pm checks, ticks "Agent, tooling" and merges. AI_POLICY.md defines all four. Above all of these: a PR that touches a path in `.github/CODEOWNERS` needs the owner's approving GitHub review, even when an exception would let pm merge it. pm requests that review and never marks such a PR ready or merges it without it. Branch protection on `main` enforces it: a pull request, a code owner's review, stale approvals dismissed, the required CI checks, and no bypass for any agent account. |
| Daily note                         | Every day at 06:00 Europe/Berlin |
| Runtime                            | Claude Code cloud environment `nexus-flow` |
| Agent account on GitHub            | `nexusflow-ug`, with the Write role on the repository only |
| Language                           | English for everything durable; chat follows the owner's language |
