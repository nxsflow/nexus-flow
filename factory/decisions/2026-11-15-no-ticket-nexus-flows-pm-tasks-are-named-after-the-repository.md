# 2026-11-15 · no ticket · nexus-flow's pm scheduled tasks are named "nexus-flow pm: …"

- Decided by: pm
- Why: the owner's account also runs the NoCentralDB factory, whose pm owns scheduled tasks named `factory pm: hourly tick` and `factory pm: daily note`, the names `factory/roles/pm.md` gives; a nexus-flow pm that took over every task of those names would disable the other repository's pm. nexus-flow's pm uses `nexus-flow pm: hourly tick` and `nexus-flow pm: daily note`, and takes over only tasks with those names.
- Instead of: the names in `factory/roles/pm.md` as they stand, which are not unique across the owner's repositories (a change to pm.md is a follow-up the owner reviews).
- Topics: factory, pm, scheduled-tasks
