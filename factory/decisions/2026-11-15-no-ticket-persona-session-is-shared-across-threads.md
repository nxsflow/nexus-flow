# 2026-11-15 · no ticket · A persona runs ONE real SDK session across all its threads, and the transcript records no user prompts - name the thread when you continue it.

- Decided by: recorded in the project memory (`nxm`, key `persona-session-is-shared-across-threads`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, personas

## The record

A persona's internal session (session_map) is ONE real Claude Agent SDK session continued across every thread that persona serves - e.g. the pm session m-01M5MSS6F6CNT77BZW5DZ6TGEB took turns for a0y8, m-01M5456J..., m-01M57STW... and m-01M5MSPF... within 20 minutes on 2026-10-23. Consequences: 'continue the same session' must NAME the thread and quote the task; 'no transcript activity' is a fact about the session, not one slot; a queued commission fired into that session arrives as a continuation (resume_real set) and must carry the body. The agent_transcript records assistant, tool and subagent entries with an 'at' timestamp, but NO user prompts - you cannot read from a transcript what a session was asked.
