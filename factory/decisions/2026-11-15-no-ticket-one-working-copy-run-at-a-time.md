# 2026-11-15 · no ticket · Commission one working-copy run at a time and wait for its release; the causing bugs are fixed since v0.207.0, but keep the rule until observed.

- Decided by: recorded in the project memory (`nxm`, key `ein-lauf-mit-arbeitskopie`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, chain

## The record

Adopted from agents (as of 2026-10-10, nxs 0.206.1): two runs holding the working copy at the same time led to build steps skipped after a long wait, coder and verifier working in the same working copy at once, and a working copy held until its deadline after an escalation answered on the root thread. The causes are the nxc bugs 6j6v.9g8j (one step per ordered run), 6j6v.1wep (queued steps do not lapse) and 6j6v.ys54 (an answered escalation releases the copy); all three are FIXED since nxs v0.207.0. Until a newer release has been observed running two working-copy runs cleanly, keep the rule anyway: commission one run, wait until it releases the working copy, then the next. The session itself does not work in the working copy while a run is going (use its own git worktree if needed).
