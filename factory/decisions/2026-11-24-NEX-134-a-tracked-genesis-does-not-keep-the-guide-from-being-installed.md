# 2026-11-24 · NEX-134 · A tracked GENESIS does not keep the guide from being installed

- Decided by: coder on NEX-134
- Why: RFC 0004's `tracked` row says `create` does nothing to GENESIS and exits non-zero, and its section on the guide installs the guide without naming GENESIS's state. GENESIS and the guide are therefore handled each on its own: on a `tracked` run the guide is still installed and reported, the report on stdout holds both files and `codeowners`, and the run exits non-zero because of GENESIS alone. The guide is nexus-flow's text, so writing it never risks the team's GENESIS, and a rerun after restoring GENESIS finds the guide `current`.
- Instead of: refusing the whole run before the guide, which would leave the report without the guide's state, and make one missing file hold back an unrelated one.
- Topics: genesis, templates, seam, v2
