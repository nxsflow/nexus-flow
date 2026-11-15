# 2026-11-15 · no ticket · A self-update does not stop chain runs since wxz0: it only restarts the service watching them, so it may run during a run; report any damage as a regression.

- Decided by: recorded in the project memory (`nxm`, key `no-self-update-during-chain-runs`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, chain, release

## The record

CORRECTED 2026-11-05, owner: an nxs self-update does not stop chain runs. It restarts the background service that watches them; the runs carry on. The fear this entry used to state came from bug 8n9a (seen 2026-10-15 on the 0cfa coding run, the restart SIGTERMed service-started sessions with no resume). That bug is closed through wxz0: a planned service restart secures every running agent session first, and the sidecar never turns that stop into a silent clean end. So a self-update during a run is allowed, and the rule no longer holds a release back. Tell the owner if a run is damaged by an update: that would be a regression, and it should be observed on the next release, not assumed either way. What still stands: `nxs init --service` reinstalling the shared service without asking is its own bug, xkvn.
