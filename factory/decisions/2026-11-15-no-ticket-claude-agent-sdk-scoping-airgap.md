# 2026-11-15 · no ticket · To air-gap a claude-agent-sdk subagent set setting_sources=[], strict_mcp_config=True and mcp_servers={} — else it inherits your MCP and the web.

- Decided by: recorded in the project memory (`nxm`, key `claude-agent-sdk-scoping-airgap`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: agent-sdk, security

## The record

claude-agent-sdk subagents inherit the user's GLOBAL config (including MCP servers such as
Playwright) and can reach the web via Bash even when `ClaudeAgentOptions`
`tools=[...]`/`allowed_tools=[...]` are set — `bypassPermissions` auto-approves MCP and network.

Discovered in a tool-scoping spike: agents in the restricted arm browsed the web via Playwright-MCP,
contaminating the very experiment that was measuring tool scoping.

To truly air-gap a scoped agent:
- `setting_sources=[]` (do not load user/project/local settings → no inherited MCP),
- `strict_mcp_config=True` together with `mcp_servers={}`,
- `disallowed_tools=['WebFetch','WebSearch']`,
- and strip `curl`/`wget`/`gh` from the env `PATH`.

Verify by grepping the transcripts for `mcp`/`playwright`/`browser`/`webfetch`/`http` tool calls —
an air gap you have not checked for is an assumption, not a control.

Also: do NOT wrap the batch in `perl -e 'alarm N; exec ...'` — it leaves SIGALRM at `SIG_DFL` and a
stray signal kills python mid-run (exit 142). Run python directly and rely on `max_turns` plus
resumability.
