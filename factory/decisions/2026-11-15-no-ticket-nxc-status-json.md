# 2026-11-15 · no ticket · Always watch nxc runs with 'nxc status --thread <id> --json', never parse the text output; done at open == 0, stop at needs_decision or awaiting_human.

- Decided by: recorded in the project memory (`nxm`, key `nxc-status-json`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, chain

## The record

Adopted from agents (owner, 2026-09-22): ALWAYS watch the state of nxc runs (coding, planning, merging ...) through 'nxc status --thread <id> --json'. Reading it: operations[].open == 0 means done; operations[].needs_decision == true or threads[].awaiting_human == true means a human must decide: report it to the owner at once instead of waiting longer; the same for threads[].escalated and interrupted. Which step is running: threads[] with state 'open' and session_state 'running' (expects names the persona). Several runs: 'nxc status --json' without --thread. Wait in the background (Monitor, or Bash with run_in_background), never with a foreground sleep.

(Translated from German on 2026-11-15.)
