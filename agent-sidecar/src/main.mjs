// nxc agent sidecar (role runtime, spec §5): run ONE Claude Agent SDK session from a spec file,
// capture the real SDK session id, stream the session's transcript out through `nxc transcript
// append` (0dbp), and — on a fresh run — record the internal↔real mapping through `nxc session
// bind` (never the db directly). Env-stamped by the caller so in-session `nxc` calls resolve their
// own caller session (NXC_SESSION) and act as the role (NXC_ACTOR).
import { query } from "@anthropic-ai/claude-agent-sdk";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import {
  resolveToolsOption,
  resolveAllowedTools,
  resolveModelOption,
  isSubcommandNotImplementedYet,
  isFlagNotImplementedYet,
  resolveSettingsSourcesOption,
  createFailureTracker,
  capFailureReply,
  resolveTurnTerms,
  retryDelay,
  REPLY_REMINDER_BOUND_MS,
  NXC_CALL_TIMEOUT_MS,
  STOPPED_EXIT_CODE,
} from "./spec-helpers.mjs";
import { createNormalizer, availabilityBoundary } from "./transcript.mjs";

function arg(name) {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const spec = JSON.parse(readFileSync(arg("--spec"), "utf8"));

/** **The stop door** (nxf 6j6v.b9nf): `SidecarWorker::stop_session` sends this process SIGTERM,
 *  and this is what the signal turns into — one `AbortController` that every `query()` in this
 *  file is handed, so that the turn it is waiting on, whichever one, ends through the SDK's own
 *  abort rather than by the process dying under it.
 *
 *  WHY A HANDLER AT ALL. Without one the default disposition applies and SIGTERM simply kills the
 *  process: no bind of the runtime session (so no later `resume` can find the conversation), no
 *  final transcript flush (up to `FLUSH_THRESHOLD` entries plus the open block gone), and no `nxc
 *  session ended` — the engine's liveness gate is left to notice the corpse on its own clock. The
 *  engine sends SIGTERM and not SIGKILL for exactly this reason: so that what follows can run.
 *
 *  WHAT A STOP IS NOT. Not a failure of the run (`recordFatal` is never called for it, and the
 *  attempt loop does not retry it — a retried stop would restart the very turn somebody just
 *  ended), and not a reason to spend a model call: the REMINDER is skipped, because a reminder
 *  tells a session it broke the answering rule and this one was told to stop.
 *
 *  WHAT A STOP DOES NOT DECIDE (fix round 2 of this item's review, Integrity #3): whether the
 *  thread still owes an answer. A SIGTERM is not only a withdrawal — it is also a system shutdown,
 *  a logout, a `docker stop`, a person's `kill <pid>` — and only the BOARD knows which. So the
 *  substitute post is not skipped on the signal; it is skipped on a discharged thread, which is
 *  what a withdrawal leaves behind and what nothing else does. See `postFailureReply`.
 *
 *  Installed HERE, before anything else is set up, so the window in which a SIGTERM still kills
 *  without a teardown is as small as it can be. A second SIGTERM is noted and changes nothing: the
 *  stop is already under way, and there is no faster ending this process can offer than the one it
 *  is performing. Exits with [`STOPPED_EXIT_CODE`] — see that constant for the number and why.
 *
 *  THE ENGINE NOW SENDS ONE (nxf 6j6v.27b9, owner decision of 2026-09-20): the tick signals a
 *  withdrawn session that is still pinning its claim five minutes after the stop, once, behind the
 *  same identity guard as the first. For THIS process it changes nothing, and deliberately so: a
 *  sidecar still here after five minutes is not in a teardown step — each `nxc` call there is
 *  bounded at `NXC_CALL_TIMEOUT_MS` — but waiting on the SDK's own wind-down, and whether a second
 *  signal may cut THAT short, with the SDK's descendants possibly still writing, is nxf
 *  6j6v.htdw's question and not one to settle in a handler. */
const stop = new AbortController();
const stopping = () => stop.signal.aborted;
process.on("SIGTERM", () => {
  if (stopping()) {
    console.error(`sidecar: session ${spec.session} was told to stop again — already stopping`);
    return;
  }
  console.error(
    `sidecar: session ${spec.session} was told to stop (SIGTERM) — aborting the running turn and ` +
      "tearing down",
  );
  stop.abort();
});

const systemPrompt = spec.useClaudeCodePreset
  ? { type: "preset", preset: "claude_code", append: spec.systemPrompt }
  : spec.systemPrompt;

const options = {
  systemPrompt,
  // `allowedTools` only auto-approves within whatever BASE toolset is active — it never shrinks
  // it. The base set is `options.tools` (below), a separate SDK field.
  //
  // The DECLARATION plus the engine's GRANT (nxf 6j6v.kffm) — see `resolveAllowedTools`. It was
  // `spec.tools ?? []`, and that `?? []` was the defect: a role that declares no `tools:` got the
  // full base toolset with nothing approved in it, so its own obligatory `nxc reply` was refused
  // and the teardown answered in its name.
  allowedTools: resolveAllowedTools(spec.tools, spec.grantedTools),
  permissionMode: spec.permissions ?? "default",
  cwd: spec.cwd,
  env: { ...process.env, ...spec.env },
  settingSources: resolveSettingsSourcesOption(),
  // Partial (token-level) messages are what the transcript normalizer coalesces into assistant and
  // thinking entries (0dbp). Required, not a nicety: extended THINKING arrives only via partials —
  // without this the captured transcript has no reasoning in it at all.
  includePartialMessages: true,
  // …and `includePartialMessages` alone is not enough for the thinking half (nxf 6j6v.w4wa). Over
  // the subscription/CLI auth path the `thinking_delta` frames DO arrive without this — they are
  // simply EMPTY, the blocks redacted down to zero characters, which is why the capture code sat
  // implemented-but-unproven for a month while two live smokes recorded not one thinking entry.
  // Asking for a display mode is what fills them: on the pinned SDK, same prompt, same auth, the
  // shipped configuration captured 0 chars of thinking and this one captured 829.
  //
  // `adaptive` because that is the model's OWN default policy — Claude decides when and how much
  // to think — so this asks to see thinking that already happens rather than buying more of it;
  // `enabled` would replace that policy with the older fixed-budget mode. `summarized` because it
  // is the only thing on offer: the SDK exposes no raw passthrough on this path, only
  // 'summarized' | 'omitted'. Safe for every model a role can pin — verified live against
  // fable/opus/sonnet, none of which rejects it; on a model that does not think it is a no-op.
  thinking: { type: "adaptive", display: "summarized" },
  // And this is what makes a SUBAGENT's half of the transcript exist at all. Per the SDK's own
  // docs (sdk.d.ts:1605-1611): "By default, only tool_use/tool_result blocks from subagents are
  // emitted (enough for a heartbeat counter). When true, the full subagent conversation is
  // forwarded so consumers can render a nested transcript." Without it a Task-spawned subagent
  // would contribute tool calls and nothing else — no reasoning, no reply text — which is exactly
  // the gap this epic exists to close (and the one the beads blueprint inherited by not setting it).
  forwardSubagentText: true,
};
if (spec.resume) options.resume = spec.resume;
// Restrict the actual base toolset to what the role declares (SDK: `tools?: string[] | {type:
// 'preset', ...}`; an explicit `[]` disables all built-in tools, per sdk.d.ts). Only apply this
// when the spec declares `tools` as an array at all — including `[]`, which is the genuine
// "no tools" state Task 1's own smoke relies on (a plain-text reply needs none). When `tools` is
// absent entirely (spec author declared no intent), leave `options.tools` unset so the SDK's
// default full toolset applies, same as before this fix — the engine's GRANT is deliberately not
// folded in there either (nxf 6j6v.kffm), because that set already contains it and turning "unset"
// into a list would narrow an undeclared role to the grant alone. See `resolveToolsOption`.
const resolvedTools = resolveToolsOption(spec.tools, spec.grantedTools);
if (resolvedTools !== undefined) options.tools = resolvedTools;

// Which model this role's session runs on (nxf 6j6v.jv8p). `spec.model` is already a finished SDK
// model id — the declared `fable`/`opus`/`sonnet` alias is resolved on the Rust side, so there is
// exactly one mapping table and it lives in the engine. Absent (the common case: the role declared
// no model) leaves `options.model` unset and the SDK's own default applies.
const resolvedModel = resolveModelOption(spec.model);
if (resolvedModel !== undefined) options.model = resolvedModel;

// Which Claude Code executable the SDK drives (nxf 6j6v.81v5). The SHIPPED sidecar is a single
// bundled file with no `node_modules` beside it, so the SDK's own resolution — a `require.resolve`
// of the optional `@anthropic-ai/claude-agent-sdk-<platform>` package that carries the native
// binary — finds nothing and `query()` throws before the first message. The host resolves the
// installed `claude` instead and states it here: resolving a NAME to an INSTALLED THING is the
// host's half of the division (the same one manufakt.io's sidecar takes `claudePath` for).
// Absent — an unresolvable `claude`, or a run from the source tree where the SDK's own resolution
// works — leaves the option unset and that resolution untouched.
if (typeof spec.claudePath === "string" && spec.claudePath !== "") {
  options.pathToClaudeCodeExecutable = spec.claudePath;
}

// Transcript capture (0dbp): the stream used to be consumed for its session id alone. Entries are
// buffered and handed to `nxc transcript append` in batches — one child process per SDK message
// would be absurd, and one per session would lose everything if the run died mid-way.
const normalizer = createNormalizer();
const transcript = [];
/** Flush after this many buffered entries, so a long run does not hold the whole transcript in RAM
 *  and so an unclean death (SIGKILL, a crash between flushes) can cost at most this many entries
 *  plus the open block — the threshold BOUNDS that hole, it does not remove it. */
const FLUSH_THRESHOLD = 32;

/** Tripped by the FIRST flush failure, never reset — the first-failure latch (nxf 6j6v.jwgc).
 *
 *  WHY A LATCH AND NOT "RECORD AND CONTINUE": the flush's own failure mode is a STUCK one. The
 *  realistic trigger is `.nxs/db.sqlite` — shared by flow, memory, chat AND sync — being held past
 *  `busy_timeout=5000` by another writer (a `nxs sync` pull applying a batch of ops in one
 *  transaction is the candidate), and a lock that just cost 5s of waiting is a lock that will still
 *  be there at the next flush point. Re-attempting at each of them would add up to 5s of dead
 *  waiting EVERY time, so a hundred-flush turn buys minutes of latency and still no transcript.
 *  One attempt, one verdict, then stop asking.
 *
 *  WHY A FLUSH FAILURE NO LONGER FAILS THE RUN (the open sub-question on 6j6v.jwgc, decided here):
 *  it is a WARNING. The epic's own risk model already says which way this falls — "losing
 *  transcript entries is recoverable" (see the teardown order below) — and the turn's actual
 *  product is not the transcript: the answer is posted from INSIDE the session via `nxc reply`, so
 *  by the time a flush fails the work has already happened and is already durable. An exit code is
 *  one bit, and spending it here would say "this turn did not work" about a turn that did. Nothing
 *  reads that bit today (`SidecarWorker::trigger` spawns detached and returns `Accepted`), so the
 *  cost is not paid now — it is paid by the FIRST consumer that ever waits on the sidecar, and the
 *  natural thing to do with a turn reported as failed is to run it again. An agent turn is not
 *  idempotent: the files, the messages, the PRs all happen a second time. Telemetry must not be
 *  able to order that. What replaces the bit is loudness that costs nothing: the line below at the
 *  moment of the failure, and `transcript=incomplete` on the `sidecar done:` line at the end, so
 *  the log states the hole instead of implying a complete record.
 *
 *  KNOWN CONSEQUENCE, ACCEPTED — and it reaches TWO readers, not one (PR #332 review, Integrity
 *  #2). Both take the transcript as the sign that a session is alive:
 *    - `crates/chat/src/liveness.rs` measures a step's progress as the transcript high-water mark
 *      and nothing else, so a run with the latch down looks quiet and can earn a nudge it does not
 *      need;
 *    - `crates/chat/src/stream.rs`'s idle clock is reset by any activity — a message OR a
 *      transcript entry — so it degrades rather than breaks: a session that keeps posting is still
 *      seen as alive. What it loses is the KNOCK, the report of what the transcript last showed
 *      before the deadline, which falls back to "nothing in the transcript at all" — the human it
 *      exists to inform then decides on no evidence instead of some.
 *  Both costs are bounded and recoverable; the behavior this replaces lost the whole turn, every
 *  time. */
let transcriptLatched = false;

function flushTranscript() {
  // No internal session to key the rows by (a bare spec, e.g. the smoke) — nothing to persist.
  if (!spec.session || transcript.length === 0) return;
  // Latch down: drop what is buffered and spawn nothing. Dropping rather than keeping is the point
  // — the entries can never be written now, and holding them would grow the buffer for the rest of
  // the run to no purpose.
  if (transcriptLatched) {
    transcript.length = 0;
    return;
  }
  try {
    // JSON-lines on stdin, one entry per line: the T1↔T2 wire contract (see transcript.mjs). The
    // buffer is cleared as part of ATTEMPTING the write, including on the failure path below —
    // re-queuing entries would double-write everything a later, real append accepts. A flush that
    // never even started (the guard above) leaves the buffer untouched.
    //
    // Building the batch is INSIDE the try, not before it (PR #332 review, Code Quality #2).
    // Nothing the normalizer builds today can make `JSON.stringify` throw — every value on an
    // entry comes from JSON the SDK already parsed — but with the build outside, a throw from it
    // escaped this function entirely: mid-run that reaches the loop's catch and gets reported as
    // "the SDK stream failed", and it takes the turn down with it in exactly the way 6j6v.jwgc
    // exists to prevent. A batch that cannot be serialized is a transcript failure like any other,
    // so it belongs on the latch, under its own name.
    const input = `${transcript.map((entry) => JSON.stringify(entry)).join("\n")}\n`;
    transcript.length = 0;
    execFileSync("nxc", ["transcript", "append", "--session", spec.session], {
      env: { ...process.env, ...spec.env },
      input,
      // Bounded like every other `nxc` call on this path — see `NXC_CALL_TIMEOUT_MS`.
      timeout: NXC_CALL_TIMEOUT_MS,
      // stdin is a pipe because that is how the entries travel; stdout/stderr keep the same
      // discipline as the `session bind` call below (stdout live, stderr piped so we can read it).
      stdio: ["pipe", "inherit", "pipe"],
    });
  } catch (err) {
    const stderr = err.stderr ? err.stderr.toString() : "";
    if (stderr) process.stderr.write(stderr);
    transcriptLatched = true;
    // The name-matched shape — clap's exit code 2 + "unrecognized subcommand 'transcript'" — is a
    // pre-Task-2 `nxc` that simply has no such subcommand. It gets its own wording because it is a
    // different DIAGNOSIS, not a different outcome: both latch, both warn, neither is fatal. Match
    // on the NAME and nothing looser, so a typo'd or renamed call site is still reported as a
    // failure rather than excused as an old binary. (A batch larger than the pipe buffer also
    // raises EPIPE here, because such an `nxc` exits without draining stdin; `err.status` is still
    // 2 in that case, so this check keeps working without an EPIPE special case.)
    if (isSubcommandNotImplementedYet(err.status, stderr, "transcript")) {
      console.error(`sidecar: nxc transcript append skipped (subcommand not implemented yet): ${err.message}`);
      return;
    }
    console.error(
      "sidecar: the transcript flush failed — transcript capture is now OFF for the rest of this " +
        `run and its transcript is INCOMPLETE. The turn itself is unaffected: ${err.message}`,
    );
  }
}

let realSessionId = spec.resume ?? null;

function bindRealSession() {
  if (!spec.resume && realSessionId && spec.session) {
    // Task 1 tracer bullet proves the SDK half only — `nxc session bind` doesn't exist yet (that
    // lands in Task 2). Attempt the callback (so this code needs no changes once it does), but
    // narrowly: catch and log-and-continue ONLY the specific "subcommand doesn't exist yet" shape
    // (clap's exit code 2 with an "unrecognized subcommand 'session'" stderr, as observed on this
    // machine's pre-Task-2 `nxc`). Any OTHER failure — a real validation error once `session bind`
    // exists, an argument-order mismatch, a broken PATH, etc. — must propagate as a genuine fatal
    // error instead of being silently swallowed. Do NOT widen this catch to a blanket try/catch;
    // that would hide real bugs once this call is exercised for real (Task 2 onward).
    try {
      execFileSync("nxc", ["session", "bind", spec.session, realSessionId], {
        env: { ...process.env, ...spec.env },
        timeout: NXC_CALL_TIMEOUT_MS,
        // stdout stays live (parity with prior behavior); stderr is piped so we can inspect its
        // text below instead of just its presence — inheriting it would leave err.stderr null.
        stdio: ["inherit", "inherit", "pipe"],
      });
    } catch (err) {
      const stderr = err.stderr ? err.stderr.toString() : "";
      if (stderr) process.stderr.write(stderr);
      const subcommandNotImplementedYet = isSubcommandNotImplementedYet(err.status, stderr);
      if (!subcommandNotImplementedYet) throw err;
      console.error(`sidecar: nxc session bind skipped (subcommand not implemented yet): ${err.message}`);
    }
  }
}

/** Every failure is written to stderr the moment it happens, in causal order, so a later teardown
 *  throw can no longer make an earlier one invisible; the tracker decides which one escapes. The
 *  "did it fail" boolean lives in the tracker (not in the truthiness of the error value) so a falsy
 *  rejection cannot be mistaken for success — see `createFailureTracker`. */
const failures = createFailureTracker();
function recordFatal(err, what) {
  process.stderr.write(`sidecar: ${what} failed: ${err?.stack ?? err}\n`);
  failures.record(err);
}

/** **The coordinator's terms for this turn** (nxf 6j6v.ntp9) — how many times a session that still
 *  owes its thread an answer may be resumed and reminded before the sidecar posts in its place (nxf
 *  6j6v.gh7f), and how many times a run whose RUNTIME never let the agent think may be started
 *  again.
 *
 *  Both used to be decided here. They are decided in the ENGINE now and travel on the spec, because
 *  a rule that lives in this file holds for the bundled sidecar and for no other worker — see
 *  `resolveTurnTerms` and `crate::worker::TurnTerms`.
 */
const TERMS = resolveTurnTerms(spec);

/** **Did the agent ever get to think?** (nxf 6j6v.553s question (b)).
 *
 *  `false` means the MODEL produced nothing on this spec — the shape a `529` at the door takes. That
 *  is not the agent failing to answer; it is the agent never having run, and it is the ONE state in
 *  which starting the turn over costs nothing, because nothing has happened yet.
 *
 *  Cumulative across retries on purpose: once any attempt has produced model output, this turn has
 *  done work, and an agent turn is not idempotent — the files, the messages, the pull requests all
 *  happen a second time. From that point a failure is a MID-TURN TEAR, which is retried never and
 *  reminded like any other unanswered turn (the session exists and can report what it did).
 *
 *  **The predicate is [`isAgentOutput`], NOT "a message arrived", and the difference is the whole
 *  feature** (independent review of PR #378, Integrity #1). This flag was set from every
 *  `SDKMessage`, and the FIRST message of every run is `SDKSystemMessage {type:'system',
 *  subtype:'init'}` — emitted by the CLI at session start, before any model call. So it was true by
 *  the time a `529` could possibly happen, the retry below could never fire, and the reminder went
 *  on resuming straight back into the outage. */
let agentEverThought = false;

/** Whether `msg` is the MODEL PRODUCING OUTPUT, as opposed to the runtime talking about itself.
 *
 *  Two variants qualify, and the list is closed on purpose (`sdk.d.ts`, pinned
 *  `@anthropic-ai/claude-agent-sdk` 0.3.x):
 *
 *  * `SDKAssistantMessage {type:'assistant'}` — a model turn. Side effects are possible from here on,
 *    because a `tool_use` block lives in exactly this message.
 *  * `SDKPartialAssistantMessage {type:'stream_event'}` — the same turn arriving token by token.
 *
 *  Everything else is deliberately excluded, and two exclusions are load-bearing rather than
 *  incidental:
 *
 *  * `type:'system'` — `init`, and `api_retry` (which the SDK emits WHILE it retries a failed API
 *    request). Both are the runtime describing itself, and `init` in particular precedes the first
 *    model call.
 *  * `type:'user'` — a tool RESULT, which can only follow an `assistant` tool_use that already set
 *    this flag, so nothing is lost. Excluding it is what keeps a RESUME honest: the SDK declares
 *    `SDKUserMessageReplay` with a shape identical to `SDKUserMessage` — same `type:'user'`, no
 *    discriminator — so replayed history would otherwise read as this turn's work. There is no
 *    replay variant of `assistant` or `stream_event`, which is why those two are safe.
 */
function isAgentOutput(msg) {
  return msg?.type === "assistant" || msg?.type === "stream_event";
}

/** **Did the RUNTIME fail this attempt?** — as opposed to the turn simply ending.
 *
 *  Read in three places, and it has to be its own fact rather than "did the stream throw": it is
 *  what decides whether to retry, whether the reminder is warranted, and which escalation text goes
 *  out. The reminder (nxf 6j6v.gh7f) asks a session to obey a rule it did not obey; a run whose
 *  runtime never ran it broke no rule, so reminding there asserts an omission that did not happen and
 *  spends the one attempt on a resume into the same failure. That was reachable: a RESUMED session
 *  (`spec.resume` set) carries a session id to resume before the first frame ever arrives.
 *
 *  **A retryable API failure is NOT a thrown error** (independent review of PR #378, Integrity #1),
 *  which is the other half of why this is a variable and not a `catch`. Per the pinned `sdk.d.ts`,
 *  the SDK retries internally, reports each attempt as `SDKAPIRetryMessage {subtype:'api_retry'}`,
 *  and when it gives up terminates the stream NORMALLY with `SDKResultError {type:'result',
 *  subtype:'error_during_execution', is_error:true}` — `for await` simply ends and no `catch` runs.
 *  A `529` therefore used to leave `failures.hasFailed()` false, the run exited 0, and the requester
 *  got the non-escalating "this session ended without ever posting a reply" for a session that had
 *  died at the door. [`resultFailure`] is what turns that ending back into the failure it is. */
let runtimeFailed = false;

/** The failure an `is_error` result message states, as an `Error` the teardown can report.
 *
 *  `api_error_status` is where an HTTP status actually surfaces — the `529` itself — so it is named
 *  first when present; `errors` carries the runtime's own sentences. Both are optional on the
 *  declared shapes, so both are guarded. */
function resultFailure(msg) {
  const status = msg.api_error_status != null ? ` (api status ${msg.api_error_status})` : "";
  const detail =
    Array.isArray(msg.errors) && msg.errors.length > 0 ? `: ${msg.errors.join("; ")}` : "";
  return new Error(`the run ended as ${msg.subtype ?? "an error"}${status}${detail}`);
}

/** **The last class the runtime named for a failed assistant turn** (nxf 6j6v.npy3), or null.
 *
 *  `SDKAssistantMessage.error` is a value from a closed union, and it is the one place the runtime
 *  says WHICH kind of failure this was. It is kept here rather than re-derived from the transcript
 *  entries, for `runtimeFailed`'s reason: the teardown branches on it, so it has to be a fact about
 *  the run and not a second reading of what was written down. */
let lastAssistantError = null;

/** **What the terminal `result` said, as text** (nxf 6j6v.npy3) — `errors` joined, which is where
 *  `SDKResultError` actually carries the runtime's own sentences (it has no `result` field; see
 *  `sdk.d.ts`). Used for the DETAIL a human reads and for nothing that is decided. */
let lastResultText = null;

/** **The availability boundary this run ended at** (nxf 6j6v.npy3), or null for every run that did
 *  not hit one — which is every ordinary run.
 *
 *  Set once, after the stream loop, from the two structured signals the runtime gives
 *  ([`availabilityBoundary`]). Three teardown steps read it, and each of them does something it
 *  would otherwise get wrong:
 *
 *  * the REMINDER does not run — the session broke no answering rule, and resuming it would spend a
 *    paid model call going straight back into the same closed window;
 *  * the SUBSTITUTE POST does not go out — it would say "I cannot carry this out" in the agent's
 *    name, which is false, and settle a debt the session is coming back to discharge;
 *  * `nxc session interrupted` DOES go out, which is the whole way back: it is what puts the hold
 *    into the operational view and arms the return. */
let boundary = null;

/** How many times the turn was started again because the runtime had not run it — `0` for every run
 *  that reached the model the first time, which is every ordinary run. Reported on the completion
 *  line for `transcript=incomplete`'s reason: a token on every line is a token readers learn to skip
 *  past, and this one has to be noticed when it is there. */
let retriedRuntime = 0;

/** Sleep, for the backoff between runtime retries. Bounded by [`retryDelay`]; nothing else in this
 *  file waits. */
function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}


/** What the reminder round did, for the completion line and for what gets posted afterwards.
 *  `null` = it never ran (no obligation, or the board could not be read). */
let remindOutcome = null;

/** **The threads this session commissioned itself and is still waiting on** (nxf 6j6v.hw2t) — set by
 *  `remindBeforeSpeakingForTheAgent` when it recognises the THIRD case, empty for every other run.
 *
 *  It is read once more by `postFailureReply`, which must post nothing at all in that state: the
 *  thread deliberately stays owing an answer, because the session that owes it is coming back for
 *  it. Carried in a variable rather than re-read because the second read would be a second `nxc`
 *  call answering a question this teardown already asked. */
let waitingOnSubRound = [];

/** **What the board says about this session's debt** (nxf 6j6v.gh7f) — read from `nxc status
 *  --thread <id> --json`, which answers BOTH questions this teardown has in one call: does the
 *  thread still owe an answer, and what did this session commission out of it that is still open.
 *
 *  Reading rather than assuming is the whole point: the sidecar cannot see an `nxc reply` the agent
 *  ran through its own Bash tool, and guessing from the message stream would be prose-matching on a
 *  tool input. The ENGINE holds the register (`expects_reply_from`) that decides this, and it is the
 *  same register `reply --if-unanswered` consults.
 *
 *  `null` on ANY failure — an older `nxc`, a locked db, output that does not parse. That is the
 *  fail-safe direction: an unreadable board skips the reminder entirely and falls straight through
 *  to the behaviour that existed before this ticket, rather than spending a model call on a guess.
 *
 *  The thread named in `spec.replyThread` is by construction the one THIS trigger declared an
 *  expectation on for THIS role (see `TriggerRequest::reply_thread`: it is set only when the trigger
 *  wrote `expects_reply_from` itself, and it never overwrites somebody else's quorum), so anything
 *  outstanding there is this session's own debt and needs no identity match.
 *
 *  **`waitingOn` is READ, not derived here** (nxf 6j6v.hw2t). This function used to compute "what
 *  did I hand out" itself, by filtering the thread list for open children of `spec.replyThread` —
 *  a second copy, in another language, of a rule the engine now states once
 *  (`crate::awaiting::own_open_sub_round`, surfaced as `waiting_on_sub_round`). The engine's
 *  version also weighs each child's OPENER, which is what keeps a channel supervisor's own fan-out
 *  out of the answer; the filter here could not have known that. An older `nxc` without the field
 *  reads as an empty list, which is the pre-item behaviour exactly.
 *
 *  @returns {{owes: boolean, waitingOn: string[]} | null}
 */
function readReplyDebt() {
  try {
    // `--json` last, so the call reads as `nxc status …` — the verb is the first token, which is
    // what every other call site here does and what a reader (and the log) keys on. The flag is
    // clap-global, so its position carries no meaning.
    const out = execFileSync("nxc", ["status", "--thread", spec.replyThread, "--json"], {
      env: { ...process.env, ...spec.env },
      encoding: "utf8",
      timeout: NXC_CALL_TIMEOUT_MS,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const threads = (JSON.parse(out).operations ?? []).flatMap((op) => op.threads ?? []);
    const mine = threads.find((t) => t.thread_id === spec.replyThread);
    if (!mine) return null;
    return {
      owes: (mine.outstanding ?? []).length > 0,
      // What this session commissioned out of its own thread and is waiting on, as the ENGINE
      // derives it — see this function's doc for why the filter that used to stand here is gone.
      waitingOn: mine.waiting_on_sub_round ?? [],
    };
  } catch (err) {
    const stderr = err?.stderr ? err.stderr.toString() : "";
    if (stderr) process.stderr.write(stderr);
    console.error(`sidecar: could not read the reply debt for ${spec.replyThread}: ${err?.message ?? err}`);
    return null;
  }
}

/** The text a reminded session is resumed with. It says the same two things the session's own system
 *  prompt says (nxf 6j6v.553s part 1: a turn ends in exactly two ways) — deliberately, because the
 *  measured failure is not a session that was never told, it is a session that was told and missed
 *  it. In the proving ground a PM read the rule and wrote "I'll keep waiting for the background task
 *  to notify me rather than ending my turn prematurely", then ended anyway.
 *
 *  **The commissioned-work paragraph is gone with nxf 6j6v.hw2t, and its absence is the point.** It
 *  used to say "do not wait for it — say what you handed out", which is a progress note in the shape
 *  of a finished one, and it is the very instruction that produced the measured false alarm. A
 *  session that IS waiting on its own round no longer reaches this text at all — the branch above
 *  returns before the reminder — so the only reader left here is one that owes an answer and has
 *  commissioned nothing, and it needs no paragraph about work it did not hand out.
 *
 *  **The body comes in on STDIN since nxf 6j6v.s46h, for the same reason and by the same wording as
 *  `role.rs::reply_obligation`.** "It says the same two things the system prompt says" is not a
 *  stylistic note: this text arrives at the ONE moment a session is most likely to copy a command
 *  line literally — its turn is ending and it has been told to do that and nothing else. Showing
 *  `"<your result>"` here would hand a verdict's backticks and `$(…)` to the shell at exactly that
 *  moment. The quoted `EOF` is spelled out because `<<EOF` without the quotes still expands. */
function reminderMessage() {
  return (
    `Your turn is ending and thread ${spec.replyThread} has no answer from you. A turn may not end ` +
    "without one: until it exists your caller has no result, and the round it commissioned counts " +
    "as open.\n\n" +
    "End it now. Give the message on STDIN, so that nothing in it is evaluated by the shell — the " +
    "quotes around EOF are what stop that:\n\n" +
    `nxc reply --thread ${spec.replyThread} - <<'EOF'\n` +
    "<your result>\n" +
    "EOF\n\n" +
    "There are exactly two ways to end the turn, and they are that same command with one flag " +
    "added or left off:\n" +
    "- no flag — you are finished.\n" +
    "- `--escalate` — you cannot reach the result and need help or a decision.\n\n" +
    "A ONE-LINE answer may be an argument instead: " +
    `\`nxc reply --thread ${spec.replyThread} --escalate "no test workspace here"\`.` +
    "\n\nDo that and nothing else."
  );
}

/** **Resume this session IN THIS PROCESS and remind it** (nxf 6j6v.gh7f).
 *
 *  IN THIS PROCESS is the load-bearing half: nxf 6j6v.7qtf's rule is one internal session, one
 *  process, and this session's own pid file names the node process running this teardown. A resume
 *  spawned from here would be refused by that guard — correctly — so the continuation is another
 *  `query()` on the same SDK session id instead, which starts nothing and cannot violate the rule
 *  it would otherwise have to work around.
 *
 *  The reminder's own messages go through the SAME normalizer and flush as the main turn: a
 *  continuation of a session is part of that session's transcript, and a reader looking for "why did
 *  this end without an answer" needs to see the turn that was asked for.
 *
 *  A failure here is a warning: it costs the reminder, not the run, and the fallback below is the
 *  behaviour that existed before this ticket. */
async function remindOnce() {
  // BOUNDED (review of PR #361, Integrity #1) — see `REPLY_REMINDER_BOUND_MS` for why an unbounded
  // one is not merely slow. The SDK's own `abortController` option is the supported door; the timer
  // is cleared on every exit path, including a throw, so this can never hold the process open past
  // the round it bounds.
  const abortController = new AbortController();
  const bound = setTimeout(() => abortController.abort(), REPLY_REMINDER_BOUND_MS);
  // A STOP ends this round too (nxf 6j6v.b9nf): the reminder is a `query()` of its own, with its
  // own controller, and a SIGTERM that only reached the main turn's would leave this one waiting out
  // its bound with the pid file still saying "running" — for up to five minutes past the withdrawal
  // that asked it to stop. Removed on every exit path, like the timer.
  const onStop = () => abortController.abort();
  stop.signal.addEventListener("abort", onStop, { once: true });
  try {
    for await (const msg of query({
      prompt: reminderMessage(),
      options: { ...options, resume: realSessionId, abortController },
    })) {
      if (msg.session_id) realSessionId = msg.session_id;
      if (process.env.NXC_SIDECAR_DEBUG) console.error(JSON.stringify(msg));
      if (!spec.session) continue;
      const entries = normalizer.push(msg);
      if (entries.length === 0) continue;
      transcript.push(...entries);
      if (entries.some((entry) => entry.kind === "result") || transcript.length >= FLUSH_THRESHOLD) {
        flushTranscript();
      }
    }
  } catch (err) {
    // An abort SURFACES AS A THROW from the SDK (`AbortError`), and the sentence it carries says
    // nothing about which of the two doors was used. Swallowed only when this round's controller
    // did fire, so that the check below can name the reason; any other throw is the stream's own
    // failure and propagates as before.
    if (!abortController.signal.aborted) throw err;
  } finally {
    clearTimeout(bound);
    stop.signal.removeEventListener("abort", onStop);
    // The transcript of the reminder is kept whichever way the round ended — an ABANDONED one is
    // precisely the round a reader looking for "why did this end without an answer" needs to see.
    transcript.push(...normalizer.flush());
    flushTranscript();
  }
  if (abortController.signal.aborted) {
    throw new Error(
      stopping()
        ? "the reminder round was abandoned because the session was told to stop"
        : `the reminder round was abandoned after ${REPLY_REMINDER_BOUND_MS} ms`,
    );
  }
}

/** **The teardown step that runs BEFORE the sidecar speaks for the agent** (nxf 6j6v.gh7f): a
 *  session that owes its thread an answer is given the turn back and told so, at most
 *  `TERMS.replyReminders` times, instead of having a failure posted in its name.
 *
 *  The substitute post is not merely incomplete, it is MISLEADING — it speaks as the agent and
 *  asserts an ending the agent never declared, so a caller reading it cannot tell whether the work
 *  ran, is running, or failed. Measured over four hours in the proving ground: 4 of 251 threads
 *  ended that way, and two of them stopped the chain until a human looked, once for over an hour,
 *  because the operation reads like a clean finish (`awaiting_human: true`, zero open threads).
 *
 *  **And skipped, since nxf 6j6v.hw2t, when the session is WAITING ON A ROUND IT COMMISSIONED
 *  ITSELF** — the third case, and the only one of the three that is not a failure of any kind. See
 *  the branch itself, in the loop below, for what it costs (nothing) and why the one attempt is not
 *  spent on it.
 *
 *  Skipped whole when there is no obligation, no SDK session to resume, or no readable board — and
 *  when THE RUNTIME FAILED BEFORE THE AGENT EVER RAN (nxf 6j6v.ntp9, answering nxf 6j6v.553s
 *  question (b)): a reminder tells a session that it broke the answering rule, and a session the
 *  runtime never ran broke no rule. Reminding there asserts an omission that did not happen and
 *  spends the one attempt this has on a resume into the same failure — which was reachable, because
 *  a RESUMED spec carries a session id to resume before the first frame ever arrives. That case is
 *  handled where it belongs, in the bounded retry around the stream; what follows either bound is
 *  the escalation `postFailureReply` posts.
 *
 *  **BOTH halves of that condition are load-bearing, and dropping either was a real defect**
 *  (independent review of PR #378, Code Quality #2 / Test Quality #5):
 *
 *  * `!agentEverThought` alone would excuse a stream that ended EMPTY AND CLEAN — no throw, no error
 *    result. Nothing says the runtime failed there, so the session is owed its reminder exactly as
 *    before nxf 6j6v.ntp9; skipping it would silently drop nxf 6j6v.gh7f for that shape, and leave
 *    the round with a substitute post carrying no `--escalate` and nobody asked to decide it.
 *  * `runtimeFailed` alone would excuse a MID-TURN tear, which is deliberately on the reminded side
 *    of this line and not the retried one: the session exists and has done work, so what it needs is
 *    to be asked for its result, which is exactly this. What it must NOT get is the turn replayed
 *    from the top. */
async function remindBeforeSpeakingForTheAgent() {
  if (!spec.replyThread || !realSessionId) return;
  // **A STOPPED SESSION IS NOT REMINDED** (nxf 6j6v.b9nf), and this is checked before every other
  // case because it is the one where the answer is not even about this session: the reminder
  // tells a session it broke the answering rule, and this one was told to stop by the withdrawal
  // that has already settled its thread. Resuming it would spend a model call to produce an answer
  // into a round the engine has taken back. Checked again inside the loop below, because a stop can
  // arrive between one reminder round and the next.
  if (stopping()) {
    remindOutcome = "stopped";
    console.error(
      `sidecar: not reminding session ${spec.session} about thread ${spec.replyThread} — it was ` +
        "told to stop, so it broke no answering rule and the withdrawal that stopped it has " +
        "settled the thread",
    );
    return;
  }
  // **THE FOURTH CASE** (nxf 6j6v.npy3), and it is checked before every other because it is the one
  // where asking anything at all costs something. A reminder tells a session it broke the answering
  // rule; this session broke no rule — the model stopped being available to it — and the reminder
  // is a `query()` on the same runtime, so it would spend the one attempt resuming straight back
  // into the window that is still closed. Not even the board is read: there is no reminder to
  // decide about, so the `nxc status` call that decides it is waste too.
  //
  // It sits ABOVE the never-ran branch rather than beside it because a boundary can be either — at
  // the door (nothing thought) or mid-turn (the measured case) — and the answer is the same both
  // times.
  if (boundary) {
    remindOutcome = "unavailable";
    console.error(
      `sidecar: not reminding session ${spec.session} about thread ${spec.replyThread} — it ran ` +
        `into an availability boundary (${boundary.limit}), so it broke no answering rule and a ` +
        "reminder would spend its one attempt resuming into the same closed window",
    );
    return;
  }
  if (runtimeFailed && !agentEverThought) {
    remindOutcome = "never_ran";
    // Said at the moment it happens, not only on the completion line: a run that ends this way is
    // usually a run that DIED, and a died run throws before it ever reaches that line.
    console.error(
      `sidecar: not reminding session ${spec.session} about thread ${spec.replyThread} — the ` +
        "runtime never ran it, so it broke no answering rule and a reminder would spend its one " +
        "attempt on a resume into the same failure",
    );
    return;
  }
  for (let attempt = 0; attempt < TERMS.replyReminders; attempt++) {
    if (stopping()) {
      remindOutcome = "stopped";
      return;
    }
    const debt = readReplyDebt();
    if (!debt) return;
    if (!debt.owes) {
      remindOutcome = remindOutcome ?? "not_owed";
      return;
    }
    // **THE THIRD CASE, and it is the reason this loop is not the whole story** (nxf 6j6v.hw2t).
    // `TurnTerms` names two ways a session can fail to answer — it ended without answering, or the
    // runtime never ran it — and both are treated above. This is the one that is not a failure at
    // all: the session commissioned a round of its own and ended its turn to wait for it, which is
    // exactly what the engine's obligation text now tells it to do. It gets NOTHING: no reminder
    // (there is no omission to remind about), no spent attempt (the one attempt belongs to the case
    // this is not), and no substitute post below. The coordinator wakes it when the sub-round comes
    // back, and the ceiling is that round's own deadline rather than a second clock here.
    //
    // Checked INSIDE the loop and before the first `remindOnce`, so the attempt is never spent —
    // and after the debt read, because the state is a fact about the board, not about this process.
    if (debt.waitingOn.length > 0) {
      waitingOnSubRound = debt.waitingOn;
      remindOutcome = "waiting_on_sub_round";
      console.error(
        `sidecar: not reminding session ${spec.session} about thread ${spec.replyThread} — it is ` +
          `waiting on a round it commissioned itself (${debt.waitingOn.join(", ")}), which is not ` +
          "a missing answer; it is woken when that round returns",
      );
      return;
    }
    try {
      await remindOnce();
    } catch (err) {
      process.stderr.write(`sidecar: the reminder resume failed: ${err?.stack ?? err}\n`);
      // A round abandoned BY A STOP is the stop, not a failure of the reminder (nxf 6j6v.b9nf).
      remindOutcome = stopping() ? "stopped" : "failed";
      return;
    }
    remindOutcome = "reminded";
  }
  const after = readReplyDebt();
  remindOutcome = after && !after.owes ? "answered" : "unanswered";
}

/** The THIRD and final teardown step (nxf 6j6v.7e9d, ticket 8): if THIS trigger's caller was told,
 *  in the prime block (ticket 6), that IT is owed a reply on `spec.replyThread` (ticket 5's
 *  `expects_reply_from`), and the session ends without ever posting one — crashed, the model forgot
 *  it, the SDK stream tore — the thread must not go quiet. `nxc reply --thread <id> --if-unanswered
 *  <text>` (ticket 7) is exactly the check that already decides whether this session is STILL on
 *  the hook: already posted, and the call is a deliberate, successful no-op; not posted, and this
 *  call is the ONLY thing that will ever say so.
 *
 *  UNCONDITIONAL: called every time `spec.replyThread` is set, whatever the run above did — the
 *  ENGINE's own `--if-unanswered` gate decides whether anything is actually written, never this
 *  code. `spec.replyThread` absent or `null` (no thread was ever told to expect an answer from THIS
 *  trigger — most callers, and the ticket 6 KNOWN LIMIT: a trigger into a thread that ALREADY
 *  carries somebody else's quorum registers nothing and tells the role nothing either)
 *  skips the call entirely — there is nothing to reply to, and no special-casing that limit here.
 *
 *  LAST, after `bindRealSession()` and the final `flushTranscript()` — see the "ORDER IS
 *  LOAD-BEARING" comment on the teardown block below, whose reasoning this extends by one step. A
 *  failure here is a WARNING, never `recordFatal`: of the three teardown steps it is the LEAST
 *  urgent — the bind is unrecoverable, the transcript is recoverable, and this is a notification
 *  ABOUT a session that has already ended one way or the other — so it ranks BELOW the transcript
 *  flush's own recoverable, non-fatal treatment (nxf 6j6v.jwgc) and must never be able to turn an
 *  otherwise successful run into a failed one. */
function postFailureReply() {
  if (!spec.replyThread) return;
  // **A STOPPED SESSION ASKS THE BOARD; IT DOES NOT ASSUME** (nxf 6j6v.b9nf, fix round 2 of that
  // item's review, Integrity #3).
  //
  // The first cut skipped this step for every SIGTERM on the ground that "the withdrawal that
  // stopped it has already settled the thread". `withdraw` does settle it — it discharges BEFORE it
  // signals — but a SIGTERM is not a withdrawal. It is also a system shutdown, a logout, a
  // `docker stop`, a container OOM, a person's `kill <pid>`. Before this item those killed the
  // process outright: no teardown, no `nxc session ended`, and the dead-holder sweep freed the
  // working copy in about a minute once somebody was queued. With a handler installed they became a
  // TIDY end — `holder_is_provably_dead` reads a session that reported its end as a clean shutdown
  // rather than a hard death — and with this step skipped as well, the thread was left owing an
  // answer that nothing would ever give, its caller unwoken and the fast path off until the
  // two-hour bound. The handler must not turn a hard death into a silent one.
  //
  // So the question is asked of the register, which is the only thing that knows: does this thread
  // still owe an answer? Discharged (a withdrawal) — nothing to say, and saying it would be the
  // misleading substitute post this step's own doc is about. Still owing (every other SIGTERM) —
  // exactly the case this step exists for, and the engine's `--if-unanswered` gate is still the one
  // that decides whether the write lands.
  //
  // An unreadable board falls THROUGH to the post, which is this file's usual fail-safe direction
  // inverted on purpose: `readReplyDebt` answers `null` for an older `nxc`, a locked database or
  // output that does not parse, and in that state the cheap mistake is a post the engine refuses
  // while the expensive one is a round left hanging.
  if (stopping()) {
    const debt = readReplyDebt();
    if (debt && !debt.owes) {
      console.error(
        `sidecar: posting nothing for thread ${spec.replyThread} — this session was told to stop ` +
          "and the thread is already discharged, so whoever stopped it has settled the round",
      );
      return;
    }
    console.error(
      `sidecar: session ${spec.session} was told to stop and thread ${spec.replyThread} still ` +
        "owes an answer — this SIGTERM settled nothing, so the substitute post goes out and the " +
        "engine's `--if-unanswered` gate decides whether it lands",
    );
  }
  // **AN INTERRUPTED SESSION IS OWED NOTHING AND SAYS NOTHING** (nxf 6j6v.npy3) — the same shape as
  // the waiting branch below, for a different reason, and it is the point of the whole item.
  //
  // What would go out here is `--escalate` with "this session ended without answering", because
  // `failures.hasFailed()` is true: the stream did end badly. Both halves of that are wrong. The
  // sentence asserts the agent cannot carry the task out, which is false — nothing is broken but
  // the availability of the model — and posting it DISCHARGES the thread, so the round reads
  // finished, `escalated: true` puts the operation on NEEDS DECISION, and the work stands there
  // until a human reads a false alarm.
  //
  // The thread therefore keeps OWING its answer, exactly as it does for a session waiting on its own
  // sub-round: the session that owes it is coming back to give it. What says so in the meantime is
  // the interruption record (`announceInterruption`), which is a field a reader can branch on rather
  // than a message body that has to be interpreted.
  //
  // Measured, and the margin is the reason this matters: in the case this item was cut from, the
  // window closed 0.6 s AFTER the session's `nxc reply` landed, so there was no debt left to settle
  // and nothing went out. One second earlier and the result of three full review rounds would have
  // been replaced by that sentence.
  if (boundary) {
    console.error(
      `sidecar: posting nothing for thread ${spec.replyThread} — this session ran into an ` +
        `availability boundary (${boundary.limit}), so the thread keeps its obligation and the ` +
        "round is taken up again rather than handed back",
    );
    return;
  }
  // **A WAITING session is owed nothing and says nothing** (nxf 6j6v.hw2t). This is the half of the
  // third case that the reminder branch alone could not deliver: a session that ends its turn to
  // wait for the round it commissioned must leave its thread OWING an answer, because it is coming
  // back to give one. Anything posted here would settle that debt — as a finished claim if it went
  // out plain, or as the false alarm this whole item exists to remove if it went out escalated.
  //
  // Not gated on `died`: a session that failed while a sub-round runs is still a failure somebody
  // has to hear about, and `remindBeforeSpeakingForTheAgent` only ever reaches the waiting branch on
  // a run that got as far as reading its own board. `runtimeFailed && !agentEverThought` is settled
  // one branch earlier still, and stays what it was — the runtime, not the agent's turn.
  //
  //  **WHAT THIS GIVES UP, stated rather than discovered** (review of PR #460, Integrity &
  //  Robustness #1). The post suppressed here was, by accident rather than by design, the only thing
  //  that ever surfaced a sub-round that will never come back. It said the wrong thing — an
  //  escalation in the caller's name, which is the false alarm nxf 6j6v.hw2t exists to remove — but
  //  it said SOMETHING, and after this it says nothing. Three cases, and only two are covered:
  //
  //    the sub-round answers                -> the caller is woken. The ordinary case.
  //    its session ends having announced it -> `nxc status` renders that thread
  //                                            "ITS SESSION HAS ENDED, nothing is coming", which is
  //                                            the finding, at the thread that actually hangs, in
  //                                            the right words. Better than what was here.
  //    its session is KILLED without         -> `session_state` reads `unknown`, the row says
  //    announcing, or hangs alive forever       nothing, and nobody is waiting on a clock. Silent.
  //
  //  The third is not covered, and it is not covered on purpose: the item's owner decided the
  //  ceiling comes from the commissioned round's own deadline and that no second clock may be built
  //  for the waiter ("Keine neue Uhr"). That reasoning assumed every commissioned round HAS a
  //  deadline, and a `send --to <persona>` round has none at all — which is exactly the shape the
  //  item was measured on. The gap is nxf 6j6v.t7y5; it is the owner's to decide, not this
  //  teardown's to paper over with a timer nobody asked for.
  if (waitingOnSubRound.length > 0 && !failures.hasFailed()) {
    console.error(
      `sidecar: posting nothing for thread ${spec.replyThread} — this session is waiting on the ` +
        `round it commissioned (${waitingOnSubRound.join(", ")}), so the thread keeps its ` +
        "obligation and no answer is owed yet",
    );
    return;
  }
  // CAPPED, like every other piece of SDK-derived text this sidecar persists (PR #336 review,
  // Integrity & Robustness #1): `failures.toError()` is an arbitrary thrown value's `toString()`
  // with no size of its own, and what is built from it here is not a log line but a durable message
  // body on a thread board other sessions fold into their prompts — the same class
  // `transcript.mjs`'s `MAX_THINKING_CHARS`/`MAX_TOOL_RESULT_CHARS` already bound. The cap is
  // applied HERE, to the finished text, so nothing unbounded ever reaches `execFileSync`.
  //  DIED vs ENDED QUIETLY, in the DATA and not only in the prose (nxf 6j6v.mqad). This function has
  //  always known the difference — `failures.hasFailed()` picks between two texts — and until now it
  //  translated it into nothing a caller could branch on: the reply went out as an ORDINARY one, so
  //  a session that died at an infrastructure error arrived at the requester as `state: "answered"`,
  //  `awaiting_human: true`, `warnings: []`, and the only thing telling a crash from a delivered
  //  answer was a message body that happened to start with `sidecar:`. That is prose where the guide
  //  itself says an app should be branching on a field.
  //
  //  `--escalate` is the carrier that already exists for exactly this meaning ("I cannot carry this
  //  out"), and it brings the two rules that matter with it, both of them already written and tested:
  //  an escalating set is NEVER folded by a `summarize` consolidator (`nxc guide channels`: running a
  //  model over it "would produce prose about a failure in the place a requester reads an answer"),
  //  and the escalation marker survives into `messages.kind`, where `ChatStore::last_reply_escalated`
  //  reads it — which is what puts `escalated: true` on the requester's own `nxc status`.
  //
  //  A session that ended cleanly WITHOUT answering is deliberately NOT an escalation: nothing went
  //  wrong, the turn simply produced no reply, and marking it would tell a supervisor to stop.
  const died = failures.hasFailed();
  // **A REMINDED session that stayed silent is an escalation** (nxf 6j6v.gh7f, DoD point 3). The
  // rule above stands for a session that simply ended quietly — nothing went wrong, and marking it
  // would tell a supervisor to stop. It does not stand once the session has been handed its turn
  // back and told, in words that admit no reading, that it owes an answer: at that point there is no
  // result and somebody has to decide what happens to the round, which is exactly what the
  // escalation bit means and exactly what `escalated: true` on the caller's own `nxc status` is for.
  // A prose-only difference would be the very shape nxf 6j6v.mqad removed from this function.
  const silentAfterReminder = remindOutcome === "unanswered";
  // **WHICH bound was reached, in the text a human reads** (nxf 6j6v.ntp9). "Ended without
  // answering" is true of a `529` at the door and of a turn that ran and crashed, and the two call
  // for opposite things from whoever reads it: one is an outage to wait out, the other is work that
  // may be half done in the checkout. Naming the attempts is what tells them apart — and it is the
  // reason the retry is worth reporting at all, since a retry that eventually succeeded says nothing
  // here.
  const neverRan = runtimeFailed && !agentEverThought;
  const attempts = retriedRuntime + 1;
  const text = capFailureReply(
    neverRan
      ? `sidecar: the runtime never ran this session — ${attempts} attempt${attempts === 1 ? "" : "s"}, ` +
        `last error: ${failures.toError()}`
      : died
        ? `sidecar: this session ended without answering — ${failures.toError()}`
        : silentAfterReminder
          ? `sidecar: this session was reminded that thread ${spec.replyThread} is waiting on its answer and ended again without one`
          : "sidecar: this session ended without ever posting a reply",
  );
  const args = ["reply", "--thread", spec.replyThread, "--if-unanswered"];
  if (died || silentAfterReminder) args.push("--escalate");
  args.push(text);
  try {
    execFileSync("nxc", args, {
      env: { ...process.env, ...spec.env },
      timeout: NXC_CALL_TIMEOUT_MS,
      // No stdin content to send (unlike the transcript flush) and nothing on stdout worth keeping
      // live; stderr is piped so its text can be inspected below, parity with the other two calls.
      stdio: ["ignore", "inherit", "pipe"],
    });
  } catch (err) {
    const stderr = err.stderr ? err.stderr.toString() : "";
    if (stderr) process.stderr.write(stderr);
    // The FLAG-shaped sibling of `isSubcommandNotImplementedYet`: `--if-unanswered` is a flag on an
    // EXISTING subcommand (`reply`), so an `nxc` older than ticket 7 rejects it as an "unexpected
    // argument", not an "unrecognized subcommand". A teardown running against such a binary must
    // not lose the bind or the transcript over this — and by now it already has not, since this
    // step runs last.
    if (isFlagNotImplementedYet(err.status, stderr, "--if-unanswered")) {
      console.error(
        `sidecar: nxc reply --if-unanswered skipped (flag not implemented yet): ${err.message}`,
      );
      return;
    }
    console.error(`sidecar: nxc reply --if-unanswered failed: ${err.message}`);
  }
}

/** **Tell the engine this session is ON HOLD, not finished** (nxf 6j6v.npy3) — the way back, and
 *  the one step that puts an availability boundary anywhere a reader can see it.
 *
 *  Until this, the fact existed EXCLUSIVELY in the transcript: `nxc session state` reported a clean
 *  `ended`, the thread read `answered`, and nothing was `escalated` or `substituted`. Measured, not
 *  inferred — the whole channel record of the run this item comes from was searched, and a weekly
 *  window that demonstrably closed mid-run left no trace in it at all. The system could not SAY it
 *  had been interrupted, which is a shorter sentence than "cannot resume" and a worse one.
 *
 *  `--until` is passed only when the runtime NAMED an instant. Absent, the hold is still recorded
 *  and still readable; what it does not get is an automatic return, which is the honest consequence
 *  of nobody having said when the window lifts.
 *
 *  BEFORE `announceSessionEnd`, and the order is load-bearing in one direction: the end announcement
 *  is what lets a supervisor act on a session that has stopped, and it must not decide anything
 *  about this session before the reason it stopped is on the record.
 *
 *  A failure here is a WARNING for `postFailureReply`'s reason, with one addition of its own: what
 *  it costs is the AUTOMATIC way back, never the work. The session's transcript, its runtime id and
 *  the anchor of its last handover are all already durable, so `nxc resume --thread <id>` still
 *  takes the operation up by hand. An `nxc` older than this item does not have the subcommand at
 *  all, which is the forward-compatibility shape `bindRealSession` and `announceSessionEnd` both
 *  already handle. */
function announceInterruption() {
  if (!spec.session || !boundary) return;
  // Said HERE and not on the `sidecar done:` line, because a run that hit a boundary can never
  // reach that line: a boundary implies `failures.hasFailed()` (see where it is resolved), and that
  // throws. This is the one line that always prints, and it is what a reader greps for.
  console.error(
    `sidecar: session ${spec.session} is ON HOLD at an availability boundary (${boundary.limit})` +
      `${boundary.until ? `, until ${boundary.until}` : ", with no reset instant stated"}: ` +
      `${boundary.detail}`,
  );
  const args = ["session", "interrupted", spec.session, "--limit", boundary.limit];
  if (boundary.until) args.push("--until", boundary.until);
  args.push("--detail", boundary.detail);
  try {
    execFileSync("nxc", args, {
      env: { ...process.env, ...spec.env },
      timeout: NXC_CALL_TIMEOUT_MS,
      stdio: ["ignore", "inherit", "pipe"],
    });
  } catch (err) {
    const stderr = err.stderr ? err.stderr.toString() : "";
    if (stderr) process.stderr.write(stderr);
    // Matched on `"interrupted"`, not on the group: an older `nxc` HAS `session` (it has carried
    // `bind` since Task 2) and rejects only the new leaf, so clap's wording names this word.
    if (isSubcommandNotImplementedYet(err.status, stderr, "interrupted")) {
      console.error(
        `sidecar: nxc session interrupted skipped (subcommand not implemented yet): ${err.message}`,
      );
      return;
    }
    console.error(`sidecar: nxc session interrupted failed: ${err.message}`);
  }
}

/** The FOURTH and last teardown step (nxf 6j6v.10yb): tell the engine that THIS session's process
 *  is over.
 *
 *  Why it cannot be observed from the outside: a channel that declares `working_tree: exclusive`
 *  opens its next step once the previous one has answered AND its session is over — and the only
 *  moment anything knows a session is ending is right here, inside the process that is ending. A
 *  liveness check made from anywhere else at this instant would say "alive", because it is. The
 *  measured cost of ending a step on the message alone is in that ticket: a coder answered at
 *  00:16:33 and went on writing until 00:34:27 while the next step ran beside it in the same
 *  checkout.
 *
 *  LAST, after `postFailureReply()`, and the order is load-bearing in one direction: the reply
 *  settles the debt, and the announcement is what lets the supervisor act on a set that is now
 *  genuinely settled. Reversed, the announcement would find the thread still owing an answer and
 *  decide nothing, and the reply that followed would then be back to deciding on a message alone.
 *
 *  A failure here is a WARNING for `postFailureReply`'s reason and one more: what it costs is a
 *  DELAY — the flow falls back to the process-liveness backstop and, past that, to the channel's own
 *  declared `timeout:` — never a wrong answer. It must not be able to fail an otherwise good run.
 *  An `nxc` older than this ticket does not have the subcommand at all, which is the same
 *  forward-compatibility shape `bindRealSession` handles for `session bind`. */
function announceSessionEnd() {
  if (!spec.session) return;
  try {
    execFileSync("nxc", ["session", "ended", spec.session], {
      env: { ...process.env, ...spec.env },
      timeout: NXC_CALL_TIMEOUT_MS,
      stdio: ["ignore", "inherit", "pipe"],
    });
  } catch (err) {
    const stderr = err.stderr ? err.stderr.toString() : "";
    if (stderr) process.stderr.write(stderr);
    // Matched on `"ended"`, NOT on the default `"session"`: an `nxc` older than this ticket HAS the
    // `session` group (it has carried `bind` since Task 2) and rejects only the new leaf, so clap's
    // wording names `ended`. Checking the group would never match and a merely-old binary would be
    // reported as a real failure.
    if (isSubcommandNotImplementedYet(err.status, stderr, "ended")) {
      console.error(`sidecar: nxc session ended skipped (subcommand not implemented yet): ${err.message}`);
      return;
    }
    console.error(`sidecar: nxc session ended failed: ${err.message}`);
  }
}

// The stream error is CAUGHT rather than left to unwind, so teardown below runs in a fixed order
// with each step independent of the others' failure. A rejected `query()` throws out of `for await`
// and must not skip the end-of-stream flush: that would drop up to FLUSH_THRESHOLD entries plus the
// open block on exactly the run whose transcript is worth the most (the one that failed).
//
// Nothing the loop BODY does can land here any more: the mid-run `flushTranscript()` used to throw
// from inside this try, and unwinding out of `for await` abandons the SDK generator — which ends
// the turn mid-answer, so the caller got nothing and sat out the workflow timeout while the session
// it was waiting on had already been torn down. That was 6j6v.jwgc, and the latch above is the fix:
// the flush now handles its own failure and returns, so this catch means the stream, and only it.
//
// ── AND IT IS A BOUNDED LOOP (nxf 6j6v.ntp9, answering nxf 6j6v.553s question (b)) ──────────────
//
// A failed attempt is one of two things, and until this item they were treated as one:
//
//   the runtime produced no model output  -> the agent never got to think; start the turn over
//   the stream ended after it had         -> the turn did work; NEVER start it over
//
// Only the first is retried, and the narrowness is the whole safety argument: an agent turn is not
// idempotent, so replaying one that already ran would redo its files, its messages and its pull
// requests. `agentEverThought` is what separates them, and it is cumulative, so a second attempt
// that gets further can never be replayed a third time. How many attempts and how long between them
// are the COORDINATOR's, not this file's — see `TERMS`.
//
// **AN ATTEMPT CAN FAIL WITHOUT THROWING**, which is why this loop reads a `failure` variable rather
// than relying on `catch` (independent review of PR #378, Integrity #1). The SDK handles a retryable
// API error itself and reports it as `system/api_retry`; when it gives up it ends the stream
// NORMALLY with a `result` whose `is_error` is true. So the one failure this whole feature exists
// for — a `529` — arrives as an ordinary end of iteration, and a `catch`-only loop never saw it.
for (let attempt = 0; ; attempt++) {
  // **THE DOOR IS READ BEFORE A ROUND IS STARTED, not only after one ended** (nxf 6j6v.b9nf, fix
  // round 2 of that item's review). The abort controller reaches a `query()` that is ALREADY
  // running; the window between two attempts — `sleep(wait)`, a bare timer — is listening to
  // nothing. So a SIGTERM that lands in the backoff was first seen at the check below, by which
  // time `query()` had been entered again with an already-aborted controller: a round started for
  // a turn somebody had just ended, which is the one thing a stop may not do. Reading the door here
  // costs one boolean per attempt and makes that unreachable, including for a signal that arrives
  // before the first attempt.
  if (stopping()) break;
  /** What ended this attempt badly, if anything: a thrown error, or the failure a terminal `result`
   *  states. `null` means the attempt ended cleanly and the loop is done. */
  let failure = null;
  // **PER-ATTEMPT, and that is a correctness rule rather than tidiness** (independent review of PR
  // #476, Code Quality #3 / Integrity #3). These three say what ended THIS attempt. A retry only
  // ever happens when the model produced nothing (`agentEverThought` is false), but an attempt can
  // still have seen a rate-limit signal at the door and been retried — and if the NEXT attempt then
  // dies of something else entirely, a signal left over from the abandoned one would classify a
  // real breakage as a harmless "on hold". That suppresses the escalation a broken run has to get,
  // and it arms a return for a boundary that is not the reason this run ended.
  //
  // `agentEverThought` is deliberately NOT reset: it is cumulative on purpose, because once any
  // attempt has produced model output the turn has done work and may never be replayed.
  lastAssistantError = null;
  lastResultText = null;
  normalizer.forgetRateLimit();
  try {
    // The stop door is handed to the MAIN turn as its `abortController` (nxf 6j6v.b9nf). It is not
    // a bound — nothing in this file fires it on a clock, and the turn's length stays the caller's
    // business (cutting it short was the defect of nxf 6j6v.jwgc); it fires on SIGTERM and on
    // nothing else. The reminder round has a controller of its own, with a timer, and listens to
    // this one — see `remindOnce`.
    for await (const msg of query({ prompt: spec.message, options: { ...options, abortController: stop } })) {
      // Confirmed on @anthropic-ai/claude-agent-sdk 0.3.215: every SDKMessage variant (system/init,
      // assistant, result, ...) carries a top-level `session_id` string — see sdk.d.ts. The *last*
      // message's session_id wins, which in practice is the same real UUID throughout one query().
      if (msg.session_id) realSessionId = msg.session_id;
      // The MODEL produced something — see `isAgentOutput` for why this is not "a message arrived",
      // and why `session_id` was never the test (a resumed spec has one before the first frame).
      if (isAgentOutput(msg)) agentEverThought = true;
      // **The two structured signals an availability boundary leaves** (nxf 6j6v.npy3). Captured
      // here rather than read back off the transcript, because the teardown branches on them and a
      // decision must rest on a fact about the run. The third — the rejected `rate_limit_event` —
      // is latched inside the normalizer, which is the one place that already knows SDK shapes.
      if (msg.type === "assistant" && msg.error) lastAssistantError = msg.error;
      // A terminal result that declares itself an error. NOT `break`ed on: the stream is allowed to
      // finish so the transcript keeps every entry of the attempt that failed, which is the one
      // whose transcript is worth the most.
      if (msg.type === "result" && msg.is_error === true) {
        failure = resultFailure(msg);
        // Where `SDKResultError` actually carries the runtime's own sentences — it has no `result`
        // field at all (`sdk.d.ts`). For the DETAIL a human reads, never for a decision.
        if (Array.isArray(msg.errors) && msg.errors.length > 0) {
          lastResultText = msg.errors.join("; ");
        }
      }
      if (process.env.NXC_SIDECAR_DEBUG) console.error(JSON.stringify(msg));
      // No internal session id to key the rows by (a hand-written bare spec — every real run gets
      // one from `SidecarWorker::trigger`), so there is nowhere to append: don't capture at all,
      // rather than buffering entries no flush will ever drain.
      if (!spec.session) continue;
      const entries = normalizer.push(msg);
      if (entries.length === 0) continue;
      transcript.push(...entries);
      // A `result` entry is a turn boundary: flush there so a complete turn is durable promptly, and
      // otherwise only when the buffer has grown past the threshold.
      if (entries.some((entry) => entry.kind === "result") || transcript.length >= FLUSH_THRESHOLD) {
        flushTranscript();
      }
    }
  } catch (err) {
    failure = err;
  }
  // **A STOP ENDS THE LOOP, whatever the attempt looked like** (nxf 6j6v.b9nf). The abort surfaces
  // as a thrown `AbortError` — the same shape a torn stream has — and it must be neither RETRIED
  // (the agent may have produced nothing yet, which is exactly the retry's condition; restarting
  // the turn somebody just ended is the one thing a stop may not do) nor RECORDED as a failure of
  // the run (`recordFatal` would put the turn on the escalating path, and a stopped turn owes
  // nobody an escalation). Checked before the clean-exit break too: a stream that happened to end
  // cleanly at the instant of the stop is still a stopped run, and the teardown reads that.
  //
  // This one reads a stop that landed during an ATTEMPT. The same door is read at the TOP of the
  // loop, for the window this one cannot see: the backoff between two attempts.
  if (stopping()) break;
  // The attempt ended cleanly — including the ordinary case of a stream that simply stops without a
  // terminal `result` at all, which is what an open trailing block looks like and is not a failure.
  if (!failure) break;
  // The doubling backoff the terms name, and the two conditions that have to hold together: the
  // agent has still produced nothing, and there are attempts left.
  //
  // **Two residuals, named rather than left to be discovered** (independent review of PR #378,
  // Integrity #5 and #6):
  //
  //  * A retried RESUME re-sends `spec.message` with `options.resume` still set, so it re-enters a
  //    session it may already have written to. It is safe on exactly the ground the guard states —
  //    the agent produced nothing, so nothing was appended on its behalf — and it stops being safe
  //    the day the CLI writes the user turn to the session before emitting its first frame. That is
  //    an SDK property, not one this file can enforce, so it is stated here to be re-checked at the
  //    next bump rather than assumed silently.
  //  * A DETERMINISTIC startup failure — a missing `cwd`, an unresolvable `claudePath`, an
  //    unreadable spec — also produces no message, so it is retried too and fails identically each
  //    time. Accepted rather than filtered: the alternative is matching on an error surface the SDK
  //    does not enumerate, which fails the wrong way (a mis-classified transient error is a chain
  //    that stops). The cost is bounded by the same two ceilings as any other retry, and such a run
  //    is loud in the log at every attempt.
  if (!agentEverThought && attempt < TERMS.runtimeRetries) {
    const wait = retryDelay(TERMS, attempt);
    console.error(
      `sidecar: the runtime never ran this session (${failure?.message ?? failure}) — retrying in ` +
        `${wait}ms (attempt ${attempt + 2} of ${TERMS.runtimeRetries + 1})`,
    );
    retriedRuntime = attempt + 1;
    await sleep(wait);
    continue;
  }
  // Out of attempts, or the turn had already begun. Either way this run is over, and the teardown
  // below escalates rather than reminding — see `runtimeFailed`.
  runtimeFailed = true;
  recordFatal(failure, "the SDK stream");
  break;
}

// **Was this an availability boundary?** (nxf 6j6v.npy3) — asked ONCE, here, between the loop and
// the teardown, because three teardown steps branch on it and they must not each re-derive it.
//
// After the retry loop rather than inside it: a boundary is a property of how the run ENDED, and an
// attempt that hit one and then succeeded on a retry hit nothing worth recording. (It cannot in
// fact retry — a boundary that arrives at the door leaves `agentEverThought` false and the retry
// bound is two attempts of 5s and 10s, which no reset window is ever inside — but the ordering is
// what makes that true by construction rather than by arithmetic.)
// **Only for a run that ENDED at one**, and the guard is not a formality: the SDK retries a
// retryable API failure internally, so a stream can carry an `assistant.error: rate_limit` and then
// go on to finish the turn perfectly well. Without `hasFailed()` this would put a session that
// ANSWERED on hold — and arm a resume for work that is already done.
boundary = failures.hasFailed()
  ? availabilityBoundary({
      assistantError: lastAssistantError,
      rateLimit: normalizer.rateLimit(),
      resultText: lastResultText,
    })
  : null;

// ── Teardown. The steps run in this order, whatever the stream or the earlier steps did. ──────────
//
// All of them, with ONE stated exception (nxf 6j6v.b9nf): a session that was TOLD TO STOP skips the
// REMINDER, which is the step that would spend a model call asking it for an answer it was just
// told not to give. The substitute post is not skipped on the signal — it asks the board whether
// the thread still owes one, because a SIGTERM from outside a withdrawal settles nothing (fix
// round 2 of this item's review, Integrity #3).

// ORDER IS LOAD-BEARING: bind BEFORE the final flush. The bind does not depend on the transcript at
// all, while a non-tolerated append failure (SQLITE_BUSY from the role's own in-session `nxc`
// holding the write lock, a validation rejection, a migration error) correctly propagates — with
// the flush first, that would leave `session_map` without a row and the next `resume` would
// silently start a fresh conversation instead of continuing this one. Losing transcript entries is
// recoverable; losing the internal↔real mapping is not. What remains open is a HARD kill mid-loop
// (the process dying without unwinding at all), not a failure of either step: a mid-loop flush
// throw now lands in the catch above and still reaches the bind.
//
// Running here also means the bind is attempted when the STREAM throws, which it was not before —
// deliberate, because the SDK session exists from the moment we saw its id and a later `resume`
// must be able to find it. That IS a trade, not a free win: if the stream died mid-turn, the bound
// session can hold a dangling `tool_use` with no `tool_result`, and a `resume` now reattaches to
// that state where the old behavior (no bind) would have started clean. Preferred anyway — a
// recoverable dangling turn beats silently abandoning the whole conversation.
try {
  bindRealSession();
} catch (err) {
  recordFatal(err, "nxc session bind");
}

// Outside the bind's try, so a throwing bind cannot skip it (that would drop the very entries the
// catch above exists to save). `normalizer.flush()` FIRST: the stream can end with a coalescing
// block still open (a final assistant answer with no trailing `result`), and it would otherwise be
// lost. Only the NORMALIZER can fail here — `flushTranscript` handles its own failures via the
// latch and does not throw — and that stays fatal on purpose: an exception out of pure in-process
// code is a defect in `transcript.mjs`, not the environment being briefly unavailable, and it says
// the entries themselves are wrong rather than merely unwritten.
try {
  transcript.push(...normalizer.flush());
} catch (err) {
  recordFatal(err, "the transcript normalizer's final flush");
}
flushTranscript();

// THE REMINDER, and it comes BEFORE the sidecar is allowed to speak for the agent (nxf 6j6v.gh7f).
// After the bind and the final flush, for the same ranking those two are ordered by: it spends a
// model call, so nothing unrecoverable may wait behind it. `await` at the top level of an ES module
// is what the whole file already runs under (`for await` above).
await remindBeforeSpeakingForTheAgent();

// THE THIRD STEP, and why it is safe to run even when the run above is about to be reported as
// failed: it reads `failures.hasFailed()` itself (to compose its text) but never WRITES to
// `failures` — see `postFailureReply`'s own doc for why its own failure is a warning, not a
// `recordFatal`. Placed here, before the throw below, so it runs on the FAILING path too: a stream
// that died mid-answer is exactly the case a caller most needs told, not one to skip past on the
// way to rethrowing.
postFailureReply();

// THE HOLD, before the end is announced (nxf 6j6v.npy3): a session that stopped because the model
// went away has to have its reason on the record before anything acts on the fact that it stopped.
// A no-op for every run that did not hit a boundary, which is every ordinary run.
announceInterruption();

// THE FOURTH STEP, and it runs on the failing path too for the third step's reason — a session that
// died mid-answer is exactly the one whose step must not be left holding a flow open.
announceSessionEnd();

// `hasFailed()`, never the truthiness of the error: a falsy rejection is still a failed run, and
// must never reach the `sidecar done:` line that tells SidecarWorker this turn succeeded. A STOPPED
// run is neither (nxf 6j6v.b9nf): it reports itself on its own line below, whatever else happened,
// because the stop is the fact a reader of this log needs first — the failures, if any, were
// written the moment they happened.
if (failures.hasFailed() && !stopping()) throw failures.toError();
// The marker is written ONLY when the latch went down, so it stays an exception a reader notices
// rather than a field on every line that they learn to skip past. A run that says nothing about
// its transcript captured everything it saw.
const transcriptState = transcriptLatched ? " transcript=incomplete" : "";
// `reminded=<outcome>` is written ONLY when a reminder round actually ran and decided something
// (nxf 6j6v.gh7f) — the same discipline `transcript=incomplete` keeps: a token on every line is a
// token readers learn to skip. `answered` means the session was handed its turn back and used it;
// `unanswered` means it did not, and the substitute post above went out as an escalation.
// `waiting_on_sub_round` (nxf 6j6v.hw2t) means no reminder ran and nothing was posted, on purpose:
// the session is waiting on work of its own and keeps its obligation until it comes back. It is
// written for the same reason the others are — a teardown that decided to stay silent has to say
// so, or the silence is indistinguishable from the bug this branch exists to remove.
// `stopped` (nxf 6j6v.b9nf) means the reminder was skipped, or abandoned mid-round, because the
// session was told to stop — written for the reason `waiting_on_sub_round` is.
const remindState =
  remindOutcome === "answered" ||
  remindOutcome === "unanswered" ||
  remindOutcome === "failed" ||
  remindOutcome === "never_ran" ||
  remindOutcome === "unavailable" ||
  remindOutcome === "waiting_on_sub_round" ||
  remindOutcome === "stopped"
    ? ` reminded=${remindOutcome}`
    : "";
// `retried=<n>` on the same discipline: written only when the runtime actually had to be asked
// again (nxf 6j6v.ntp9). A run that reached the model first time says nothing about it.
const retryState = retriedRuntime > 0 ? ` retried=${retriedRuntime}` : "";
if (stopping()) {
  // `sidecar stopped:`, never `sidecar done:` — a stopped turn is not a finished one, and a reader
  // grepping for the completion line must not find it. The exit code says the same thing to a
  // process: `STOPPED_EXIT_CODE`, set rather than `process.exit()`ed so that stderr drains the way
  // it does on every other ending of this file.
  console.error(
    `sidecar stopped: role=${spec.role} session=${spec.session} real=${realSessionId}${transcriptState}${remindState}${retryState}`,
  );
  process.exitCode = STOPPED_EXIT_CODE;
} else {
  console.error(
    `sidecar done: role=${spec.role} session=${spec.session} real=${realSessionId}${transcriptState}${remindState}${retryState}`,
  );
}
