# 2026-11-15 · NEX-37 · Commits name the session role that made them

- Decided by: Carsten
- Why: the history then shows which role on which ticket did the work and who commissioned it, as NEX-16 does for NoCentralDB members. A commit's author name is the role on the ticket (`coder on NEX-123`, or `pm`), its author email the verified email of the agent account `nexusflow-ug` (`github@nxsflow.com`, CONFIG.md), never the owner's address. Every commit message carries a `Role:` trailer, because trailers survive the squash merge and author names do not; pm's squash message keeps the `Role:` and `Claude-Session:` trailers, and the owner stays `Co-authored-by`. The `commit-roles` workflow (`cargo xtask commit-roles`) fails a pull request with a commit that lacks the trailer; merge commits and commits already on the base branch are not checked.
- Instead of: the cloud session's default author (`Claude <noreply@anthropic.com>`), which names neither the role nor the ticket, or the owner's address, which puts personal data into a public history and makes the agent's work look like the owner's.
- Topics: factory, git, identity
