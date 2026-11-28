# 2026-11-28 · NEX-249 · nxs init prints the factory and genesis reports, and a tracked GENESIS's error on stderr

- Decided by: coder on NEX-249, carrying out the owner's answer to NEX-243
- Why: the owner decided NEX-243 (a), so NEX-139's provisional safe subset ends. `nxs init`'s JSON keeps `space`, `created`, `sync` and `v1_workspace`, then `factory` and `genesis`, built from the same output types `nxs factory install` and `nxs genesis create` print, so the two cannot drift apart. `--md` keeps the space's lines and adds one line for each install, last: the files it wrote (state `written`) or "wrote nothing", the paths no CODEOWNERS rule covers in the wording each command already uses, and for GENESIS a note when a GENESIS git tracks was not written. What NEX-139 decided for a tracked GENESIS stays: the record goes to stdout, the `error: …` of `nxs genesis create` to stderr, and the exit code is 1.
- Instead of: one line per file in `--md`, which repeats `nxs factory install --md` and buries the space's lines under it; or listing every file with its state in the line, when a person reading `nxs init` needs to know what to commit.
- Topics: init, factory, genesis, cli, output, v2
- Supersedes: 2026-11-27-NEX-139-nxs-init-prints-its-record-unchanged-and-a-tracked-genesis-error-on-stderr.md
