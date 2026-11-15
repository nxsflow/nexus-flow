# 2026-11-15 · no ticket · A Bash tool runs outside the session's process group, in its own session: a group signal never reaches it, and the group is what separates the CLI and MCP servers from a tool's work.

- Decided by: recorded in the project memory (`nxm`, key `claude-cli-tool-process-tree`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, agent-sdk, processes

## The record

The Claude CLI (SDK 0.3.215, measured on macOS and Linux with the real binary, 6j6v.tv58 step 0) starts each Bash tool's shell in a session AND group of its own; the leader's group holds only the leader, the CLI and stdio MCP servers. So a signal to the session's group does not reach a tool (SIGKILL of the group leaves the tool alive), and the group is the exact way to leave CLI and MCP CPU out of a sample: a waiting CLI spends CPU at every look. nexus-run-session-claude's sampler and stop rest on this. After an SDK bump re-measure: cargo test -p nexus-run-session-claude --test real_sidecar -- --ignored --test-threads=1 (needs npm ci && npm run build in agent-sidecar, no credentials). Also measured: Darwin's proc_pidinfo answers nothing about a zombie, so a start time cannot be compared there before a waitpid.
