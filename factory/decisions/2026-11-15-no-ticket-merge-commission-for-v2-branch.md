# 2026-11-15 · no ticket · A merge commission for a branch coding-v2 built always says: read it as origin/<branch> after git fetch origin - the merger's git log otherwise reads the root's checkout.

- Decided by: recorded in the project memory (`nxm`, key `merge-commission-for-v2-branch`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: nxc, chain, merging

## The record

A merge commission to the merging channel for a branch that coding-v2 built (the new run engine, nxc rebuild slice 1, epic 6j6v.zjyv) ALWAYS carries this sentence, because the merger's only read of the local checkout (merger SKILL.md: git log --oneline -20) runs in the registered root, which never has a v2 branch checked out - the v2 run works in its own clone <root>-v2: 'This branch was built on the new engine in a separate clone, so your working copy never has it checked out. Read it from the remote: run git fetch origin, then git log --oneline -20 origin/<branch> in place of git log --oneline -20, and wherever you would read the branch locally, read origin/<branch>.' Without it the merger judges ride-along-or-merge on whatever branch the root stands on, and nothing reports it. Chosen over a change to the user-level merger (6j6v.131c), which every workspace on the machine reads. Obsolete once the merging channel itself moves to the new engine (6j6v.2vtw); forget it then.
