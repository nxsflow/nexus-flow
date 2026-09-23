# E4 — Continuous Sync, Client Side (Design Spec)

> **Date:** 2026-07-30 · **Status:** Approved (design)
> **Prerequisites:** E4-T4 (`bfpd`, the bidirectional push/pull pass) and `c42x` (the DynamoDB
> relay op-store, shipped in v0.35.0).
> **Items:** `kgn5` (bind: stream identity + endpoint persistence) · `n4dn` (the daemon) ·
> `p5sa` (the `HttpTransport` path-encoding bug, found by the consumer mid-design).
> **Upstream design note:** manufakt-io
> `docs/superpowers/specs/2026-07-26-web-oplog-sync-skeleton-design.md` §9a (PR #107) — this spec
> is the nexus-flow-side elaboration of §9a.1/§9a.2/§9a.4. Approach **(A)**.

## 1. Goal

Make sync **continuous** on the client, so a machine that has been idle is not read from while
stale.

`nxs sync run` already performs a complete bidirectional push+pull anti-entropy pass (two
watermarks, dedup by `op_id`; E4-T4). The gap is not the algorithm — it is that the pass only ever
happens when a human types the command. Push-on-write does not close it either, because **reads
happen long before writes**: `nxf next`, an analysis, creating an epic. The concrete failure this
spec removes: work on the Mac → continue on the web → return to the Mac two days later → an
analysis and a new epic get built on two-day-old local state.

The fix is a process that **pulls independently of local writes**. That process is a client-side
daemon; the relay stays serverless (Lambda, scale-to-zero), so "no permanent *server*" continues to
hold.

### Non-goals

- **WebSocket receive on the desktop** — `2xff`, blocked on this spec's daemon (§7).
- **A hard "sync-before-read" barrier.** Freshness is eventual: seconds after wake, far ahead of
  the moment an epic gets created. A barrier is possible but not planned.
- **Auth-based stream routing** — `6aza`/`ax54`, a separate thread. One endpoint serves all streams
  today (§9a.4).
- **Retiring the deferred DynamoDB stubs** (`g17z`/`n2qf`, superseded by `c42x`) — separate cleanup.

## 2. Architectural Decisions

The forks taken, each with its rationale and the rejected alternative.

1. **`stream_id` = `stream-` + `sha256(host/owner/repo)[..24]`, not the literal slug.**
   Every clone of a repo must land on the same stream with no random id to copy. Hashing a
   *normalized* remote makes that deterministic; including the **host** stops a fork on a different
   forge with the same `owner/repo` from silently sharing a stream. *Rejected:* using the literal
   slug (`nxsflow/manufakt-io`) as the id, which §9a.2 shows as its example. It is not path-safe —
   see `p5sa` (§6) — and the readable-id variant needs escaping rules whose edges collide
   (`a/b-c` vs `a-b/c`).

2. **A global default endpoint plus a per-workspace override, not one or the other.**
   Many repos sync to the same relay, so repeating the URL per workspace is friction; but a local
   relay for one workspace has to be expressible. Two levels with one precedence rule resolved in
   one place. *Rejected:* endpoint-at-bind only (repetitive), and persisting whatever
   `run --remote` was last handed (the endpoint becomes a side effect of a sync pass, and a
   freshly-bound workspace stays invisible to the daemon until someone runs a manual pass).

3. **The write-nudge is a touched file plus a short poll, not IPC.**
   A marker file next to the db is cross-process, needs no port, no socket lifecycle, no
   stale-socket reclamation, and no `cfg(unix)` fork in the code. It costs one cheap `stat` per
   second per workspace. *Rejected:* a Unix-domain socket (zero latency, but socket lifecycle and
   more test scaffolding), and no-nudge-at-all — the daemon comparing `MAX(rowid)` against
   `pushed_through` each tick, which needs a db handle per workspace per tick and satisfies the
   nudge requirement only implicitly.

4. **The interval is a safety net, not the steady-state mechanism.**
   Continuous polling is what `2xff`'s WebSocket receive exists to remove. So the interval is set
   lazily (300s) and the trigger sources sit behind **one** channel, so `2xff` adds a
   `Trigger::Remote` variant rather than rewriting the loop. Wake, not the interval, is what
   actually covers the two-days-away case.

5. **Cursor-pull remains the backbone even after WebSocket lands.**
   A WebSocket delivers only while connected. The two-days-away case is by definition an offline
   gap, which no fan-out bridges; it is closed by a catch-up pull on (re)connect, then live. Ops
   carry `op_id`/`seq`, so the overlap between catch-up and live delivery deduplicates
   idempotently — which is also the safety net for a dropped publish.

6. **`bind` becomes idempotent; a *changing* stream id needs `--rebind`.**
   Today binding is strictly one-time, so a wrongly-bound workspace is only fixable by recreating
   `.nxs/`. Re-running `bind` with an unchanged id is made a successful no-op (so `bind` is safe
   from scripts and `nxs init`), while a *different* id is still refused unless `--rebind` is
   explicit. The protection against a silent stream switch is kept; the cliff is removed.
   *Rejected:* leaving it one-time (no migration path off the old random ids), and dropping the
   guard entirely.

7. **Stream-id validation lives at the bind seam, not at load.**
   Rejecting a malformed id when it is written turns a runtime 404 into a bind-time error naming
   the fix. Validating on *load* would break existing, working bindings. See §6.

## 3. Component Map

`crates/nxs/src/sync.rs` (287 lines) becomes a directory module, so each unit has one purpose and
fits in context:

| File | Purpose | Depends on |
|---|---|---|
| `sync/mod.rs` | `SyncMeta` load/save, `bind`, `run`, and the shared `run_pass` | everything below |
| `sync/slug.rs` | remote URL → canonical slug → `stream_id`; id validation | pure, no IO |
| `sync/endpoint.rs` | the global default + precedence resolution | filesystem only |
| `sync/daemon.rs` | trigger sources, `Scheduler`, the loop | `run_pass` behind a trait |
| `nxs-service` (`crates/service/`) | the `~/.nexusflow` home, the registry, the lock, the heartbeat | filesystem only |
| `sync/launchd.rs` | plist rendering, install/uninstall/status | `launchctl` behind a trait |

Two moves this forces:

- **`registry.rs` out of `mcp/`.** The workspace registry (`~/.nexusflow/workspaces.toml`) is today
  `#[cfg(feature = "mcp")]`. The daemon must not hang off the MCP feature, so it moves to
  `crates/nxs/src/workspaces.rs` together with `write_new_exclusive`, re-exported under the old path
  so the MCP tests read unchanged. `directories` goes from optional to a normal dependency.
  **Superseded by 6j6v.5zst:** it moved once more, out of `crates/nxs` entirely and into the new
  `nxs-service` crate, together with the daemon's lock and heartbeat. Same reason one level further
  out — an embedding app links neither the MCP feature nor the umbrella CLI, and the service can
  only attend a workspace it has been told about. `crates/nxs/src/workspaces.rs` still exists and
  still re-exports under the old names, so the note above about the MCP tests holds unchanged.
- **`nxs_foundation::store::Store` gains the local-write marker** (§5.3).

## 4. `kgn5` — Bind

### 4.1 Stream identity

```
git remote get-url origin
    git@github.com:nxsflow/manufakt-io.git
    https://github.com/nxsflow/manufakt-io.git
    ssh://git@github.com:22/nxsflow/manufakt-io
        ↓ normalize: unify the three forms, drop user@ and port, strip .git, lowercase
    github.com/nxsflow/manufakt-io
        ↓ sha256 → hex[..24]
    stream-3f9a2c1d8b7e4a05c6d1e2f3
```

`slug.rs` is table-driven and pure. The `hex[..24]` idiom is already in the codebase — chat derives
its DM channel ids the same way (`crates/chat/src/cli.rs`), and `sha2` is an existing workspace
dependency.

**The colon is disambiguated by the scheme, not by guessing.** git's scp-like syntax
`[user@]host:path` has no port syntax at all — a port can only appear when a URL scheme was
present. So normalization records whether it stripped a `://` and permits the "drop an all-digits
segment as a port" branch *only* in that case; without a scheme the colon is always the path
separator. An earlier draft of this spec guessed from the digits alone, which silently collided two
distinct repos whose owner segment is numeric (`git@host:1234/repo` and `git@host:5678/repo` both
normalized to `host/repo`, hence to one `stream_id`) — the exact property the derivation exists to
provide. Numeric owner names are legal on GitHub and unrestricted on self-hosted forges, so this is
reachable, not theoretical. Caught in review of `9540`; regression-tested there.

No `origin`, or not a git repo at all ⇒ a **loud error** that names `--create` and
`--join` as the two ways forward. `bind` with no flags is the new default path (slug);
`--create`/`--join` keep their current semantics and their tests.

The result is path-safe by construction: `stream-` plus 24 lowercase hex characters.

### 4.2 Endpoint

One resolution function, `endpoint::resolve`:

```
nxs sync run --remote <url>                        one-shot override, never persisted
  > .nxs/sync.toml          endpoint = "…"         per-workspace override
    > ~/.nexusflow/config.toml  [sync] default_endpoint = "…"
      > loud error naming both ways to set one
```

- `nxs sync endpoint <url>` sets the global default; with no argument it prints the current one.
- `nxs sync bind --endpoint <url>` sets the per-workspace override.
- `run --remote` goes from **required to optional**. This is the only compatibility change to the
  existing surface.

`SyncMeta` gains `endpoint: Option<String>`, `#[serde(default)]` so a bind-only meta written by an
older binary still parses.

### 4.3 Registration

`bind` idempotently upserts the workspace into `~/.nexusflow/workspaces.toml` through the same
helper `nxs mcp install` uses. Without this the daemon never sees the workspace.

### 4.4 Rebind

| Situation | Behaviour |
|---|---|
| re-run, derived/given id **equals** the stored one | success, no-op; updates the endpoint if given, ensures the daemon is installed |
| re-run, id **differs**, no `--rebind` | `Conflict`, naming both ids and `--rebind` |
| `--rebind` | switches, and **resets both watermarks to 0** |

The watermark reset is a correctness requirement, not an option: `pushed_through` is a *local
rowid* and `pulled_through` is a *server cursor on the old stream*. Both are meaningless against a
new stream, and keeping them would skip history in both directions. Re-pushing is idempotent
(dedup by `op_id`), so the cost is one larger batch, once.

Prefix registration needs no special handling — the existing step 0 of `run` (`register_prefix`)
claims the prefix against whatever stream is bound, and adopts a reassignment as it already does.

Side effect worth naming: `--rebind` is also the **migration path** for workspaces bound to a
pre-`kgn5` random `stream-<ulid>`.

## 5. `n4dn` — The Daemon

### 5.1 Trigger layer

**As built, this is a single-threaded 1s poll loop, not the multi-thread/channel design
originally specified here.** The original design fanned trigger *sources* out across a dedicated
ticker thread (interval + wake) and a watcher thread (the nudge-marker poll), joined into the
`Scheduler` over one `mpsc` channel:

```
Ticker thread   ──▸ Trigger::Interval          300s — safety net, not steady state
     └── detects wake ──▸ Trigger::Wake        immediate, bypasses debounce
Watcher thread  ──▸ Trigger::Nudge(ws)         1s poll on .nxs/last-write mtime
        [2xff]  ──▸ Trigger::Remote(ws)        WS delta — one more variant, no rewrite
                         │
                    one mpsc channel  ──▸  Scheduler  ──▸  pass per workspace
```

What got built (`serve`, `crates/nxs/src/sync/daemon.rs`) is simpler: ONE thread runs a `loop`
that wakes every 1s (`TICK`), inline-checks the wake heuristic and every registered workspace's
`last-write` mtime, feeds `Scheduler::on_trigger` directly, and sweeps when `Scheduler::due()`
says so. No threads are spawned and no channel exists — `Trigger::Interval` is never even
constructed; `due()` instead reads `last_pass_at + interval` straight off the clock (§5.2), so the
variant exists in the enum only as the already-covered case in `on_trigger`'s match. Thread-per-
source turned out to buy nothing here: everything was already single-process, the debounce window
is measured in seconds, and a 1s poll is cheap enough (one `stat` per registered workspace) that a
second thread and a channel would only add synchronization surface for no behavioural gain — and
the simpler shape is *easier* to test: the scheduler tests in this file inject a clock and drive
`tick` directly, with no channel to drain and no thread to join.

The `Scheduler`/`Trigger` vocabulary itself is exactly as specified, so the claim this design was
built around still holds even though the delivery mechanism does not: a later WebSocket receive
(`2xff`) adds a `Trigger::Remote` match arm and a call to `on_trigger` from wherever the socket is
read, not a rewrite of the loop.

**Wake detection** needs no new dependency: each tick compares the monotonic elapsed time
(`Instant`) against the wall-clock elapsed time (`SystemTime`). A divergence beyond ~30s means the
machine slept ⇒ immediate pass. Testable with an injected clock.

### 5.2 Scheduler — the pure core

No IO, no threads, clock injected. This is where the debounce lives, **once** — no per-write
complexity leaks into the apps.

- A nudge sets `last_nudge = now`. A pass fires when `now - last_nudge >= debounce` (2s). Each
  further nudge in a burst pushes `last_nudge` forward, so a burst produces **one** pass at its end.
- A ceiling `max_wait = 10s`, measured from the *first* nudge of the burst, also fires a pass.
  Without it a continuous write stream (Yjs typing) would defer the push indefinitely. The burst
  test and the ceiling test are separate tests.
- Interval: `last_pass_at + 300s`. Wake: immediate.

Since the pass pushes everything pending at once, a keystroke burst coalesces into one push
regardless of how many ops it produced.

### 5.3 The local-write marker

`nxs_foundation::store::Store` gains `nudge_path: Option<PathBuf>` — a sibling of the db file,
`.nxs/last-write` — and a `dirty` flag set by `emit`. `Drop` touches the file best-effort, once per
process, and only when something was actually emitted. In-memory stores get `None`.

`emit` is the single central seam every local op of every product already flows through
(`nxf`/`nxm`/`nxc` → `Store::emit` → `ingest`), so this is one change, not N call sites.

The name is deliberately neutral: the substrate records "this replica wrote at T". It knows nothing
about sync; the daemon is currently its only consumer. Foreign ops merged via `apply` (a pull) do
**not** touch it, so the daemon's own pass cannot nudge itself into a loop. Even if a remap during
a pass did emit, the effect is bounded to exactly one extra pass, which then consumes the marker.

### 5.4 Pass execution

`sync/mod.rs` grows one `run_pass(ws, endpoint) -> Outcome`, called by both `nxs sync run` and the
daemon, so the push/pull logic continues to exist exactly once. In the daemon it sits behind a
trait, so tests can count passes without a relay.

### 5.5 Which workspaces the daemon serves

Each pass iterates the registry and **skips, without error**:

- a registered workspace that is **not bound** (no `.nxs/sync.toml`) — sync is opt-in per workspace;
- a bound workspace whose endpoint **does not resolve** (neither an override nor a global default).
  This is logged once per workspace per daemon run, naming `nxs sync endpoint <url>`, rather than on
  every pass — a missing endpoint is a configuration state to report, not an error to spin on;
- a registry entry whose path no longer holds a `.nxs/` (a stale or typo'd entry). Consistent with
  the registry contract: nothing treats an entry as pre-validated.

A skip is never a reason to fail the pass for the *other* workspaces.

### 5.6 Robustness

- **Errors stay local.** A broken workspace never kills the loop: it is logged, and that workspace
  backs off exponentially (300s → 600s → … capped at 30min), resetting on success. A dead endpoint
  therefore does not hammer.
- **One instance only.** `~/.nexusflow/sync-daemon.lock`, written `O_EXCL` with the pid; a lock
  whose pid is gone is reclaimed. This stops a hand-started daemon from double-syncing next to the
  launchd agent. Double-syncing would still be *correct* (dedup by `op_id`), but it wastes
  invocations and races the watermark save.
- **Liveness is about a PROCESS, not a number** (6j6v.0wvp). `kill(pid, 0)` only asks whether the id
  belongs to *a* process, and after the service dies the OS eventually reuses it — so `status`
  reported a service gone for weeks as running. The heartbeat already carried `started_at`;
  comparing it against the process's real start time (macOS `proc_pidinfo(PROC_PIDTBSDINFO)`, Linux
  field 22 of `/proc/<pid>/stat` plus `/proc/stat`'s `btime`) is what makes the answer about the
  process. No third answer was needed — a platform with no start-time API says `Unknown`, exactly as
  it already did for the existence question.
- **A per-pass wall-clock budget** (6j6v.25f6). The pull loop was capped only by
  `PULL_PAGE_CEILING` = 20,000 pages, and an empty page with a one-step cursor costs a relay
  nothing — so one workspace could hold the single-threaded sweep for days and starve every other
  workspace's sync AND its deadlines. `engine::PassBudget` is a question the loop asks once per
  page; the engine stays clock-free (all time lives here, injected and provable without sleeping),
  and `sync::WallClockBudget` answers it out of an `Instant`. Stopping is clean, exactly like the
  page ceiling: the watermark already sits at the resume point.
- **`daemon status` without IPC.** After each pass the daemon writes
  `~/.nexusflow/sync-daemon.json`: pid, `started_at`, `last_pass_at`, and per workspace `last_ok`,
  `last_error`, pushed/pulled. `status --json` reads that file — the same file-based idiom as the
  nudge.

  **Plus one question the file cannot answer** (6j6v.kvda): what launchd itself holds under this
  instance's label. `status` on macOS runs `launchctl print gui/<uid>/<label>` and compares the
  `path` it reports — the plist launchd was BOOTSTRAPPED FROM — against this instance's own plist.
  Those are two different facts, and on 2026-09-04 they were two different files: a test run had
  registered the production label from a `TempDir` that was deleted moments later, the real plist
  never loaded, and the machine's clock stood for five days behind a registration nothing looked
  at. Everything `status` had to say about it was "stale heartbeat — the process is gone".

  The comparison is still IPC-free in the sense that matters (no protocol, no socket, no running
  service required); it is one read of the machine's own launchd, and it is gated on the process's
  `$HOME` being the login session's — see §5.7 — so it cannot read a session it is not part of.

### 5.7 launchd

`~/Library/LaunchAgents/com.nxsflow.nexus-flow.plist` with `RunAtLoad` + `KeepAlive`,
`ProgramArguments = [~/.nexusflow/bin/nexus-flow]`, and stdout/stderr to
`~/.nexusflow/logs/service.{log,err.log}`.

**Renamed and re-pointed by 6j6v.8see**, when the sync daemon became THE nexus-flow service and the
clock moved into it. The label was `com.nxsflow.nxs.sync` and the program was the `nxs` binary with
`sync daemon` appended. Two things forced the change: macOS reads a process's displayed name off the
path it `exec`s, so a background item running `nxs` reads `nxs` — and the retired label has to be
booted out on install, or a machine that had the old agent runs both. `nexus-flow` is a fifth
`argv[0]` persona of the one binary (`nxf`/`nxm`/`nxc` are the others) meaning "run the service";
the link lives in `~/.nexusflow/bin` and is refreshed on every install.

**Bootstrapped with a bounded retry and then read back** (6j6v.0yrp). `install` writes the plist,
boots out its predecessors and its own label, and then attempts `bootstrap` up to five times with a
short growing wait (150/300/600/1200 ms, 2.25 s in total), booting the label out again between
attempts. A single attempt is inherently a race, and the build creates it: `self-update` tells the
user to run `install` immediately afterwards, which is exactly when launchd is still tearing the
previous job down and answers `Bootstrap failed: 5: Input/output error`. After a bootstrap that
returns 0, `launchctl print` is read back — exit 0 is a receipt for the request, not for the job —
and the receipt says `confirmed: true` only when the job launchd holds came from the plist just
written. A read-back that could not be performed is reported as unconfirmed, never as confirmed.

When every attempt fails, the error names the precondition that was false rather than launchctl's
errno: plist unreadable/empty/not-a-plist, program alias unresolved or pointing at nothing, missing
log directory, a volume under 64 MiB free, a label on launchd's disabled list, or another
registration already holding the label. The raw `launchctl` sentence is kept as the last line, and
the half-state the failure leaves (plist written, alias re-pointed, nothing loaded) is named at the
point of failure as well as by `status`.

**Only from the login session's own home** (6j6v.kvda). `launchctl bootstrap gui/<uid>` addresses
the login session of uid, whose home is what the user database says — never what `$HOME` says. So
`nxs_service::launchd::RealCtl` is constructible only when the two agree, and `install`/`uninstall`
construct it before writing anything. A pinned `$HOME` isolates files and not launchd: a black-box
test that ran the real installer registered the production label with its own `TempDir` paths in
the developer's real session, and that registration outlived the directory. The counter-proof lives
in `crates/nxs/tests/no_test_reaches_the_real_launchd.rs`, which builds the dangerous call in under
an instance name no machine runs and then asks the real launchd whether anything was created.

**What that rule costs, and the one way out** (review of PR #428). The check compares `$HOME` with
the passwd home, canonicalised — which resolves two SPELLINGS of one directory (`/tmp` vs
`/private/tmp`, a symlinked volume) and cannot resolve two genuinely different ones. A Mac whose
`$HOME` is redirected on purpose — MDM, a roaming profile, DLP tooling — is exactly that shape, and
for its owner the refusal is a false accusation: both paths are real and permanent, not a `TempDir`
about to vanish. Before this rule existed their install worked, so an unconditional refusal is a
regression for them.

`--allow-redirected-home` is that escape, on `install`, `uninstall` and `status`, and the refusal
names it. It is a **flag and not an environment variable**, deliberately: an env var is inherited by
every subprocess from every shell and every `.envrc`, which is precisely how this gate would rot
back into the hole it fills — one setting in a test harness and every test on the machine is
unprotected again, silently. A flag is typed once per invocation, cannot be inherited, and is
greppable; `no_test_names_the_redirected_home_escape` asserts that no test in this repo names it.

`status` carries the same flag for the same reason on the read side: `launchctl print gui/<uid>`
answers for the real login session whatever `$HOME` says, so from a redirected home an ungated read
would hold somebody else's registration against this process's plist path and report a
machine-dependent `FOREIGN` on a healthy Mac.

- `install` is idempotent: write the plist, `launchctl bootout`, then `bootstrap gui/$UID`.
- `uninstall` = bootout + delete.
- `bind` calls `install` unless `--no-daemon` is given.
- Non-macOS: `install`/`uninstall` fail loudly with "macOS only". `nxs sync daemon` itself runs in
  the foreground on every platform — that is exactly what launchd starts.

A systemd user unit for Linux was considered and dropped: two lifecycles and two test paths for a
need nobody has today.

### 5.8 Named instances (6j6v.gd9p)

Everything §5.6 and §5.7 name as a constant — the label, the home directory, the registry, the
lock, the heartbeat, the log directory and the alias — is derived from ONE value, an
`nxs_service::Instance`. The production instance is spelled exactly as above, byte for byte, so a
machine that already has a service keeps it unchanged; a named instance extends every one of them
with its own qualifier:

| instance name    | launchd label                | home directory     | alias              |
|------------------|------------------------------|--------------------|--------------------|
| `nexus-flow`     | `com.nxsflow.nexus-flow`     | `~/.nexusflow`     | `nexus-flow`       |
| `nexus-flow-dev` | `com.nxsflow.nexus-flow-dev` | `~/.nexusflow-dev` | `nexus-flow-dev`   |

**§5.6's "one instance only" is therefore per instance, and that is the whole isolation
mechanism**: the lock is a file inside the home directory, so separate homes are separate locks by
construction rather than by agreement. Two services run side by side and neither can boot the other
out — `RETIRED_LABELS` is swept for the production instance alone, because everything on it was
installed by the single unnamed service that existed before this.

**How each side learns which instance it is**, and the three answers differ because the constraint
does. §5.7's `ProgramArguments` carries exactly one element, so a launchd-started service cannot be
handed a flag: it reads its instance out of its own `argv[0]`, which is the alias name, which is the
instance name. A CALLER (`nxs`, `nxf`, `nxm`, `nxc`) is not started through an alias, and what
decides for it is **the binary it is running** (6j6v.cvpy): a binary at its installation location is
the production instance and nothing in its environment moves it, while a binary inside a build
directory is a development instance and `NXS_SERVICE_INSTANCE` — an `.envrc` per checkout, which is
the mechanism all three consuming repos already run — says WHICH. That distinction is what keeps the
installed command line on the machine's own service while standing in a working copy, and it is what
makes a development instance's alias point at the development build by construction: an install
links the alias to the binary that ran it. A job the service spawns is handed that variable
explicitly, because the spawner forces `argv[0]` to `nxs` for routing and thereby throws away the
name the parent read its own instance from — and it is obeyed there because the service's own
program resolves, through its alias, into the build directory it was installed from.

**One workspace, two instances is possible and is REPORTED, not prevented.** The registries are two
hand-editable files and no service is in a position to police another's. Both would attend the
workspace and both would read its deadline book, so a window that comes due can be started twice.
`nxs sync bind` says so at the moment the overlap is created, `nxs sync daemon status` on request,
and the running service once per workspace per run — all three out of one sentence
(`nxs_service::shared_workspace_note`).

### 5.9 Keeping the machine awake (6j6v.7q3r)

While any agent session is alive in any workspace it attends, the service holds a
`kIOPMAssertionTypePreventUserIdleSystemSleep` assertion, and releases it when the last one ends. It
is here rather than in the engine because such an assertion must be held by a live process and dies
with it: `nxc` as a CLI is over the moment it has sent, and an embedding app is gone exactly when the
assertion is wanted.

Liveness is read from the session claims chat already writes (`<workspace>/.nxs/agent-logs/*.pid`)
through `nexus_chat::worker::live_sessions_in` — the same pid-file answer `Worker::session_is_running`
gives for one session, asked of a whole workspace. The service does not learn that directory's
layout; it asks the module that owns it.

**The failure direction that matters is the release**, because an assertion never given back is a
machine that never sleeps again, met days later as a battery problem. Three guarantees, and the
tests are written in that order: the ordinary release when the last run ends, `Drop` on the keeper
for every early return out of the loop, and the kernel reclaiming it when the process dies without
running either. All three are proven against a real `pmset` (`crates/service/tests/wake_release.rs`
for the mechanism, `crates/nxs/tests/service_stays_awake.rs` for the running service).

Idle sleep only: not the display, not a closed lid, not a person choosing Sleep.

## 6. `p5sa` — The path-encoding bug

Found by manufakt-io during this design, reproduced against the v0.35.0 relay binary (13b1e62d)
with DynamoDB Local:

```
error: sync failed: sync transport:
http://localhost:18787/streams/nxsflow/manufakt-io/ops?since=0&limit=500: status code 404
```

`HttpTransport` builds all three URLs as `format!("{}/streams/{}/ops", base, stream.as_str())`
(`crates/sync/src/engine.rs`, push/pull/register) with no percent-encoding, so an id containing `/`
becomes several path segments and the axum route `/streams/:id/ops` never matches. The counter-test
with `nxsflow__manufakt-io` pushes and pulls cleanly, isolating the cause to path encoding — not
the relay, not the store.

**This does not block `kgn5` as specified here**, because §4.1's derivation is path-safe. It was hit
because §9a.2 shows the literal slug as its example. The bug is nonetheless real and shipped:
`--join <id>` accepts arbitrary operator input, and beyond the 404 a `#` in an id truncates the
path while a `?` injects query parameters.

Two levels of fix:

1. **Primary — validation at the bind seam.** A `stream_id` must match `[a-z0-9_.-]+`;
   `bind` (create/join/slug) refuses anything else and names a slug-safe alternative. The error then
   lands at bind time instead of as a runtime 404. *Why validation rather than encoding alone:* API
   Gateway and CloudFront — the stack the deployed relay sits behind per §9a.3 — normalize `%2F` in
   paths. A purely client-side encoding fix can be green locally against axum and still 404 against
   the deployed relay. That is not a bet worth taking, and it is only verifiable after a deploy.
2. **Secondary — defensive percent-encoding in `HttpTransport`.** Encode the path segment (keep the
   unreserved set `A-Za-z0-9-._~`, percent-encode every other byte) so an already-bound or
   hand-edited legacy id produces a well-formed request rather than a mangled path, and `#`/`?` can
   no longer inject. A small helper in `engine.rs`, no new dependency, table-driven test. The relay
   needs no change: axum's `Path<String>` percent-decodes.

Explicitly **not** done: validation when *loading* `.nxs/sync.toml`. That would break existing,
working bindings.

## 7. Relationship to `2xff` (WebSocket receive)

WebSockets replace the *trigger*, not the *daemon*. A WS connection needs a long-lived process on
the machine to hold it, rebuild it after wake, and fold the received delta into the local `.nxs` —
that process is `n4dn`, which is why `2xff` depends on it and not the reverse.

Four things must exist before desktop WS receive can be built, and three of them lie outside this
package:

| Prerequisite | Where |
|---|---|
| a long-lived client process | **this spec** (`n4dn`) |
| DynamoDB Streams → `rt.publish` fan-out bridge | manufakt-io `apps/web`, §9a.3 half (A) |
| a repaired `@aws-blocks/blocks` install | manufakt-io, §11 |
| a Rust WS client speaking the `Realtime` frame protocol | `2xff` |

Once `2xff` lands, `Trigger::Remote` joins the channel and the interval may relax further (e.g.
30min) — the loop itself does not change.

## 8. Testing

| Unit | How |
|---|---|
| `slug` | table-driven: the three remote URL forms → expected `stream_id`; no-remote → error |
| id validation | table-driven over the charset boundary, including `/`, `#`, `?`, space |
| `endpoint::resolve` | precedence matrix over all four levels |
| `Scheduler` | injected clock. The DoD case — 20 nudges in 3s → **exactly one** pass — runs in microseconds, no sleeps, no relay. Separate tests for the `max_wait` ceiling, the interval, and wake |
| backoff, lock, heartbeat | unit tests |
| workspace skips | unbound / endpoint-less / stale-path entries are skipped, and a healthy workspace in the same registry still syncs |
| `HttpTransport` encoding | table-driven unreserved/encode boundary, plus a regression pass against the axum app with an id that needs encoding |
| legacy load | a `sync.toml` holding a malformed id still loads |
| `launchd` | plist rendering as a golden; `launchctl` behind a trait, so install/uninstall are testable without touching the real session |
| rebind | id-equal no-op, id-differs conflict, `--rebind` resets both watermarks |

**Real verification (the DoD for both items):** two clones of the same repo against a local relay
converge with **no** manual `nxs sync` — proving the slug derivation binds both to one stream
without `--join`, and that the daemon closes the loop on its own.

## 9. Definition of Done

- Two clones of the same remote bind to the same `stream_id` **without a manual `--join`**.
- The bound endpoint is persisted per workspace and readable by the daemon.
- Existing `--create`/`--join` semantics are untouched and still test-covered.
- `nxs sync daemon` runs as a launchd agent and is started **when a workspace is bound**.
- It runs the bidirectional pass **periodically, on wake, and on write-nudge**, across arbitrarily
  many registered workspaces and endpoints.
- A change on replica A is visible on replica B **without a manual `nxs sync`**, within the interval.
- **Debounce is test-covered:** a burst of N writes produces **one** push pass, not N.
- A malformed `stream_id` is refused at bind time; `HttpTransport` encodes the path segment; the
  encoding regression is covered against the real axum app.
- Quality gates green (`cargo test`, `cargo test --release`,
  `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`), and the behaviour verified for
  real with two converging replicas.

## 10. Starting from a snapshot (6j6v.mxt2)

A fresh replica used to fold the whole history on its first pass. A **snapshot** lets it start from a
replica that already did: `nxs_sync::snapshot::{export, import}` at the embedding seam,
`nxs sync snapshot FILE` / `nxs sync bind --snapshot FILE` at the CLI. The module docs of
`crates/sync/src/snapshot.rs` and `crates/foundation/src/image.rs` carry the reasoning; this section
records the decisions.

- **Content.** The substrate's image — the WHOLE op log with its rowids, every table the exporting
  store's reducers declare in `Reducer::view_tables`, and that store's folded-through watermark — plus
  the stream id, `pulled_through` and an anchor, gzipped JSON, `FORMAT = 1`. The log rides along so a
  replica started from a snapshot is an ordinary replica, and views another engine folded are folded
  again from it instead of refused. Machine-local tables never ride along: a table is carried only if
  a reducer declares it a view, and the chat and memory stores have tests that make every table be
  classified one way or the other. The log is not compacted anywhere (E1 §9 still holds).
- **Version stand.** One relay number, `pulled_through` — a stream lives on one relay and a
  workspace syncs one endpoint. The **anchor** is the id of the op the relay held at that position
  when the snapshot was taken; `export` refuses when the replica does not have that op, and `import`
  requires the relay it will pull from to hold the same op there, else it pulls from 0 (a full pull,
  every held op skipped as a duplicate). A check of the numbering at one position, not a proof of the
  whole prefix.
- **Where it lives.** Not on the relay: it cannot fold, and while it authenticates nobody (6j6v.6aza)
  a snapshot planted there would be folded state nobody can check against its log. A CLI user hands
  over a file; an embedding host keeps the bytes where it trusts them.
- **Trust.** A snapshot is taken as given, log and views alike — a crafted file could forge ops as
  easily as views, so refolding would add no trust. What the import does guarantee is that it is
  total: a log or view cell of the wrong type, a lamport past `MAX_LAMPORT` (2^62), a duplicate op id
  or a header that disagrees with its body is refused before anything is written.
- **Engine versions.** Views are taken only on the same engine version, schema, product and tables
  (a column the exporter no longer reads is left behind; one the importer has and the image lacks
  means refold); a compatibility floor above the importer is refused by name. A recorded fold
  revision would make the version key independent of the migration checklist: 6j6v.y3r4.
- **Ordering at the CLI.** The prefix is registered on the still-empty store, then the snapshot is
  imported, and only then is `.nxs/sync.toml` written — once, with both watermarks and the endpoint —
  so a background service (which skips unbound workspaces) never starts a pass into a half-loaded log.
