# 2026-11-15 · no ticket · Several consecutive BETA releases are explicitly fine and NOT a backlog — an idea proves itself in the apps before the chain goes stable.

- Decided by: recorded in the project memory (`nxm`, key `release-ring-policy`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: release

## The record

OWNER DECISION 2026-08-02 — RELEASE RING POLICY: several consecutive BETA releases are explicitly
fine and are NOT a backlog.

The final proof of an idea ALWAYS happens in the apps — not in tests, not in the CLI. Only once an
idea has proven itself there and is meant to be published is the WHOLE CHAIN pulled to stable:
nexus-flow first, then app-foundations, then the affected app. That makes a promotion expensive;
that cost is accepted deliberately.

The owner will not be pushed into premature stable versions while something has only proven itself
in tests and a little in the CLI.

CONSEQUENCE FOR THIS REPO: a release may sit in the beta ring with a clear conscience, even when
unpromoted beta versions already sit in front of it. A release recommendation must NOT present that
as a problem. nexus-flow is the FIRST link in the chain when a promotion does happen.
