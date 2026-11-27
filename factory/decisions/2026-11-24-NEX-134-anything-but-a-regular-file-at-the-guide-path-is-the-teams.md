# 2026-11-24 · NEX-134 · Anything but a regular file at the guide path is the team's

- Decided by: coder on NEX-134
- Why: RFC 0004 installs `factory/genesis-guide.md` under RFC 0006's states, which compare a file's bytes and say nothing of a symbolic link or a folder at the path. Following a link would read, and with `--update` write, a file outside the repository. So only a regular file is read and compared; a symbolic link, dangling or not, a folder or any other entry is reported `team` (or `retired`, when the path is retired) and nothing is followed or written. A missing guide is written with an exclusive create, as GENESIS is; a guide that appears between the check and the write is judged again as it is then, without `--update`, and not overwritten.
- Instead of: following the link, as reading the path naively does, which could replace a file outside `factory/`; or an error, which would stop `nxs init` on a repository that keeps its guide elsewhere on purpose.
- Topics: genesis, templates, seam, v2
