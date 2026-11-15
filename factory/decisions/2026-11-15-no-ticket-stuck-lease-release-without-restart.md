# 2026-11-15 · no ticket · A delivered run stuck holding the copy (vtg9): reply on the root to open a round, then withdraw at once; never restart the service while others run.

- Decided by: recorded in the project memory (`nxm`, key `stuck-lease-release-without-restart`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, chain

## The record

When a delivered nxc operation keeps holds_working_tree and needs_decision true with held_by_hand_back {kind: escalation} (vtg9), answering the escalation thread is refused ('round is over'), and withdraw refuses ('nothing running'). The working release path without a service restart (measured by manufakt 2026-10-20 on 0.216.0): one short reply on the ROOT opens a new round, then immediately run nxc withdraw --thread <root>. The coder is stopped, its state parked onto a branch, and after about 2-4 min the lease and the flag clear. Never restart the service while other workspaces run sessions (8n9a).
