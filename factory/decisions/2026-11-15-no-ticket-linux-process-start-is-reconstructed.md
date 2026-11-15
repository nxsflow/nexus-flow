# 2026-11-15 · no ticket · On Linux a process start time is RECONSTRUCTED from boot time plus ticks and can be a second off — never assert it against a wall clock.

- Decided by: recorded in the project memory (`nxm`, key `linux-process-start-is-reconstructed`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: service, linux, testing

## The record

A process start time is RECONSTRUCTED on Linux, not reported, and must therefore never be checked
against a wall clock to the millisecond.

`crates/service/src/heartbeat.rs::process_started_at` has two implementations. macOS/iOS reads the
instant DIRECTLY from `proc_bsdinfo` (`pbi_start_tvsec` + `pbi_start_tvusec`) and hits the real
`execve` to the microsecond. Linux computes it: field 22 of `/proc/<pid>/stat` is ticks since BOOT,
and the boot instant comes from the `btime` line of `/proc/stat` — in WHOLE SECONDS. The
reconstruction can therefore be about a second away from the real start, in either direction.

CONSEQUENCE FOR TESTS: an assertion like `process_started_at(kind) >= instant_before_the_spawn`
passes on the Mac and fails on Linux. That happened on 2026-09-03 in the work on the daemon status
report: locally on macOS all four gates green, `quality-gates` on the Linux runner red — and both
times the code the test accused was correct. If you bound a start time against the wall clock, give
the bound at least two seconds and write the reason beside it.

CONSEQUENCE FOR PRODUCTION: none. `born_together` allows a stamp LATER than the process start
without limit, and an earlier one by `START_TIME_SLACK` (60 s) — sixty times more room than this
reconstruction can be off. Two readers on the same machine also compute the same result (`btime` is
stable), so comparing two reconstructions is exact.
