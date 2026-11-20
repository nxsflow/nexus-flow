# 2026-11-20 · NEX-44 · pm keeps a session's framework handle in a comment on its ticket

- Decided by: Carsten
- Why: pm needs the agent framework's handle on a running session, such as a Claude Code session id, to message, wake or archive it, and a new pm needs it to take over. A comment pm writes on the session's ticket when it starts the session needs no new field, outlives the session, and is how this factory works today; nexus-flow neither reads nor checks its shape. Written into RFC 0006, "The running state" (NEX-10).
- Instead of: a `framework` field on RFC 0001's `spaceMember` record, which would make nexus-flow store a framework detail; or not storing the handle at all, so that a new pm could not take over the running sessions.
- Topics: factory, sessions, storage
