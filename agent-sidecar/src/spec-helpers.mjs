// Pure, side-effect-free decision logic extracted out of main.mjs so it can be unit-tested
// without going through the SDK query()/execFileSync() machinery. No I/O, no SDK calls here —
// keep it that way; anything that touches the filesystem, the network, or a child process
// belongs in main.mjs, not here.

/**
 * The engine's grant as a plain array — `spec.grantedTools` if it is one, otherwise empty.
 *
 * The grant is what the COORDINATOR added because it demands something of this session (nxf
 * 6j6v.kffm: a trigger that registers an expectation tells the session to end its turn with
 * `nxc reply`, so it has to be able to run one). It is deliberately NOT merged into `spec.tools` on
 * the Rust side: `tools` is the role author's declaration, and the two feed the SDK's two different
 * lists below.
 *
 * An older engine sends no such key, which reads as no grant — the same forward-compat shape every
 * other optional spec key has.
 *
 * @param {unknown} grantedTools - `spec.grantedTools` as parsed from the role spec.
 * @returns {string[]} the granted tools, never undefined.
 */
function grantedOf(grantedTools) {
  return Array.isArray(grantedTools) ? grantedTools.filter((t) => typeof t === "string") : [];
}

/**
 * Decide what main.mjs should assign to the SDK's `options.tools` (the base toolset), given
 * `spec.tools` and `spec.grantedTools` as parsed from the role spec JSON.
 *
 * Mirrors the exact guard main.mjs used inline: `if (Array.isArray(spec.tools)) options.tools =
 * spec.tools;`. Only an actual array — including `[]` — is a genuine declaration of intent:
 *   - `tools` absent (`undefined`) or `null` -> returns `undefined`, meaning "leave
 *     `options.tools` unset", so the SDK's default full toolset applies. The grant is NOT folded
 *     in here, and that is the point: the default set already contains everything the grant names,
 *     and turning "unset" into a list would SHRINK an undeclared role's toolset to exactly the
 *     grant — which is the regression this whole split exists to avoid.
 *   - `tools: []` -> the grant, and nothing else. An explicit zero-tools role that is put under an
 *     obligation gets exactly the means for that obligation and no more — the same narrow scoping
 *     nxf 6j6v.04es gave the `summarize` synthesizer by hand, now derived.
 *   - `tools: [...]` -> that array plus anything granted that is not already in it, order
 *     preserved (declaration first).
 *
 * @param {unknown} tools - `spec.tools` as parsed from the role spec (may be undefined, null,
 *   an array, or any other JSON value).
 * @param {unknown} [grantedTools] - `spec.grantedTools` as parsed from the role spec.
 * @returns {string[] | undefined} the value for `options.tools`, or `undefined` to leave it unset.
 */
export function resolveToolsOption(tools, grantedTools) {
  if (!Array.isArray(tools)) return undefined;
  const granted = grantedOf(grantedTools);
  return [...tools, ...granted.filter((t) => !tools.includes(t))];
}

/**
 * Decide what main.mjs should assign to the SDK's `allowedTools` (the AUTO-APPROVAL list within
 * whatever base toolset is active — it never widens that set, only pre-approves inside it).
 *
 * This is the half nxf 6j6v.kffm was actually about. It used to be `spec.tools ?? []`, so a role
 * that declared no `tools:` — the default — got the SDK's full base toolset and an EMPTY approval
 * list: every tool call it made, its own obligatory `nxc reply` included, came back "This command
 * requires approval". Union of the declaration and the grant, so the obligation is runnable
 * whatever the declaration says, and a role that was granted nothing reads exactly as before.
 *
 * @param {unknown} tools - `spec.tools` as parsed from the role spec.
 * @param {unknown} [grantedTools] - `spec.grantedTools` as parsed from the role spec.
 * @returns {string[]} the value for `allowedTools`.
 */
export function resolveAllowedTools(tools, grantedTools) {
  const declared = Array.isArray(tools) ? tools.filter((t) => typeof t === "string") : [];
  const granted = grantedOf(grantedTools);
  return [...declared, ...granted.filter((t) => !declared.includes(t))];
}

/**
 * Decide what main.mjs should assign to the SDK's `options.model`, given `spec.model` as parsed
 * from the role spec JSON.
 *
 * The alias the role author declares (`fable`/`opus`/`sonnet`) is resolved to an SDK model id on
 * the Rust side (`role::Model::sdk_id`) — one mapping table, in the engine — so by the time it
 * reaches here it is already a finished model string and this function only decides whether to set
 * the option at all:
 *   - `model` absent (`undefined`) or `null` (what the spec actually carries for a role that
 *     declared none) -> returns `undefined`, meaning "leave `options.model` unset", so the SDK's
 *     own default model applies.
 *   - a non-empty string -> returned verbatim.
 *   - anything else (empty string, number, object — only reachable from a malformed spec) ->
 *     `undefined`, so a broken spec degrades to the default model rather than failing the session
 *     with an opaque model error.
 *
 * @param {unknown} model - `spec.model` as parsed from the role spec.
 * @returns {string | undefined} the value for `options.model`, or `undefined` to leave it unset.
 */
export function resolveModelOption(model) {
  return typeof model === "string" && model.length > 0 ? model : undefined;
}

/**
 * The exact `execFileSync` catch-narrowing check from main.mjs's callbacks into `nxc`: true only
 * for the specific "subcommand doesn't exist yet" shape (clap's exit code 2 with an "unrecognized
 * subcommand '<name>'" stderr, either quote style). Any other status/stderr combination must
 * propagate as a genuine fatal error rather than being silently swallowed.
 *
 * `subcommand` is a parameter (defaulting to the original `session` caller) because the transcript
 * flush (0dbp) tolerates the same pre-Task-2 gap for `nxc transcript append`. It is matched by name
 * on purpose: tolerating ANY unrecognized subcommand would silently swallow a typo'd or renamed
 * call site.
 *
 * @param {number | undefined} status - `err.status` from the caught `execFileSync` error.
 * @param {string} stderr - captured stderr text (empty string if none was captured).
 * @param {string} [subcommand] - the subcommand that was invoked (a literal from the call site).
 * @returns {boolean} true iff this is the "subcommand not implemented yet" shape for that name.
 */
export function isSubcommandNotImplementedYet(status, stderr, subcommand = "session") {
  return status === 2 && new RegExp(`unrecognized subcommand ['"]${subcommand}['"]`).test(stderr);
}

/**
 * The sibling check for a FLAG an older `nxc` does not know yet, as opposed to a whole SUBCOMMAND
 * ({@link isSubcommandNotImplementedYet}): clap's exit code 2 with an "unexpected argument '<flag>'"
 * stderr (either quote style) — its wording for an option no subcommand in that `nxc` declares.
 *
 * Exists for the teardown's `nxc reply --if-unanswered` (nxf 6j6v.7e9d, ticket 8): `--if-unanswered`
 * is a FLAG on an EXISTING subcommand (`reply`), so an `nxc` built before ticket 7 rejects it with
 * this shape, not the "unrecognized subcommand" one. Matched by NAME, same discipline as the
 * subcommand check: tolerating ANY unexpected argument would silently swallow a typo'd or renamed
 * call site as if it were merely an old binary.
 *
 * @param {number | undefined} status - `err.status` from the caught `execFileSync` error.
 * @param {string} stderr - captured stderr text (empty string if none was captured).
 * @param {string} flag - the flag that was passed (a literal from the call site), e.g.
 *   `"--if-unanswered"`.
 * @returns {boolean} true iff this is the "flag not implemented yet" shape for that flag.
 */
export function isFlagNotImplementedYet(status, stderr, flag) {
  return status === 2 && new RegExp(`unexpected argument ['"]${flag}['"]`).test(stderr);
}

/**
 * Track whether the run failed, and which error should escape once teardown has run.
 *
 * main.mjs cannot simply let an exception unwind: the stream error is caught so that all THREE
 * teardown steps (`nxc session bind`, then the final transcript flush, then the unanswered-thread
 * discharge added by nxf 6j6v.7e9d) still run, each independent of the others' failure. That
 * catch-and-rethrow reintroduced a hole a plain `finally` was structurally
 * immune to — `if (fatal) throw fatal` never fires when the stream rejects with a FALSY reason (a
 * bare `reject()` on an abort/cancel path, `throw null`), so the sidecar printed `sidecar done:`
 * and exited 0, reporting a stream that died mid-answer to `SidecarWorker` as a success.
 *
 * The fix is structural, not a guard at one call site: "did it fail" is a boolean of its own, kept
 * separately from the error VALUE, so no falsy payload can make a failure look like a success. This
 * lives here, in the pure-helpers module, precisely so the property is unit-tested rather than
 * resting on an uncommitted harness.
 *
 * The FIRST failure is the one that escapes — it is the run's root event. Later failures are not
 * lost: main.mjs writes every one to stderr the moment it happens, in causal order.
 *
 * @returns {{ record: (err: unknown) => void, hasFailed: () => boolean, toError: () => Error | null }}
 */
export function createFailureTracker() {
  let failed = false;
  let first;
  return {
    record(err) {
      if (failed) return; // keep the root event, not the last one
      failed = true;
      first = err;
    },
    hasFailed() {
      return failed;
    },
    toError() {
      if (!failed) return null;
      // Normalize a falsy rejection into a real, throwable, printable Error naming what arrived —
      // `throw null` would otherwise surface as an unreadable "Uncaught null".
      return first || new Error(`the run failed with a falsy reason (${String(first)})`);
    },
  };
}

/**
 * Truncate to `max`, appending a note naming how much was dropped.
 *
 * THE one overflow marker in this sidecar, shared by every cap, so a consumer reading a capped value
 * has exactly one form to recognise. It lives here rather than in `transcript.mjs`, where it started
 * as a private helper, because the third caller ({@link capFailureReply}) is not a transcript
 * concern at all — and a second copy of the marker is precisely what the "exactly one form" rule
 * cannot afford. `transcript.mjs` imports it for `capToolResult`/`capThinking`.
 *
 * @param {string} text - the text to bound.
 * @param {number} max - the maximum number of characters to keep.
 * @returns {string} `text` unchanged, or its first `max` characters plus the truncation note.
 */
export function truncate(text, max) {
  if (text.length <= max) return text;
  return `${text.slice(0, max)}\n…[truncated ${text.length - max} chars]`;
}

/** Max characters of the sidecar's automated failure reply — the text `postFailureReply` posts
 *  through `nxc reply --if-unanswered` when a session ends without answering the thread it owed.
 *
 *  Bounded for the reason `transcript.mjs`'s two caps are (PR #336 review, Integrity & Robustness
 *  #1): it is SDK-derived text of no fixed size — an arbitrary thrown `Error`'s `toString()`, which
 *  can carry a whole stream dump or a provider payload — and unlike a log line it is PERSISTED, as a
 *  message body on a thread board that other sessions later fold into their own prompts. Smaller
 *  than either transcript cap (4000/8000) because this is a DIAGNOSIS, not content: what a reader
 *  needs is which session died and why it says it died, and that is the head of the first line. */
export const MAX_FAILURE_REPLY_CHARS = 2000;

/** **How long any one `nxc` call from this sidecar may take before it is killed** (review of PR
 *  #361, Integrity #2). Milliseconds.
 *
 *  Every `nxc` call here is `execFileSync` — synchronous by design, because the teardown's whole
 *  contract is a fixed ORDER — and an `execFileSync` with no `timeout` waits forever. The realistic
 *  way that bites is the one this file already reasons about elsewhere: `.nxs/db.sqlite` is shared
 *  by flow, memory, chat AND sync, and a writer holding the lock past `busy_timeout` is a call that
 *  does not come back. One wedged call there wedges the whole teardown, and with it the process
 *  whose EXIT is what a `working_tree: exclusive` channel is waiting for (nxf 6j6v.10yb) — so an
 *  unbounded call on this path holds a checkout, not just a process.
 *
 *  Pre-existing on three calls and inherited by the two nxf 6j6v.gh7f/6j6v.10yb added; closed for
 *  all five at once rather than only for the new ones, because the hazard is the path, not the call.
 *
 *  Thirty seconds: an order of magnitude past sqlite's own five-second `busy_timeout`, so a merely
 *  contended write still succeeds, and far short of anything a human would call "hung". A killed
 *  call raises the same `err` shape every caller here already handles — for four of the five that is
 *  a warning, and for `session bind` it is the fatal it already was. */
export const NXC_CALL_TIMEOUT_MS = 30 * 1000;

/** **How long the reminder round may take before it is abandoned** (nxf 6j6v.gh7f; review of PR
 *  #361, Integrity #1). Milliseconds.
 *
 *  The reminder is an EXTRA turn taken after the stream already ended, and it runs with the same
 *  `options` as the original one — the same tools and the same permission mode. That is deliberate
 *  and unavoidable: what the reminder asks for is `nxc reply`, which needs Bash, so a text-only
 *  round could not comply with its own instruction. The cost is that a reminded session can still
 *  act, and a session that STALLS there (the SDK's own retry loop on a 529, a network stall, a model
 *  that will not stop) stays alive and tool-enabled for as long as it likes — while its member
 *  deadline can strike underneath it, because the transcript normalizer emits nothing for
 *  retry-shaped messages and a deadline only resets on real transcript writes. A lapsed member is
 *  one the engine deliberately advances past (`stale` slots are not gated on liveness, or a hung
 *  session would wedge the channel with nothing able to release it), so an unbounded reminder can
 *  widen exactly the overlap window nxf 6j6v.10yb exists to close.
 *
 *  Bounding it is the fix that fits: the cap is on the round this feature added, it is independent
 *  of the SDK's own retry policy, and abandoning the round costs only the reminder — what follows is
 *  what the teardown did before the reminder existed.
 *
 *  Five minutes: far longer than a session needs to run one command, far shorter than the two-hour
 *  working-tree bound it has to stay well inside.
 *
 *  It lives HERE rather than in `main.mjs` for `MAX_FAILURE_REPLY_CHARS`'s reason — a tunable a test
 *  asserts against belongs where a test can import it. */
export const REPLY_REMINDER_BOUND_MS = 5 * 60 * 1000;

/** **The exit code of a session that was told to stop** (nxf 6j6v.b9nf) — `SidecarWorker::
 *  stop_session` sends the sidecar SIGTERM, the sidecar catches it, aborts the running turn, runs
 *  its teardown and exits with this.
 *
 *  128 + 15: the number a shell reports for a process that DIED of SIGTERM, kept for one that caught
 *  it. So whoever reads a session's log sees one number for "stopped", whether or not the teardown
 *  got to run, and never the `0` of a turn that finished or the `1` of one that failed — a stop is
 *  neither. Nothing reads the code today (`SidecarWorker::trigger` spawns detached and returns
 *  `Accepted`); it is fixed here, where a test can import it, so the first consumer that waits on
 *  the sidecar is not left to guess which of three endings it saw. */
export const STOPPED_EXIT_CODE = 128 + 15;

/** **The coordinator's terms for this turn, read off the spec** (nxf 6j6v.ntp9, answering nxf
 *  6j6v.553s question (b)).
 *
 *  These used to be constants of this file — `MAX_REPLY_REMINDERS` was one — which made both bounds
 *  a property of the BUNDLED runtime rather than of the engine. A host that installs its own
 *  `WorkerConfig::Custom` worker inherits engine verbs; it does not inherit a number in
 *  `main.mjs`. So the engine states them on every trigger (`crate::worker::TurnTerms`) and this
 *  reads them back.
 *
 *  **The fallbacks are what an OLDER engine means, not a second opinion.** A spec written before
 *  that item carries none of these keys, and the right reading of its silence is the behaviour that
 *  existed then: one reminder, and no retry at all. A non-integer or negative value reads the same
 *  way — this is a bound, and a bound that cannot be parsed must not become "unbounded".
 *
 *  **And it is clamped at the TOP as well** (independent review of PR #378, Integrity #2). The
 *  earlier cut checked `value >= 0` only, so a large value passed straight through — and a waiting
 *  sidecar is not idle: it holds the `working_tree: exclusive` claim, and the release is its own
 *  process EXIT (nxf 6j6v.10yb). The delay overflow is worse than merely long, because it is silent:
 *  Node collapses any `setTimeout` above `2**31 - 1` to **1 ms**, so an unclamped doubling backoff
 *  stops backing off altogether and the bounded retry becomes a hot spawn loop. The clamp is the
 *  runtime's own safety bound, stated here and named on `crate::worker::TurnTerms` at the other end;
 *  every value the engine actually ships is far inside it.
 *
 *  @returns {{replyReminders: number, runtimeRetries: number, retryBackoffMs: number}}
 */
export function resolveTurnTerms(spec) {
  const bound = (value, fallback, ceiling) =>
    Number.isInteger(value) && value >= 0 ? Math.min(value, ceiling) : fallback;
  return {
    // No ceiling worth naming: a reminder is one paid turn taken once at teardown, it holds nothing
    // and cannot compound. `Number.MAX_SAFE_INTEGER` keeps the shape of the other two rather than
    // inventing a limit nobody needs.
    replyReminders: bound(spec?.replyReminders, 1, Number.MAX_SAFE_INTEGER),
    runtimeRetries: bound(spec?.runtimeRetries, 0, MAX_RUNTIME_RETRIES),
    // Not `bound(..., 0, …)`: a zero here would turn a retry into an immediate re-attempt against a
    // runtime that has had no time to recover, which is the one thing backing off is for.
    retryBackoffMs: bound(spec?.retryBackoffMs, 5000, MAX_RETRY_DELAY_MS) || 5000,
  };
}

/** **The ceiling on how many times one turn may be started again.** Five, and the ceiling is the
 *  point rather than the number: what it bounds is a spawn loop holding a checkout, not an
 *  arithmetic overflow. Together with {@link MAX_RETRY_DELAY_MS} it bounds the whole retry at
 *  2.5 minutes of held claim in the worst case. */
export const MAX_RUNTIME_RETRIES = 5;

/** **The ceiling on ONE wait between runtime retries**, in milliseconds — and the reason it exists
 *  is not tidiness. Past `2**31 - 1` Node warns `TimeoutOverflowWarning` and sets the delay to
 *  **1 ms**, so an unclamped doubling backoff does not merely grow wrong, it DISAPPEARS.
 *
 *  Thirty seconds: long enough for a transient overload to clear, short enough that five of them
 *  stay well inside the shortest window a board is likely to declare. */
export const MAX_RETRY_DELAY_MS = 30 * 1000;

/** How long to wait before the attempt after `attempt`: the coordinator's base delay, doubled per
 *  attempt already spent, clamped at {@link MAX_RETRY_DELAY_MS}.
 *
 *  Pure, and here rather than in `main.mjs`, for {@link MAX_FAILURE_REPLY_CHARS}'s reason — a bound a
 *  test has to assert belongs where a test can import it. Asserting it through the real sleep would
 *  mean a test that waits exactly as long as the thing it is checking.
 */
export function retryDelay(terms, attempt) {
  return Math.min(terms.retryBackoffMs * 2 ** attempt, MAX_RETRY_DELAY_MS);
}

/** Cap an automated failure reply to {@link MAX_FAILURE_REPLY_CHARS}. Applied to the WHOLE composed
 *  reply rather than to the error alone, so what is bounded is exactly what reaches `nxc`. Pure —
 *  unit-tested. */
export function capFailureReply(text) {
  return truncate(text, MAX_FAILURE_REPLY_CHARS);
}

/**
 * Decide what main.mjs should assign to the SDK's `options.settingSources` (filesystem settings
 * isolation for role runtime sessions).
 *
 * Role Runtime v2 requires every role session to be self-contained and not depend on the
 * operator's or the project's `.claude/` plugin/skill/hook configuration (ticket 6j6v.93hz).
 * Per the SDK's own type docs (sdk.d.ts), passing `[]` disables filesystem settings (SDK
 * isolation mode). This is unconditional for sidecar sessions: there is no scenario where
 * loading filesystem settings is wanted.
 *
 * @returns {string[]} always returns an empty array for SDK isolation mode.
 */
export function resolveSettingsSourcesOption() {
  return [];
}
