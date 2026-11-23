# 2026-11-23 · NEX-98 · A session member's database opens as a sub-agent

- Decided by: coder on NEX-98
- Why: the SDK takes a member's kind from the open options, not from the database, so a session's database must be opened with the kind it has. RFC 0001 makes every role session pm's sub-agent, and opening as a sub-agent is safe for them. pm's own session is an agent paired into the person's account (`nxs session start --role pm --pairing`), which the ticket that builds session start must keep apart, for example with a mark in the session folder. Until then, `NXS_SESSION` and `Engine::as_session` open every session as a sub-agent.
- Instead of: opening without a kind, which would open every session as a device.
- Topics: storage, seam, sessions, nocentraldb, v2
