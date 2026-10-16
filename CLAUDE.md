# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

## Quality gates: CI is the gate

The full suite and clippy take hours on the development machine and minutes in CI, so **the
project's gate is the CI workflow, not a local run.** Never run `cargo clippy`, `cargo test --all`
or `cargo test --workspace` locally.

Locally, run only `cargo fmt --all -- --check` and the test files your change touched
(`cargo test -p <crate> --test <file>`). Then the gate is:

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

