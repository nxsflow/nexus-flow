# 2026-11-20 · NEX-82 · A role session sees a request through a copy in an ad-hoc space pm gives it

- Decided by: Carsten
- Why: a role session must see the request its ticket waits for (NEX-47), but pm is only a writer of the coordination space and cannot add members there. NoCentralDB's ad-hoc spaces let a parent agent give its own sub-agent copies of the objects a task needs without being an admin of the source space (NoCentralDB RFC 0003, "Sub-agents get an ad-hoc space"; RFC 0009, "Sub-agents and ad-hoc spaces"), so role sessions stay out of the coordination space, as RFC 0001 has it. Written into RFC 0009, "Members and roles".
- Instead of: pm as an admin of the coordination space, so it adds its sessions there as readers; or the owner's device adding every session.
- Topics: cross-repo, coordination, members, nocentraldb
