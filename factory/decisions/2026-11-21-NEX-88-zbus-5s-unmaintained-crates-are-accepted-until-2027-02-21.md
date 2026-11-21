# 2026-11-21 · NEX-88 · zbus 5's unmaintained crates are accepted until 2027-02-21

- Decided by: pm
- Why: RUSTSEC-2026-0370, -0371 and -0372 mark zvariant, zbus_names and zvariant_derive 5.x unmaintained (merged into zbus 6), with no exploitable defect and no safe upgrade. They reach nxs only through nocentraldb's Linux keyring store, which pins zbus 5. Under docs/runbooks/dependency-vulnerabilities.md, unmaintained with no known defect is low severity, so it gets a dated exception of at most three months: `deny.toml` ignores all three until 2027-02-21. The request to move the keystore to zbus 6 is NOC-408 on NoCentralDB's tracker, and NEX-214 drops the exceptions at the pin move that takes it.
- Instead of: holding the SDK pin until NoCentralDB ships a zbus 6 keystore, which would stall priority 1, or patching the SDK's dependency tree in nexus-flow.
- Topics: dependencies, advisories, nocentraldb
