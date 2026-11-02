# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

## Quality gates: CI is the gate

The full suite and a workspace-wide clippy take hours on the development machine and minutes in
CI, so **the project's gate is the CI workflow, not a local run.** Never run `cargo test --all`,
`cargo test --workspace`, or `cargo clippy` without `-p` (that includes `--all` and
`--workspace`) locally.

Locally, run only:

- `cargo fmt --all -- --check`;
- the test files your change touched (`cargo test -p <crate> --test <file>`);
- clippy **scoped to the crates your change touched, and nothing else**:
  `cargo clippy -p <crate> --all-targets -- -D warnings`, one `-p` per crate that contains a
  file you created or edited. Clippy cannot lint a single file; the crate is the smallest scope it
  has, so the crate is the scope. Run it once, before you push, not after every edit: on this
  machine a crate-scoped clippy of `nexus-chat` measured 15–25 min (2026-09-18), but the same
  lint error found by CI costs a full CI cycle, which was 80+ min in November 2026.

Then the gate is:

```
git push origin HEAD
gh workflow run ci.yml --ref "$(git branch --show-current)"
run=$(gh run list --workflow ci.yml --branch "$(git branch --show-current)" --limit 1 --json databaseId -q '.[0].databaseId')
gh run watch "$run" --exit-status
```

Check that the run's `headSha` is `git rev-parse HEAD` before you trust it: a run proves only the
commit it ran on. If you hand over a gate result, give that commit and the run URL. If a job is
red, the failing job's log (`gh run view "$run" --log-failed`) is the evidence.

## Removing a verb, a flag or a path

A removal leaves **no identifier to grep for**. `cargo check` and every gate stay green while the
prose that named the thing rots in place — and prose is the surface an agent reads, so a help page
or a system prompt naming a verb that no longer exists is a false map, not a cosmetic defect.

After a removal, run:

```
bash xtask/sweep-retired-vocabulary.sh [<base-ref>]
```

It lists files that still name a retired verb or path and that your change never opened. **It is a
review aid, not a gate** — it exits 0 either way, it is not in CI, and every line it prints is for
you to judge (a supersession note or a changelog fragment names a retired term correctly). Add your
own removal to its vocabulary list; that list, and the exclusions with their reasons, are the whole
content of the file. Its header also records the two shapes it cannot see — lines ADJACENT to lines
you edited, and files INSIDE your own change set — and how to sweep for those by hand.

