# 2026-11-15 · no ticket · A pinned $HOME does NOT isolate launchd — RealCtl refuses outside the real login session, and still no test runs the real install.

- Decided by: recorded in the project memory (`nxm`, key `launchd-install-is-not-home-isolated`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: service, launchd, testing

## The record

A pinned `$HOME` isolates files but NOT launchd. `nxs sync daemon install` does write its plist
under `$HOME/Library/LaunchAgents`, but it then calls `launchctl bootstrap gui/<uid>` — and that
domain is the developer's REAL login session, whatever `$HOME` says. A black-box test that actually
runs the install path therefore bootstraps a real agent into the session of the human running it,
pointing at a test binary in a `TempDir` that is about to disappear.

THIS HAPPENED ONCE AND COST FIVE DAYS: the production sync clock on this machine stood still from
30 August to 4 September, because launchd held a registration for `com.nxsflow.nexus-flow` from a
deleted `TempDir` and the real plist could not displace it.

SINCE THAT FIX THIS MEMORY IS A CHECK, NOT AN ADMONITION. `nxs_service::launchd::RealCtl` can only
be constructed through `RealCtl::for_login_session()`, which refuses with `forbidden` when the
process's `$HOME` is not the home directory the user database holds for `getuid()`.
`install_for`/`uninstall_for` construct it FIRST, before plist, alias and log directory. A test with
a pinned `$HOME` now gets a named refusal instead of a silent poisoning; the counter-proof lives in
`crates/nxs/tests/no_test_reaches_the_real_launchd.rs`.

THE GAP THAT REMAINS: a test that does NOT pin `$HOME` is indistinguishable at runtime from a human
typing the command — it still reaches launchd. An open board item proposes a gate that would
enforce `$HOME` pinning for black-box tests; this refusal is not that gate.

SO, STILL: no test in this repo runs the real install. `sync bind` tests all pass `--no-daemon`;
`nxs init` tests use `--no-service` or no flag at all. The yes-branch is proven at a trait seam
(`background_service::Machine`, `sync::mod::Autostart`), not at the process. Anyone building a new
surface that can install needs the same seam — otherwise the test is not isolated, merely not yet
executed.

AND IF IT DOES HAPPEN, SOMETHING SAYS SO NOW: `nxs sync daemon status` compares the loaded
registration (`launchctl print gui/<uid>/<label>`, field `path`) against its own plist and reports
a foreign or orphaned one as `registration: FOREIGN — ...` with path and cause, instead of letting
it pass as a stale heartbeat.
