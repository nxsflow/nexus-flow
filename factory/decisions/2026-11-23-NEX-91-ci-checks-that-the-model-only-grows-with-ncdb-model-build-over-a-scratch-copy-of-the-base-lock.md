# 2026-11-23 · NEX-91 · CI checks that the model only grows with ncdb model build over a scratch copy of the base lock

- Decided by: coder on NEX-91
- Why: RFC 0001, "Building it", says CI "runs `ncdb model check` against the lock on the base branch". At ncdb 0.1.0-beta.1, `check` also fails when the lock it is given is out of date, so against the base lock it fails every PR that adds a field (tried). `ncdb model build` applies the same additivity rule, refuses a removed or renamed field, and writes only the scratch copy it is given. The base lock is the reference, not the PR's own lock, so a PR that rebuilds its lock from nothing cannot hide a removal. The rebuild check is a second, separate `build` over a scratch copy of the committed lock, compared byte for byte, because each rebuild raises the model version and a branch that rebuilt twice is at version 3 where a rebuild from the base writes 2.
- Instead of: `ncdb model check --lock <base lock>`, which fails on every additive change; or comparing the committed lock with one rebuilt from the base lock, which fails a branch that rebuilt more than once.
- Topics: model, nocentraldb, ci, v2
