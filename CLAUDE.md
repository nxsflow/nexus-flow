# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

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

