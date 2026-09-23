# nxc agent sidecar

Standalone Node sidecar that spawns exactly one Claude Agent SDK `query()` session from a JSON
spec file, captures the **real** SDK session id, and — on a fresh run (no `resume`) — reports the
internal↔real session mapping back to `nxc` via `nxc session bind <session> <realId>` (never the
db directly). This is the role-runtime tracer bullet (nxc epic 6j6v.zenf, Task 1): it proves only
the SDK half of the design. `nxc` itself doesn't call into this yet (see Task 2 onward).

## Run it

```bash
cd agent-sidecar && npm install   # first time only
node src/main.mjs --spec <spec.json>
```

Set `NXC_SIDECAR_DEBUG=1` to dump every raw SDK message to stderr as it streams in — useful for
inspecting the exact message shape (this repo's tracer-bullet run is recorded in
`.superpowers/sdd/task-1-report.md`).

### Spec file shape

```json
{
  "session": "s-smoke",
  "resume": null,
  "role": "smoke",
  "message": "Reply with exactly the word: pong.",
  "systemPrompt": "You are a smoke test. Answer tersely.",
  "useClaudeCodePreset": false,
  "tools": [],
  "grantedTools": [],
  "permissions": "default",
  "cwd": ".",
  "env": {}
}
```

- `session` — the internal (nxc-owned) session id; used as the first argument to `nxc session
  bind` on a fresh run. Omit if you don't want a bind attempt.
- `resume` — a previously-captured **real** SDK session id to resume; when set, no bind is
  attempted (an existing mapping is being reused, not created).
- `useClaudeCodePreset` — when `true`, `systemPrompt` is *appended* to the built-in
  `claude_code` preset (`{ type: "preset", preset: "claude_code", append: ... }`) rather than
  replacing it outright.
- `tools` — what the ROLE AUTHOR declared. It sets the SDK's `options.tools` (the actual base
  toolset available to the session — an empty array means zero built-in tools) and feeds
  `options.allowedTools` (auto-allowed without a permission prompt, within whatever base set is
  active). A role declaring `tools: [Bash]` is genuinely restricted to `Bash` (see "Findings"
  below). **Absent entirely** means the author declared no intent: `options.tools` is left unset, so
  the SDK's own full default toolset applies.
- `grantedTools` — what the ENGINE added because it DEMANDS something of this session (nxf
  6j6v.kffm). A trigger that registers an expectation tells the persona, in its own system prompt,
  to end its turn with `nxc reply --thread <id>`, so it has to be able to run one. It is unioned
  into `allowedTools` always, and into `options.tools` only where `tools` is an array — an
  undeclared base toolset stays unset, because folding the grant in there would narrow the session
  from the SDK's full default set to exactly the grant. Absent (an older engine, or a trigger that
  demands nothing) means no grant, which is what every spec looked like before this key existed.

  It is a second key rather than a wider `tools` for exactly that reason: the two drive two
  different SDK options, and the defect it fixes was that one field was doing both jobs. A role that
  declared no `tools:` got the full base toolset and an EMPTY approval list, so its own obligatory
  `nxc reply` came back `"This command requires approval"` and the teardown answered in its name.
- `declarationHash` — which VERSION of `.nxs-personas/<role>.yaml` this session's `systemPrompt` was
  composed from (nxf 6j6v.pkw9). **A record, not a control**: the sidecar does not read it and
  nothing branches on it. It is here so that "did this session run under the rule I wrote?" is a
  comparison — the declaration folder lives in the working copy the agents themselves work in, so a
  branch switch can roll it back with nothing else reporting it, and the only way to establish that
  before this key was to probe the stored `systemPrompt` for the rule's text. Absent or `null` for a
  spawn with no declaration behind it (the `summarize` synthesizer, whose prompt comes from the
  channel), and for an older engine.
- `claudePath` — which Claude Code executable the SDK drives (`options.pathToClaudeCodeExecutable`).
  The **host** resolves it (`nxs` looks up `claude` on the `PATH`) and states it here; omit it to
  leave the SDK's own resolution in charge. Required in practice for the *shipped* sidecar — see
  "How it ships" below.

## How it ships

The bundle is **compiled into the `nxs` binary** (nxf 6j6v.smsz). `nxs` unpacks it on first use into
`<cache>/sidecar/<content-hash>/nxc-agent-sidecar.mjs` — `NXF_CACHE_DIR`, else `XDG_CACHE_HOME/nxf`,
else `~/.cache/nxf` — and hands that path to `node`. Nothing has to be installed beside the binary
and no `NXC_SIDECAR` has to be set; if the cache is not writable, `nxc` says so and names the way
out rather than failing silently.

It was a second file next to `nxs` in 0.52.0/0.53.0, and that could never arrive on an existing
machine: the code that places it ships *in* the new version, while the update is performed by the
old one. Embedded, the binary and its sidecar are one artifact, so an update is one file replaced.

`NXC_SIDECAR` remains the developer override, and a `cargo`-built binary with no bundle compiled in
falls back to `agent-sidecar/src/main.mjs` two levels up from `target/<profile>/`.

```bash
cd agent-sidecar && npm ci && npm run build   # → dist/nxc-agent-sidecar.mjs
```

`crates/chat/build.rs` embeds that file when it exists and embeds nothing when it does not, so a
contributor without Node still builds; `NXF_EMBED_SIDECAR=require` (set by CI and the release job)
turns the absent case into a failed build.

**Plain `npm ci`, never `--omit=optional`.** The flag was aimed at the
`@anthropic-ai/claude-agent-sdk-<platform>` packages (~245 MB — they carry the native `claude`
binary), but esbuild declares ITS platform binary optional too: omitting them skips the
integrity-pinned `@esbuild/<platform>` and esbuild's postinstall then fetches its binary from the
registry with no lockfile `integrity` check, putting a build tool of version-only provenance in the
middle of a chain whose whole point is sha256 + minisign.

Those SDK platform packages being absent from the *bundle* is also *why* `claudePath` exists:
bundled into one file with no `node_modules` beside it, the SDK cannot `require.resolve` that
native binary, so the host hands it the machine's installed `claude` instead. Same division of
labour as manufakt.io's sidecar, whose host passes `claudePath` for the same reason (nxf
6j6v.81v5).

## What a failed run means — and what a transcript gap does not

The last line the sidecar writes to `.nxs/agent-logs/<internal-session>.log` is the verdict on the
run:

```
sidecar done: role=… session=… real=…                          the turn finished, everything landed
sidecar done: role=… session=… real=… transcript=incomplete     the turn finished; its transcript has a hole
(no `sidecar done:` line at all, non-zero exit)                 the turn FAILED
```

The teardown itself runs four steps, always, in a fixed order: `nxc session bind`, the final
transcript flush, `nxc reply --thread <id> --if-unanswered <text>` (only when the spec carries a
`replyThread`), and — last — `nxc session ended <session>`. The last one is the only report of a
fact nothing outside this process can observe in time: it is the session saying its own process is
over, which is what lets a channel declared `working_tree: exclusive` open its next step on the
session being finished rather than on a message having arrived (nxf 6j6v.10yb). It comes after the
failure reply on purpose — the reply settles the thread's debt, and the announcement is what lets
the engine act on a set that is then genuinely settled.

Three things fail a run, and nothing else does:

- **the SDK stream itself** — the turn did not happen;
- **`nxc session bind`** — losing the internal↔real mapping is unrecoverable, because the next
  `resume` would then silently start a fresh conversation instead of continuing this one;
- **the transcript normalizer's final `flush()`** — an exception out of pure in-process code is a
  defect in `transcript.mjs` rather than the environment being briefly unavailable, and it says the
  entries themselves are wrong, not merely unwritten. Deliberately NOT lumped in with the write
  failures below, which are environmental and expected in the field.

The completion line is the machine-readable part of this: a fixed `sidecar done: ` prefix followed
by `key=value` tokens, with `transcript=incomplete` present only when the transcript has a hole in
it. Key off those tokens rather than the prose lines around them — the tokens are pinned by
`test/main-teardown.test.mjs`, the prose is not.

A failing `nxc transcript append` is **not** one of them (nxf 6j6v.jwgc). The first failure latches
transcript capture off for the rest of the run — no later flush is even attempted, so a stuck
`.nxs/db.sqlite` write lock cannot charge the turn 5s of `busy_timeout` at every flush point — and
the run says so twice: once at the moment it happens, and once as `transcript=incomplete` above.
The turn keeps going and delivers its answer, because the answer is posted from *inside* the
session (`nxc reply`) and does not travel through the transcript at all. Before this, the throw
unwound out of the sidecar's `for await`, abandoned the SDK generator and ended the turn mid-answer:
the caller got nothing back and waited out the workflow timeout.

One thing to know when reading a `transcript=incomplete` run: two different clocks read the
transcript as the sign that a session is alive, so such a run looks quieter than it is.

- The orchestrator's **step-liveness clock** (`crates/chat/src/liveness.rs`) measures progress as
  the session's transcript high-water mark and nothing else, so a latched-down run can be nudged
  while working normally.
- `nxc send --stream`'s **idle clock** (`crates/chat/src/stream.rs`) is reset by any activity — a
  message *or* a transcript entry — so it degrades rather than breaks: a session that keeps posting
  is still seen as alive. What it loses is the **knock**, the report of what the transcript last
  showed before the deadline, which falls back to "nothing in the transcript at all" — so the human
  it is meant to inform decides on no evidence instead of some.

## Thinking has to be asked for, or it arrives empty

The transcript's reasoning half needs `includePartialMessages` **and** a thinking display mode
(nxf 6j6v.w4wa). Without the second, `thinking_delta` frames still arrive — carrying zero
characters, the blocks redacted — which counts the same as never arriving and left the capture code
unexercised for a month. Probed on the pinned SDK, one reasoning-heavy prompt, ambient CLI auth,
everything else as shipped:

| session options | thinking captured | answer text |
| --- | --- | --- |
| no `thinking` option (what shipped before) | 0 chars | 3666 chars |
| `thinking: { type: 'adaptive', display: 'summarized' }` | 829 chars | 4086 chars |
| `thinking: { type: 'enabled', budgetTokens: 4096, display: 'summarized' }` | 1078 chars | 3156 chars |

The sidecar ships the middle one. `adaptive` is the model's own default policy — Claude decides
when and how much to think — so this asks to *see* thinking that already happens rather than buying
more of it; `enabled` would replace that policy with the older fixed-budget mode. `summarized` is
the only display on offer: there is no raw-passthrough mode on this path. None of the three models a
role can pin (fable/opus/sonnet) rejects it, and on a turn that does not think it is simply a no-op —
so an empty transcript of reasoning is now a statement about that run, not about the platform.

`setMaxThinkingTokens(N, 'summarized')` is *not* the way to do this, though the SDK also offers it:
control requests are only available in streaming input mode and the sidecar passes a plain string
prompt, and the method is deprecated in favour of the option above.

## Authentication

No `ANTHROPIC_API_KEY` is required. On a machine with an authenticated `claude` CLI session
(OAuth credentials under `~/.claude/`), `@anthropic-ai/claude-agent-sdk` picks up that same
authentication automatically — confirmed live: the smoke run's `system`/`init` message reports
`"apiKeySource":"none"` and a real response still comes back. If neither an API key nor a CLI
login is present, `query()` will fail to authenticate.

## This is a live smoke, not CI

Every `node src/main.mjs --spec ...` run spawns a **real** Claude Agent SDK session against the
real API/CLI auth on this machine — it costs real tokens/money and is not deterministic (model
output varies) or hermetic (it shells out to `nxc`, reads ambient `~/.claude` credentials and
plugin/skill config). It is not wired into `cargo test`/CI and must be run by hand to verify the
sidecar still works against the currently-installed SDK version. There is no throwaway spec
committed to the repo — create one ad hoc (see the shape above) and delete it when done.

## Dependency version note

The role-runtime plan's original draft pinned `@anthropic-ai/claude-agent-sdk` at `^0.1`, which
was a placeholder — the real npm history never published past `0.1.x` under that caret range in a
way that's still current; `latest` at the time this sidecar was built was `0.3.215`. `package.json`
pins that resolved version exactly (no caret) and `package-lock.json` locks it in full.

## Findings from the tracer bullet run

- `query({ prompt, options })` and the `Options` fields the brief assumed (`systemPrompt` incl. the
  `{ type: "preset", preset: "claude_code", append }` shape, `resume`, `permissionMode`,
  `allowedTools`, `cwd`, `env`) all matched `@anthropic-ai/claude-agent-sdk@0.3.215` as published —
  no field renames were needed.
- **`session_id` is a top-level field on every `SDKMessage` variant** (confirmed against the
  package's `sdk.d.ts`), not nested — `msg.session_id` from the brief's capture line is correct
  as written. The *last* message observed carries the same real session UUID as the very first
  (`system`/`init`) message in a fresh run.
- The spawned session is a **full Claude Code session**, not a bare model call: it runs
  `SessionStart` hooks and picks up this machine's user-scoped plugins/skills (e.g. `superpowers`)
  from `~/.claude`, even with `cwd` pointed at `agent-sidecar/`. The SDK has two distinct tool
  options — `allowedTools` (auto-allow list) and the separate `tools` (the actual base toolset) —
  and the original tracer-bullet run only wired `spec.tools` to the former, so a role's declared
  tools didn't actually restrict availability. Fixed in the same task: `main.mjs` now also sets
  `options.tools = spec.tools` (an empty array means zero built-in tools), so a role declaring
  `tools: [Bash]` is genuinely restricted to `Bash` — verified live by spawning a session with
  `tools: ["Bash"]` and confirming the `system`/`init` message's `tools` array excludes `Read`,
  `Write`, `Edit`, etc., and that the model itself reports `Read` unavailable. MCP-provided tools
  are outside `options.tools`'s reach by SDK design; that's an SDK behavior, not a sidecar gap.
- `nxc session bind` does not exist yet on this machine's installed `nxc` (`nxc session ...` ->
  `error: unrecognized subcommand 'session'`, exit 2). `main.mjs` wraps that `execFileSync` call in
  a `try`/`catch` so a fresh-run smoke still reports the captured session id even before Task
  2 lands the subcommand; once it exists, the call executes exactly as originally written.

## Task 2 smoke: env-stamp → in-session `nxc reply` (live)

Proves the *next* layer beyond Task 1: that env-stamping the spawned session with
`NXC_SESSION`/`NXC_ACTOR`/`NXC_ORIGIN`/`NXC_DB` and instructing it to run `nxc reply` via the
`Bash` tool actually lands a chat message under the role's identity. `nxc session bind` (Task 5)
is not needed for this smoke — `session` is simply omitted from the spec so no bind is attempted.

**There are TWO `nxc` binaries on this machine**: a stale, separately-installed release under
`~/.local/bin/nxc` (on `PATH` by default), and the current one at `target/debug/nxc` (a symlink to
`target/debug/nxs`, rebuilt from this branch). Always prepend the fresh binary's directory to
`PATH` — for every shell command below **and** inside the spec's own `env` block, so the in-session
`nxc reply` the SDK session runs also resolves to the fresh binary:

```bash
export PATH="/path/to/nexus-flow/target/debug:$PATH"   # do this first, every time
cargo build -p nxs                                     # if unsure the debug binary is current
```

### Step 1: workspace + channel + a message to reply to

> **Updated 2026-08-19 (`6j6v.dvyq` §3).** The recipe below used to mint a channel with
> `nxc channels create work` and capture its `m-…` id, because `work` was only a display name. That
> whole step is gone with the raw channels: `nxc channels` no longer exists, a channel is DECLARED,
> and `send --to <persona>` opens the direct conversation AND its thread and hands the thread id
> back. The rest of this report is the record of a run made before that, and is left as it was.

```bash
cd /tmp && rm -rf sk && mkdir sk && cd sk
nxs init --json --module flow --module chat
mkdir -p .nxs-personas
printf 'handle: bob\nsystem_prompt: You are bob.\n' > .nxs-personas/bob.yaml
TID=$(nxc send --to bob "please reply" --json | sed 's/.*"thread_id":"//;s/".*//')
```

The target is a NAME resolved against the declarations, and the reply address is the thread the send
hands back: `nxc reply --thread "$TID" "…"`.

### Step 2: run the sidecar with an env-stamp + a reply instruction

`spec2.json` (`session` intentionally omitted — see above; `env.PATH` copies the fresh-binary
`PATH` from the shell so the in-session `nxc` resolves the same binary):

```json
{
  "role": "bob",
  "message": "Run exactly this shell command and nothing else: nxc reply <MID> \"hi from bob\"",
  "systemPrompt": "You are bob. Do exactly what the message says using the Bash tool.",
  "useClaudeCodePreset": true,
  "tools": ["Bash"],
  "permissions": "acceptEdits",
  "cwd": "/tmp/sk",
  "env": {
    "NXC_SESSION": "s-bob",
    "NXC_ACTOR": "bob",
    "NXC_ORIGIN": "local",
    "NXC_DB": "/tmp/sk/.nxs/db.sqlite",
    "PATH": "<the fresh-binary PATH from the shell>"
  }
}
```

```bash
node agent-sidecar/src/main.mjs --spec spec2.json
```

### Step 3: verify

```bash
nxc search "hi from bob" --json
```

Observed (2026-07-19):

```json
[{"message_id":"m-01KXXG7SZVXA40Q8W5R1G779EM","channel_id":"m-01KXXG6HFW5NQ411H9JXPACAHV","sender":"local/bob","body":"hi from bob"}]
```

`sender` is `local/bob` — the `NXC_ACTOR`/`NXC_ORIGIN` env-stamp genuinely became the message's
identity, exactly as the design requires. **`refs` is absent from this output entirely** — `nxc
search --json`'s hit shape (`MessageHit`/`MessageHitView`: `message_id`, `channel_id`, `sender`,
`body`) never carried `refs`, independent of anything this smoke does. A direct read of the
underlying store confirms what actually landed:

```
$ sqlite3 /tmp/sk/.nxs/db.sqlite "SELECT sender, refs, body FROM messages WHERE body='hi from bob';"
local/bob|{}|hi from bob
```

### Finding: the ambient ­`NXC_SESSION` → `refs.session_id` return address is NOT wired yet

The stored `refs` is `{}` — empty, no `session_id`. This is not a bug in this smoke; it's the
expected state of the code today, confirmed by reading the source: `NXC_SESSION` is not read
anywhere in `crates/chat` (only `NXC_ACTOR`/`NXC_ORIGIN`, in `actor()`/`origin()`,
`crates/chat/src/cli.rs`). `nxc send`/`nxc reply`/`nxc ask` (that last one removed since, `6j6v.dvyq`
§3) only populate `refs.session_id` from an
explicit `--ref session_id=<id>` flag (`parse_refs`); there is no ambient fallback to an env var.
Since the spec's message told the model to run `nxc reply <MID> "hi from bob"` with no `--ref`
flag (per the brief), no `session_id` was ever going to be recorded — the env-stamp only reached
the message as far as `sender`, not as a return address.

This is exactly the gap **Task 6 (ambient caller-session resolution)** exists to close: `nxc`'s
write verbs need to read `NXC_SESSION` themselves and fold it into `refs.session_id` when the
caller doesn't pass `--ref` explicitly (mirroring how `actor()`/`origin()` already fall back to
`NXC_ACTOR`/`NXC_ORIGIN`). A second, narrower gap for whoever picks that up: even once `refs` is
populated, **no `nxc` CLI verb currently surfaces `refs` in its JSON output** — `search` (above)
and `inbox` (`InboxOut`) both omit it; only the in-process `Engine`/`facade::MessageView` (not
exposed over the CLI) carries `refs` today. Verifying Task 6 end-to-end through `nxc` alone will
need either a CLI surface change or a raw store read like the one above.

### Cleanup

The scratch workspace lives entirely under `/tmp/sk` (`rm -rf /tmp/sk` when done); no scratch
files or specs are committed to the repo.

## Skeleton smoke — observed run (nxf epic 6j6v.zenf, Task 8)

`agent-sidecar/scripts/skeleton.sh` is the epic's capstone live smoke: a scratch workspace + two
roles (`pm`, `coding`) seeded with a channel, kicked off with `NXC_WORKER=sidecar` — a real PM→coding
round-trip **with a forced follow-up question** that must resume the coding session with intact
context. Run it with `bash agent-sidecar/scripts/skeleton.sh` (no `ANTHROPIC_API_KEY` needed on a
machine with an authenticated `claude` CLI session; see the script's own header for the fresh-binary
`PATH` requirement).

### Pre-flight fixes over the epic's original literal script

> **Three of these five stopped existing on 2026-08-19 (`6j6v.dvyq` §3), and the script beside this
> report was rewritten accordingly.** Bug 1 (capturing a minted channel id) and bug 4 (joining both
> roles to that channel by hand) were both artefacts of a channel nobody declared; there is no such
> channel any more, and no `nxc channels` to make one. Bug 3 (nothing tells a role its incoming
> message's id) is answered by the surface itself: the text a persona is woken with now ENDS with
> the command that answers each open conversation, naming its thread (since nxf 6j6v.s46h with the
> body on STDIN: `nxc reply --thread <id> -`), so the discovery step both prompts used
> to open with is gone — and since 2026-08-27 (`6j6v.1gm9`) so is the verb it used: `nxc inbox` and
> `nxc read` are removed, on the finding that an agent is always pushed its message and never has to
> ask. Bugs 2 and 5 stand. The list below is left as the record of what that run found.

Three bugs were foreseeable from a careful read alone (caught before spending real API calls):

1. **`nxc channels create work` mints its own `m-…` id** — `work` is only the display name. The
   script now captures `channel_id` from the `--json` output and interpolates the REAL id into both
   roles' system prompts (via a `__CHANNEL_ID__` placeholder + `sed`, not a literal shell variable
   inside a quoted heredoc, since a role's YAML is read verbatim by the spawned session — it never
   re-expands `$CH`).
2. **`nxs init --json` with no `--module` flags defaults to `flow` only** — the script now passes
   `--module flow --module chat` explicitly.
3. **Nothing ever tells the coding role its incoming message's id** — `send`/`reply` hand the
   spawned/resumed session only the raw message body as its prompt (`TriggerRequest.message =
   body`), never a message id. Both roles' prompts now open with an explicit discovery step
   (`nxc inbox --json`, falling back to `nxc search "<phrase>" --json`) before ever replying.

Two more mechanical bugs surfaced only once the corrected script above was actually run (documented
here rather than guessed at up front):

4. **`nxc inbox`/`nxc search` are membership-scoped reads** (`crates/chat/src/store.rs`: both
   `inbox` and `search_messages` `INNER JOIN membership_adds` on the querying handle), but
   `nxc channels create` only auto-joins its OWN caller — never a role it names later. Without
   `nxc channels join <channel> local/pm` / `local/coding` up front, bug 3's fix returns nothing for
   either role. The script now joins both roles to the channel right after creating it.
5. **The brief's literal `ANTHROPIC_API_KEY` precondition (`: "${ANTHROPIC_API_KEY:?...}"`) would
   abort the script on this machine** before spawning anything — this machine authenticates via an
   existing `claude` CLI session (`apiKeySource:"none"`, confirmed in Task 1/2), no key set. Dropped
   rather than faked.

### A live-run-only finding: organic "ambiguity" is not reliably forced

The first live run used the epic's own wording almost verbatim ("Ambiguity is expected") and let the
coding role decide for itself whether to ask a question. It didn't: pm, when delegating to coding,
pre-empted the ambiguity itself ("a short friendly greeting, your choice of wording"), so coding saw
nothing to ask about and went straight to `hello.txt` — no question, no resume of coding, only a
single resume of pm (to close out coding's completion report). That round-trip is real (see below),
but does not satisfy the epic's `FORCED follow-up question that resumes the coding session` bar.

Fix: the "ask a question" behavior is now a **deterministic, turn-numbered rule** in coding's own
`system_prompt`, not left to the model's ambiguity judgment — turn 1 (a truly fresh session: no
earlier turn of its own in the conversation) ALWAYS asks exactly one clarifying question and does
NOT touch the filesystem; turn 2 (a resumed session — it already has an earlier turn) does the real
work using the answer. This removed the reliance on the model happening to judge the order
"ambiguous enough," and reliably reproduced the full required chain on the very next run (below).

### A second finding, worked around rather than silently routed past: `reply()` doesn't restamp a return address

`send()`'s ambient `NXC_SESSION` → `refs.session_id` stamping (`crates/chat/src/cli.rs`, `fn send`)
has no counterpart in `fn reply` — a reply's own outgoing message keeps `refs: {}` unless the caller
passes an explicit `--ref session_id=...`. That's fine for the FIRST hop (`send` stamps it
automatically), but a multi-hop bounce (order → question → answer → report) only stays resumable at
every hop if each reply that expects a further follow-up restamps its own session id explicitly.
Both role prompts now do this via `--ref session_id=$NXC_SESSION` (the ambient env var the sidecar
already stamps into every triggered/resumed session — no `nxc` code change needed to work around
it). This is a plausible gap in T7's own scope worth a look by whoever owns it next; see the Task 8
report for the precise recommendation (`crates/chat/src/cli.rs`'s `fn reply` vs `fn send`).

**Update (final-review fix round):** `fn reply` now ambient-stamps `refs.session_id` from
`NXC_SESSION` itself, mirroring `fn send` — the gap above is closed at the CLI level, and a role
prompt no longer needs to pass `--ref session_id=...` by hand for a chain to stay resumable. The
transcript below predates that fix and remains an accurate historical record of the workaround-era
behavior; a run of this same script today would show a `refs.session_id` on rows 5/6 too (each
reply now stamps its own sender's session, not just the ones that "expect a follow-up").

### The observed chain (second, corrected run — full round-trip proven)

Workspace `$WS` = a fresh `mktemp -d`; channel `$CH` = `m-01KXXPE9PT63E41N0NX52GA1XA`. Every row
below is a **direct `sqlite3` read of the `messages` table** (`nxc search`'s `--json` omits `refs`,
per the Task 2 finding, so this is the only way to verify `sender`/`refs.session_id` at each hop):

| # | sender | kind | refs.session_id | body |
|---|--------|------|------|------|
| 1 | `local/ckoch` (human) | info | *(unset)* | "Kick off: have coding create a file hello.txt containing a greeting." |
| 2 | `local/pm` | info | `m-01KXXPE9S0JB58F1FZV020M3RB` (pm's own session) | "Please create a new file named hello.txt in the project root containing a simple friendly greeting message." |
| 3 | `local/coding` | **question** | `m-01KXXPEFJ3ZT8KY2AKSVPRYBCF` (coding's own session) | "Should the greeting in hello.txt be addressed to anyone in particular, or just a generic friendly greeting like 'Hello, world!'?" |
| 4 | `local/pm` | info | `m-01KXXPE9S0JB58F1FZV020M3RB` (pm's own session) | "Just a generic friendly greeting, e.g. 'Hello, world!' is fine." |
| 5 | `local/coding` | report | *(unset — last hop, not needed)* | "Created hello.txt in the project root with the greeting 'Hello, world!'" |
| 6 | `local/pm` | info | *(unset — last hop)* | "Done: hello.txt created in project root with 'Hello, world!' greeting." |

`session_map` (also a direct read) confirms exactly two role sessions, each with a stable `real_sdk_id`
that every resume reused — never a fresh one after the first:

```
pm:     internal m-01KXXPE9S0JB58F1FZV020M3RB -> real 4eadd595-ac0e-4873-947f-c5bc24fd862e
coding: internal m-01KXXPEFJ3ZT8KY2AKSVPRYBCF -> real 37572ea3-c3cb-4cef-9e7f-817b09b72c3f
```

The successive `*.spec.json` the `SidecarWorker` wrote for each trigger (captured live, before the
next trigger overwrote it — see the caveat below) make the resume genuine and the hop-guard counter's
propagation visible directly, hop by hop:

```
pm      fresh   hop=1  resume=None                                    msg="Kick off: ..." (relayed as pm's own paraphrase to coding)
coding  fresh   hop=2  resume=None                                    msg="Please create a new file named hello.txt ..."
pm      resume  hop=3  resume=4eadd595-ac0e-4873-947f-c5bc24fd862e    msg="Should the greeting in hello.txt be addressed to ...?"
coding  resume  hop=4  resume=37572ea3-c3cb-4cef-9e7f-817b09b72c3f    msg="Just a generic friendly greeting, e.g. 'Hello, world!' is fine."
pm      resume  hop=5  resume=4eadd595-ac0e-4873-947f-c5bc24fd862e    msg="Created hello.txt in the project root with the greeting 'Hello, world!'"
```

`hello.txt` (created only on coding's SECOND, resumed turn — proven by the spec above: the Write
tool call happens after, and using the exact wording from, the resumed turn's message):

```
$ cat "$WS/hello.txt"
Hello, world!
```

Every proof point from spec §8.2 / the brief's Step 2 holds: (a) two roles spawned detached
(`session_map` — two distinct real SDK ids); (b) the follow-up question round-tripped and BOTH
sessions resumed with intact context — pm resumed to answer, coding resumed and only then wrote the
file, using the answer's own wording; (c) `sender`/`refs.session_id` is correct at every hop (table
above, direct store read); (d) no process was left running afterward (`ps aux | grep -i
"main.mjs\|claude-agent-sdk-darwin"` empty once the chain settled); (e) the PM's final message to the
human (`local/pm → info`, row 6 above) is a one-line, model-authored summary — "Done: hello.txt
created in project root with 'Hello, world!' greeting." — posted via an explicit `nxc reply ...
--kind report`/`info` call in the prompt, not a raw transcript dump or a mechanical last-line grab.

**The epic's completion criterion also names "roles are git-tracked."** This smoke's own
`.nxs-personas/*.yaml` live only inside the ephemeral `mktemp -d` scratch workspace, which this
script never `git init`s — so no declaration from either run was itself committed. The property the
criterion is actually asserting is architectural, and it holds independent of this smoke:
`Workspace::personas_dir()` (`crates/chat/src/workspace.rs`) resolves to
`<workspace-root>/.nxs-personas` — a sibling of `.nxs/`, which is the ONLY directory this platform
blanket-`.gitignore`s. Despite the shared dot prefix the two are different names, so `.nxs-personas`
is NOT matched by that rule (checked with `git check-ignore`), and a declaration folder created in a
real, git-tracked project (as opposed to this throwaway `/tmp` scratch workspace) is therefore
git-tracked by ordinary means, with no special-casing needed. This smoke demonstrates the runtime
mechanics against a disposable workspace; it does not itself exercise "commit a declaration and have
it survive", which is a property of `personas_dir()`'s placement, not of this script.

> The folder was `<workspace-root>/roles` until nxf 6j6v.dvyq moved it — the same reasoning
> transfers unchanged. A legacy `roles/` folder is still READ (and reported as legacy); nothing
> rewrites it.

**Log-verbosity caveat**: `.nxs/agent-logs/*.log` only carries `main.mjs`'s own two `console.error`
lines (`"bound ..."` and `"sidecar done: role=... session=... real=..."`) unless
`NXC_SIDECAR_DEBUG=1` is set — the SDK's own turn-by-turn message stream (tool calls, the model's
reasoning) is NOT captured there by default. The chain above was reconstructed from `messages` +
`session_map` (the durable, authoritative record) and from each trigger's `*.spec.json` snapshot
(captured live before the next trigger overwrote it — `SidecarWorker::trigger` re-creates both the
`.log` and reuses the SAME `.spec.json` filename, keyed by the internal session id, on every
resume — a later resume truncates/overwrites the prior turn's files in place, so anyone repeating
this observation needs to poll and copy proactively, not just `cat` at the end).

No orphaned processes and no leftover scratch state outside `$WS` (a fresh `mktemp -d` per run, never
committed to the repo).
