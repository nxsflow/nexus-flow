# 2026-11-15 · no ticket · An installed nxs is ALWAYS the production instance; only a build reads NXS_SERVICE_INSTANCE, and it says which, not whether.

- Decided by: recorded in the project memory (`nxm`, key `the-running-binary-decides-the-service-instance`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: service, sync, development

## The record

Since nxf 6j6v.cvpy the precedence in `nxs_service::Instance::resolve_from` is THREE inputs, not two,
and the middle one is new:

1. `argv[0]` — an alias (`nexus-flow-dev`) wins outright. That is how a launchd-started service knows
   itself: `ProgramArguments` carries exactly one element, so it has no other channel. It is also the
   escape hatch for a human — `~/.nexusflow-dev/bin/nexus-flow-dev uninstall` addresses that instance
   from anywhere. The alias STANDS FOR `nxs sync daemon`, so the verb follows it directly;
   `<alias> sync daemon uninstall` answers `unrecognized subcommand 'sync'`.
2. `nxs_service::Origin` — where the running binary SITS. A build tree is recognised by the Cargo
   profile directory (`target/{debug,release}`, or one level below it where every test binary lives),
   by any `target` ancestor (a `--profile` of one's own), and by Cargo's own `CACHEDIR.TAG` checked by
   its SIGNATURE (a private profile inside a redirected `CARGO_TARGET_DIR`). Anything else is
   INSTALLED. Judged on the CANONICAL path — `Origin::ambient` goes through
   `program::running_program`, which canonicalises — because a service alias and every job the daemon
   spawns through it are symlinks that lie about where they live and tell the truth about what they
   are.
3. `NXS_SERVICE_INSTANCE` — for a BUILD only, and it says WHICH working copy's instance, never
   whether there is one. For an installed binary it is not read at all, not even to be validated.

WHAT THIS MEANS IN PRACTICE. Install a checkout's own service by running the BUILD
(`cargo build && ./target/debug/nxs sync daemon install`), never the installed `nxs`. An install
links the alias to the binary that ran it, so that is also the only way a development service ever
runs a development build. Five verbs say so when the variable named an instance they could not give a
vote to — `sync daemon install`/`uninstall`, `sync bind`, `sync unregister` and every `init` path —
out of one sentence, `nxs_service::instance_env_ignored_note` (the ambient form is
`ignored_instance_env()`), which lives beside `shared_workspace_note` for that function's own reason.

WHY, so nobody restores the old rule by accident: direnv exports the variable for a whole working
copy, so before this every call made while standing in one — the installed `nxf` on the PATH included
— was a call of the development instance. Measured on the owner's machine on 2026-09-05: all three
registered services pointed at the same `~/.local/bin/nxs`, so no development service had ever run a
development build. It also restored itself, because `sync bind` registers the service.

THE ASYMMETRY THAT DECIDES EVERY EDGE CASE: a false `Build` verdict only leaves the environment the
vote it had before this rule existed; a false `Installed` verdict silently moves a development build
onto the production registry. So every question `Origin::of` asks may only widen `Build`, and a
process that cannot name its own binary is `Build`.

FOR TESTS: a test binary is a BUILD (`target/<profile>/deps/…`), so the variable still decides for
any subprocess a test starts, and every `nxs_test_support` convention is unchanged —
`PinHome` clearing it, `pinned_home_env()` setting it to `nexus-flow`. The ignored-instance note is
`None` in every test for the same reason, so it cannot make a suite's output depend on the
environment its runner was started in.
