# How sessions reach each other (2026-11-14)

This research was part of `factory/GENESIS.md` until 2026-11-15 (NEX-33), when it moved here: it is dated and goes stale, and the product file should not.

No framework reaches a session of another framework, and none reliably reaches across repositories or teams. That is why the ticket carries the message. Web research of 2026-11-14, from docs and issue trackers, not tried out:

| Framework | Same machine | Other machine or cloud | Across teams or repositories |
| --- | --- | --- | --- |
| Claude Code | `ListAgents` and `SendMessage` over local sockets ([docs](https://code.claude.com/docs/en/cross-session-messaging)) | Through Remote Control; a cloud session receives messages but cannot yet send them back | The owner's own sessions, one account only |
| Kiro Crew | A long-running gateway that runs several sessions ([docs](https://kiro.dev/docs/crew/)); a call that delivers a message as another session's next turn is not confirmed: the review of 2026-11-15 (NEX-30) found no `session_send` in the docs, which this table cited until then | Only sessions on the same crew gateway | Not yet; cross-crew delegation is an open request ([#13476](https://github.com/kirodotdev/KiroCrew/issues/13476)) |
| Kiro CLI | No messaging to a running session; an open request ([#5848](https://github.com/kirodotdev/Kiro/issues/5848)) | – | – |
| Codex | `codex app-server` (JSON-RPC, experimental) and `codex agents` on a shared server ([docs](https://developers.openai.com/codex/app-server)) | `codex --remote` to a server; messaging between cloud tasks is not documented | – |
