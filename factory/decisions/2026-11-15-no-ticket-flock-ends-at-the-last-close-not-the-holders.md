# 2026-11-15 · no ticket · An flock ends at the LAST close of its open file description — any fork keeps it alive, so always LOCK_UN in an explicit Drop.

- Decided by: recorded in the project memory (`nxm`, key `flock-ends-at-the-last-close-not-the-holders`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: service, locking

## The record

An `flock` does NOT end when the holder closes its descriptor. It hangs off the OPEN FILE
DESCRIPTION, and the kernel drops it only when the LAST descriptor referring to that description
closes. `fork` — and `posix_spawn`, which is how `std::process::Command` starts every child on macOS
— copies the whole descriptor table, so a child that has been forked and has not yet reached
`execve` holds a second reference to it. `O_CLOEXEC` closes it there, but not before.

So in ANY multi-threaded process that also spawns children, this is a race:

    let f = File::open(p)?; flock(f, LOCK_EX|LOCK_NB); ... drop(f);   // releases NOTHING, sometimes
    let g = File::open(p)?; flock(g, LOCK_EX|LOCK_NB);                // EWOULDBLOCK

THE RULE: always `flock(fd, LOCK_UN)` in an explicit `Drop` before the descriptor closes. That
releases the lock on the description itself, so letting go depends on the holder and on nothing else
in the process. The kernel's release-on-last-close stays the backstop for paths that run no
destructor (a crash, `SIGKILL`), so nothing about crash safety is lost.

MEASURED on macOS on 2026-09-07 (nxf 6j6v.1zs2), 100 000 acquire/close/acquire cycles per row, with
one thread spawning `/usr/bin/true` beside them:

    spawners  release        re-acquire refused   first acquire refused
    0         close only     0 / 100 000          0 / 100 000
    1         close only     165 / 100 000        2 139 / 100 000
    3         close only     372 / 100 000        6 520 / 100 000
    1         LOCK_UN+close  0 / 100 000          0 / 100 000
    3         LOCK_UN+close  0 / 100 000          0 / 100 000

Remove `O_CLOEXEC`, so the child holds the copy for its whole life rather than until `execve`, and
the first-acquire refusal rate goes 2.1% -> 29%: the window is the fork-to-exec gap and nothing else.

WHERE IT BIT: `nxs-service`'s `ServiceLock` and `RegistryLock` both left release to the close, and
both module docs stated the false version as fact. It surfaced as `macOS tests (hosted)` answering
PASS/FAIL/FAIL/PASS on one identical tree — the neighbouring test in that same binary spawns
`sleep 30`. Both now carry an explicit `Drop`.

HOW TO TEST IT deterministically, without a race: `dup` the holder's fd, drop the holder, then try to
acquire. `dup` and `fork` both add a reference to the same open file description, so `dup` reproduces
every time what a fork reproduces 165 times in 100 000.
