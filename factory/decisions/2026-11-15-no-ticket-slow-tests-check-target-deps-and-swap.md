# 2026-11-15 · no ticket · 88-122 min for cargo test --all is NORMAL here (external HFS+ disk) — only a sudden drop below that baseline is worth diagnosing.

- Decided by: recorded in the project memory (`nxm`, key `slow-tests-check-target-deps-and-swap`); the record below names the owner wherever the owner decided
- Why: the reasons are in the record below, migrated unchanged from NEXUS_MEMORY.md when the decision log replaced it
- Instead of: the record below names the options not taken where the memory recorded them
- Topics: testing, build, performance

## The record

**THE BASELINE FIRST, because the diagnosis below is only meaningful against it.** The checkout
lives on an external USB disk formatted HFS+ (it used to be internal, APFS, PCIe). Measured:
creating 2000 small files takes 30.4 s there against 5.6 s internally (5.4x) — exactly the shape
of a Rust build. A full `cargo test --all` cycle is therefore **regularly 88 to 122 minutes**.
That is the NORMAL CASE, not a symptom. Anyone who starts counting files in `target/debug/deps`
because a run passed an hour is hunting a problem that is not there. The real consequence of this
baseline lives in [gates-run-in-ci-not-locally](2026-11-15-no-ticket-gates-run-in-ci-not-locally.md): full gates do not belong on this machine at
all.

DIAGNOSE ONLY WHEN IT GETS SUDDENLY MUCH SLOWER THAN THAT BASELINE, or when the disk is genuinely
filling up. On 2026-07-09 a full `cargo test` took >1h45m at a time when minutes were normal: each
CLI integration-test binary ran 2-10 min at ~0 CPU, stuck in uninterruptible disk wait. Two
compounding machine-level causes, neither of them the code:

1. `target/debug/deps` had ballooned to **901,522 files / 36+ GB**. The accumulation was NOT
   months old — nothing in it was older than 15 days, because churn here is roughly 60k files a
   day (parallel sessions, debug+release double builds, feature variants). **That is why
   `cargo sweep --time 15` cleans NOTHING here and `cargo sweep --maxsize <GB>` is the right
   lever** (2026-07-09: reclaimed 37.6 GiB; note it may empty `debug/deps` entirely while keeping
   newer release artifacts, so the next debug build is cold). Never sweep while a build or test is
   running.
2. The Mac was swap-bound: 11.7/12 GB swap used, the data volume 92% full (~18 GB free) so swap
   could not grow; load >20; `XprotectService` had burned 161 h of CPU, most likely rescanning the
   mass of freshly built binaries.

HOW TO APPLY: when a build or test here is *unbelievably* slow, do not debug the tests first — run
`/bin/ls -f target/debug/deps | wc -l` (use `-f`; a sorted `ls -lt` itself takes minutes at that
size) plus `sysctl vm.swapusage` and `df -h /System/Volumes/Data`. Also: piping a background test
through `tail -N > log` hides all progress (a 0-byte log until EOF) — poll with `ps` showing the
current `target/debug/deps/<test>-<hash>` child instead.

SPOTLIGHT IS THE ONE MITIGATION THAT KEEPS GETTING LOST. `target` was once a symlink to
`target.noindex` precisely so Spotlight would skip it. That arrangement did not survive the move
to the external disk: as of 2026-09-05 `target` in the main repo is an ordinary directory,
`target.noindex` does not exist, and the volume is indexed. Restoring it means recreating the
symlink (never a plain `target/` dir) or turning indexing off for the whole volume with
`sudo mdutil -i off`. Local git excludes for both names live in `.git/info/exclude`.
