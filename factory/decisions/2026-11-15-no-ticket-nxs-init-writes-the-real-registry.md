# 2026-11-15 · no ticket · nxs init writes to the real background-service registry (~/.nexusflow/workspaces.toml) on every run — any black-box test that runs it MUST pin $HOME.

- Decided by: recorded in the project memory (`nxm`, key `nxs-init-writes-the-real-registry`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: service, testing

## The record

`nxs init` WRITES to the background service's workspace registry (`~/.nexusflow/workspaces.toml`,
or `~/.nexusflow-<name>` under `NXS_SERVICE_INSTANCE`) — on every run, `--json` included. That
makes every black-box test that runs `nxs init` and does NOT pin `$HOME` a leak: it leaves one dead
`TempDir` entry per case in the developer's REAL registry. That is how the 98 corpses measured once
came about; while implementing the change that introduced the write, two full test runs had already
added 77 new ones.

It does not show up locally (nothing goes red) and in CI not at all (there `$HOME` is a throwaway
directory anyway) — the same fail-open shape as the earlier bug where ten tests pinned `HOME` to a
`TempDir` but inherited `NXS_SERVICE_INSTANCE` from the environment.

RULE: every test that starts `nxs init` pins `$HOME`. `nxs_test_support::cargo_bin` already does
this; if you build your own command (to avoid inheriting that helper's defaults), add
`.pin_home(nxs_test_support::pinned_home())` — `pinned_home()` is public for exactly this. The
tests that were affected were `crates/nxs/tests/{e2e,retrieval_rule,the_session_start_ceiling,
a_persona_gets_the_suites_prime}.rs`.

THERE IS NO GATE THAT ENFORCES IT: the existing `every_pinned_home_pins_the_instance` gate only
checks that a PINNED home also pins the instance, not that anything is pinned at all. An open board
item tracks closing that hole.
