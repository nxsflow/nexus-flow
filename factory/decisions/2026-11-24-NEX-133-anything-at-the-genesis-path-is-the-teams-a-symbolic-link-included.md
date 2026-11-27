# 2026-11-24 · NEX-133 · Anything at the GENESIS path is the team's, a symbolic link included

- Decided by: coder on NEX-133
- Why: RFC 0004's table calls GENESIS `team` when it is "present, with any content", and says the write fails on a symbolic link, dangling or not, without naming the state then. Any entry named `factory/GENESIS.md` (a file, a symbolic link, dangling or not, or a folder) is reported `team`, nothing is followed and nothing is written, and the run exits 0: the path is not free, and what is there is not nexus-flow's to judge. A file whose name differs only in case is `team` with `found`; when several do, `found` names the first, bytewise. A GENESIS that appears between the check and the write makes the exclusive create fail, and is reported `team` too.
- Instead of: an error for a symbolic link, a state of its own that the RFC's output does not list, or following the link, which could write the template outside the repository.
- Topics: genesis, seam, v2
