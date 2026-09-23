// The sidecar's I/O half, under test at last (nxf epic 6wt2, ticket 5bym — carried over from
// ticket 0dbp/T1). `spec-helpers.test.mjs` and `transcript.test.mjs` cover the pure modules; this
// file covers `main.mjs` itself: the flush lifecycle, the bind→flush ORDER, and error propagation
// on a throwing stream. Those paths had no committed coverage at all — T1's implementer and its
// reviewer each proved them with a throwaway scratch harness and then deleted it — even though the
// epic's "no transcript data is dropped on the floor" property lives exactly there, and one of the
// four cases below guards a regression that was introduced and caught during T1's own fix round.
//
// `main.mjs` runs `query()` at import time and shells out to `nxc`, so it is exercised as a CHILD
// PROCESS against a scratch environment, never imported:
//
//   * the SDK is redirected, not replaced: an `--import`ed `register.mjs` installs a module-resolve
//     hook that maps the bare specifier `@anthropic-ai/claude-agent-sdk` to a generated stub whose
//     `query()` yields a scripted `SDKMessage` sequence (and optionally throws). Node's ESM
//     resolution ignores NODE_PATH and would otherwise walk up to `agent-sidecar/node_modules` and
//     find the REAL SDK; the hook short-circuits before that, so nothing under `node_modules` is
//     touched, no install is needed (CI runs this suite with no `npm ci`), and the file under test
//     is the real `src/main.mjs`, byte for byte;
//   * `nxc` is a fake on `PATH` that appends `{argv, stdin}` to a call log and can be told, per
//     `"<verb> <subverb>"`, to exit with a chosen status and stderr.
//
// `main.mjs` is NOT weakened to make any of this possible — there is no "if under test" branch in
// it, and there must never be one: the whole value here is that the shipped file is what runs.

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { MAX_FAILURE_REPLY_CHARS, REPLY_REMINDER_BOUND_MS, STOPPED_EXIT_CODE } from "../src/spec-helpers.mjs";

const MAIN = fileURLToPath(new URL("../src/main.mjs", import.meta.url));

/** The scripted stream error, asserted on verbatim: the ORIGINAL cause must stay visible. */
const STREAM_ERROR = "scripted stream failure";

// ---- scripted SDK messages ------------------------------------------------------------------

const init = {
  type: "system",
  subtype: "init",
  session_id: "real-1",
  model: "claude-opus-5",
  apiKeySource: "none",
  tools: ["Read"],
  skills: [],
  slash_commands: [],
  permissionMode: "default",
};

const toolUse = {
  type: "assistant",
  session_id: "real-1",
  parent_tool_use_id: null,
  message: { content: [{ type: "tool_use", id: "toolu_1", name: "Read", input: { file_path: "/x" } }] },
};

const toolResult = {
  type: "user",
  session_id: "real-1",
  parent_tool_use_id: null,
  message: { content: [{ type: "tool_result", tool_use_id: "toolu_1", content: "fn main() {}" }] },
};

const result = {
  type: "result",
  subtype: "success",
  session_id: "real-1",
  is_error: false,
  num_turns: 1,
  total_cost_usd: 0.01,
  result: "done",
};

/** **What the SDK actually emits when an API request keeps failing** (`sdk.d.ts`:
 *  `SDKAPIRetryMessage`, doc: "Emitted when an API request fails with a retryable error and will be
 *  retried after a delay"). It is a `system` message, like `init` — the runtime talking about
 *  itself, not the model producing anything. */
const apiRetry = {
  type: "system",
  subtype: "api_retry",
  attempt: 1,
  max_retries: 3,
  retry_delay_ms: 1000,
  error_status: 529,
  session_id: "real-1",
};

/** **And what it emits when it gives up** (`sdk.d.ts`: `SDKResultError`). The stream ends NORMALLY
 *  here — `for await` simply stops — which is why a `catch`-only loop never saw the one failure the
 *  retry exists for. */
const overloadedResult = {
  type: "result",
  subtype: "error_during_execution",
  session_id: "real-1",
  is_error: true,
  num_turns: 0,
  api_error_status: 529,
  errors: ["overloaded_error"],
};

/** The whole of a `529` at the door: the session starts, the SDK retries internally, it gives up.
 *  **Not one model turn in it** — which is exactly what makes replaying it free. */
const A_529_AT_THE_DOOR = [init, apiRetry, overloadedResult];

/** A turn that produced real model output and THEN ended in an error — the mid-turn tear. Replaying
 *  this would redo whatever `toolUse` did. */
const MODEL_OUTPUT_THEN_AN_ERROR = [init, toolUse, toolResult, overloadedResult];

const textDelta = (text) => ({
  type: "stream_event",
  session_id: "real-1",
  parent_tool_use_id: null,
  event: { type: "content_block_delta", delta: { type: "text_delta", text } },
});

/** A complete turn (flushed mid-run at the `result` boundary) followed by a trailing assistant
 *  block that is still OPEN when the stream ends — only the teardown `normalizer.flush()` can
 *  produce it, which is exactly the entry the ordering rules below must not lose. */
const A_TURN_THEN_AN_OPEN_BLOCK = [
  init,
  toolUse,
  toolResult,
  result,
  textDelta("trailing "),
  textDelta("answer"),
];

/** A long turn: 40 tool round-trips (80 entries, so it crosses `FLUSH_THRESHOLD` twice) before the
 *  `result` that flushes a third time. Text deltas would NOT do — the normalizer coalesces them
 *  into one entry however many arrive, so only distinct blocks grow the buffer. */
const MANY_TOOL_CALLS = [
  init,
  ...Array.from({ length: 40 }, (_, i) => [
    { ...toolUse, message: { content: [{ ...toolUse.message.content[0], id: `toolu_${i}` }] } },
    {
      ...toolResult,
      message: { content: [{ ...toolResult.message.content[0], tool_use_id: `toolu_${i}` }] },
    },
  ]).flat(),
  result,
];

// ---- the harness ------------------------------------------------------------------------------

/**
 * Build a scratch environment and run the REAL `main.mjs` against it.
 *
 * @param {import("node:test").TestContext} t - the test's own context; the scratch dir is removed
 *   through its `after` hook (four temp trees per run would otherwise accumulate in $TMPDIR).
 * @param {object} opts
 * @param {object[]} opts.messages - the scripted `SDKMessage` sequence `query()` yields.
 * @param {number} [opts.throwAfter] - yield this many messages, then throw {@link STREAM_ERROR}.
 * @param {string} [opts.errorText] - the message the scripted throw carries, when it must be
 *   something other than {@link STREAM_ERROR} (an unbounded one, for the reply cap).
 * @param {number} [opts.poisonAt] - index of a `tool_use` message whose `input` is replaced, at
 *   yield time, with an object `JSON.stringify` throws on. It has to be built inside the stub:
 *   the scripted messages travel into it as JSON, and JSON cannot express a value that refuses to
 *   serialize. This is the only way to reach `flushTranscript`'s serialization step from a test.
 * @param {Record<string, {status: number, stderr?: string}>} [opts.nxcFailures] - keyed by the
 *   fake `nxc`'s first two argv tokens (e.g. `"session bind"`), how that call should fail.
 * @param {object} [opts.spec] - extra spec fields merged over the default scratch spec, for the
 *   fields whose whole effect is which `options` reach `query()` (e.g. `claudePath`).
 * @param {number} [opts.hangAt] - index of the message BEFORE which the stub stops yielding and
 *   waits, the way the real SDK waits on a model that is still working: it settles only when the
 *   `abortController` main.mjs handed it fires (then it throws the SDK's own `AbortError`), and
 *   never otherwise. While it waits it writes `hanging.txt` into the scratch dir, which is what
 *   {@link runSidecarAndStopIt} watches for before it sends the signal. Only that runner uses
 *   this; under `spawnSync` a hang would be a hang.
 * @param {number} [opts.hangInRound] - which `query()` round `hangAt` applies to (default `0`,
 *   the main turn; `1` is the reminder round).
 * @returns {{ status: number, stderr: string, stdout: string, query: object | null,
 *   calls: {argv: string[], stdin: string}[], consumed: number }} — `query` is the single argument
 *   main.mjs passed `query()`, i.e. `{ prompt, options }`, recorded by the stub. `consumed` is how
 *   many scripted messages the `for await` loop took AND handed control back for: it counts a
 *   message only once the loop body returned to the generator, so a loop body that unwinds — which
 *   ABANDONS the generator, ending the turn mid-answer — leaves that message uncounted. It is the
 *   only thing that can tell "the turn ran to the end of its stream" apart from "the turn died
 *   quietly after the same visible flushes" (nxf 6j6v.jwgc). Written from the stub's `finally`, so
 *   it is recorded on the abandoning path too.
 */
function runSidecar(t, opts) {
  const { argv, env, collect } = scratch(t, opts);
  const run = spawnSync(process.execPath, argv, {
    encoding: "utf8",
    // stdin is closed immediately: `nxc session bind` INHERITS it (main.mjs), and the fake nxc
    // reads fd 0, so an open stdin would hang the whole run.
    input: "",
    env,
  });
  return collect(run);
}

/**
 * The scratch environment {@link runSidecar} runs in, built but not run: the SDK stub, the loader
 * that redirects the bare specifier to it, the fake `nxc` and the spec file. Split out so that a
 * case which has to reach the sidecar WHILE IT RUNS — a signal mid-query, which `spawnSync` cannot
 * deliver — runs the same file against the same environment through `spawn` instead.
 *
 * @returns {{ dir: string, argv: string[], env: object, collect: (run: {status: number|null,
 *   signal: string|null, stderr: string, stdout: string}) => object }} — `collect` turns a finished
 *   run into the result shape `runSidecar` documents.
 */
function scratch(
  t,
  {
    messages,
    throwAfter = null,
    nxcFailures = {},
    nxcStdout = {},
    nxcDelays = {},
    spec = {},
    poisonAt = null,
    errorText = STREAM_ERROR,
    laterRoundMessages = null,
    reminderThrows = false,
    throwEveryRound = false,
    hangAt = null,
    hangInRound = 0,
  },
) {
  const dir = mkdtempSync(join(tmpdir(), "nxc-sidecar-teardown-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const callLog = join(dir, "calls.log");
  const queryLog = join(dir, "query-arg.json");
  const consumedLog = join(dir, "consumed.txt");
  writeFileSync(callLog, "");

  // The stub also RECORDS what it was called with: `options` is how main.mjs configures the real
  // session, and two of its flags (`includePartialMessages`, `forwardSubagentText`) are the entire
  // reason a transcript has reasoning and subagent activity in it at all. Nothing else can pin them
  // deterministically — the live smoke's own failure message admits it cannot tell a regressed flag
  // apart from a model that simply did not think or delegate this run.
  // ROUND-AWARE (nxf 6j6v.gh7f): the teardown may call `query()` a SECOND time to resume the
  // session and remind it, so the stub scripts a round at a time and records every argument rather
  // than the last. `laterRoundMessages` scripts round two; without it round two replays round one,
  // which is what every pre-gh7f case wants and what keeps their assertions untouched. `throwAfter`
  // and `poisonAt` stay bound to round one, where each of them is about the main turn.
  writeFileSync(
    join(dir, "sdk-stub.mjs"),
    `import { appendFileSync, writeFileSync } from "node:fs";\n` +
      `const ROUNDS = ${JSON.stringify([messages, laterRoundMessages ?? messages])};\n` +
      `const THROW_AFTER = ${JSON.stringify(throwAfter)};\n` +
      `const POISON_AT = ${JSON.stringify(poisonAt)};\n` +
      `const REMINDER_THROWS = ${JSON.stringify(reminderThrows)};\n` +
      // ROUND-INDEPENDENT THROWING (nxf 6j6v.ntp9): `THROW_AFTER` is bound to round one, because
      // every case before this item was about the main turn. A RUNTIME RETRY re-runs the main turn
      // in round two, three, … — so proving that the retries are BOUNDED needs a stream that keeps
      // failing. Default `false` leaves every existing case exactly as it was.
      `const THROW_EVERY_ROUND = ${JSON.stringify(throwEveryRound)};\n` +
      // PER-ROUND THROWING: an ARRAY `throwAfter` scripts each round separately, which is the only
      // way to express "attempt one produced nothing, attempt two got further and then tore" — the
      // shape the cumulative half of `agentEverThought` exists for, and the one a single number
      // cannot say (review of PR #378, Test Quality #4).
      `const THROW_AFTER_IS_LIST = ${JSON.stringify(Array.isArray(throwAfter))};\n` +
      // A STREAM THAT WAITS (nxf 6j6v.b9nf): the real `query()` blocks between messages for as long
      // as the model takes, and the only door out of that wait is the `abortController` the caller
      // handed in — abort it and the SDK throws an `AbortError` out of `for await`. The stub does
      // exactly that and nothing looser: no abort controller, or one that never fires, and the wait
      // is forever, which is what makes a SIGTERM that does not reach the abort observable as a
      // hang rather than as a pass.
      `const HANG_AT = ${JSON.stringify(hangAt)};\n` +
      `const HANG_IN_ROUND = ${JSON.stringify(hangInRound)};\n` +
      `const HANGING = ${JSON.stringify(join(dir, "hanging.txt"))};\n` +
      // The interval is what the real SDK's child process and pipes are to the event loop: a
      // HANDLE, so the process stays up while nothing is happening. A bare pending promise keeps
      // nothing alive — without this Node drains the loop, reports an unsettled top-level await and
      // exits 13 before any signal can arrive.
      `function untilAborted(signal) {\n` +
      `  return new Promise((_, reject) => {\n` +
      `    const held = setInterval(() => {}, 1000);\n` +
      `    const abort = () => { clearInterval(held); const e = new Error("Request was aborted."); e.name = "AbortError"; reject(e); };\n` +
      `    if (signal?.aborted) return abort();\n` +
      `    signal?.addEventListener("abort", abort, { once: true });\n` +
      `  });\n` +
      `}\n` +
      `let round = 0;\n` +
      `export async function* query(arg) {\n` +
      `  const roundIndex = round;\n` +
      `  const throwsThisRound = THROW_AFTER_IS_LIST\n` +
      `    ? (THROW_AFTER[roundIndex] ?? null)\n` +
      `    : ((roundIndex === 0 || THROW_EVERY_ROUND) ? THROW_AFTER : null);\n` +
      `  const first = round === 0;\n` +
      `  if (!first && REMINDER_THROWS) { round += 1; throw new Error("reminder stream failure"); }\n` +
      `  const MESSAGES = ROUNDS[Math.min(round, ROUNDS.length - 1)];\n` +
      `  round += 1;\n` +
      `  appendFileSync(${JSON.stringify(queryLog)}, JSON.stringify(arg ?? null) + "\\n");\n` +
      `  let consumed = 0;\n` +
      `  try {\n` +
      `    for (const [i, m] of MESSAGES.entries()) {\n` +
      `      if (throwsThisRound !== null && i >= throwsThisRound) throw new Error(${JSON.stringify(errorText)});\n` +
      `      if (HANG_AT !== null && roundIndex === HANG_IN_ROUND && i === HANG_AT) {\n` +
      `        writeFileSync(HANGING, String(i));\n` +
      `        await untilAborted(arg?.options?.abortController?.signal);\n` +
      `      }\n` +
      `      if (first && i === POISON_AT) {\n` +
      `        m.message.content[0].input = { get unserializable() { throw new Error("no JSON for you"); } };\n` +
      `      }\n` +
      `      yield m;\n` +
      `      consumed = i + 1;\n` +
      `    }\n` +
      `    if (throwsThisRound !== null) throw new Error(${JSON.stringify(errorText)});\n` +
      `    if (HANG_AT !== null && roundIndex === HANG_IN_ROUND && HANG_AT >= MESSAGES.length) {\n` +
      `      writeFileSync(HANGING, String(MESSAGES.length));\n` +
      `      await untilAborted(arg?.options?.abortController?.signal);\n` +
      `    }\n` +
      `  } finally {\n` +
      `    if (round === 1) writeFileSync(${JSON.stringify(consumedLog)}, String(consumed));\n` +
      `  }\n` +
      `}\n`,
  );
  writeFileSync(
    join(dir, "loader.mjs"),
    `const STUB = ${JSON.stringify(pathToFileURL(join(dir, "sdk-stub.mjs")).href)};\n` +
      `export async function resolve(specifier, context, next) {\n` +
      `  if (specifier === "@anthropic-ai/claude-agent-sdk") return { url: STUB, shortCircuit: true };\n` +
      `  return next(specifier, context);\n` +
      `}\n`,
  );
  writeFileSync(
    join(dir, "register.mjs"),
    `import { register } from "node:module";\n` +
      `register(new URL("./loader.mjs", import.meta.url));\n`,
  );

  // CommonJS on purpose: an extensionless executable has no `.mjs` to key module type off and no
  // package.json above /tmp, so Node reads it as CJS — `require` keeps that unambiguous.
  mkdirSync(join(dir, "bin"));
  const fakeNxc = join(dir, "bin", "nxc");
  writeFileSync(
    fakeNxc,
    `#!/usr/bin/env node\n` +
      `const { appendFileSync, readFileSync } = require("node:fs");\n` +
      `const argv = process.argv.slice(2);\n` +
      `let stdin = "";\n` +
      `try { stdin = readFileSync(0, "utf8"); } catch { stdin = ""; }\n` +
      `appendFileSync(process.env.NXC_CALL_LOG, JSON.stringify({ argv, stdin }) + "\\n");\n` +
      `const key = argv.slice(0, 2).join(" ");\n` +
      // A call that TAKES a while (nxf 6j6v.27b9): the teardown runs its `nxc` calls synchronously,
      // so a slow one is a window in which the sidecar is alive and mid-teardown — the window a
      // second SIGTERM has to land in to be tested at all. Keyed like the failures, in ms.
      `const delay = JSON.parse(process.env.NXC_DELAYS || "{}")[key];\n` +
      `if (delay) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, delay);\n` +
      // Scripted stdout, keyed like the failures and consumed ONE ANSWER PER CALL (nxf 6j6v.gh7f):
      // the teardown reads the board before the reminder and again after it, and those two reads
      // have to be able to differ — that difference IS what "the reminder worked" means. A single
      // string answers every call; the last entry of a list repeats once the list runs out.
      `const scripted = JSON.parse(process.env.NXC_STDOUT || "{}")[key];\n` +
      `if (scripted !== undefined) {\n` +
      `  const answers = Array.isArray(scripted) ? scripted : [scripted];\n` +
      // THIS call is already in the log (appended above), so the number of EARLIER calls with the
      // same key is that count minus one — which is the index into the scripted answers.
      `  const nth = readFileSync(process.env.NXC_CALL_LOG, "utf8").split("\\n")\n` +
      `    .filter((l) => l.trim() !== "").map((l) => JSON.parse(l))\n` +
      `    .filter((c) => c.argv.slice(0, 2).join(" ") === key).length - 1;\n` +
      `  process.stdout.write(answers[Math.min(nth, answers.length - 1)]);\n` +
      `}\n` +
      `const failure = JSON.parse(process.env.NXC_FAILURES || "{}")[key];\n` +
      `if (failure) {\n` +
      `  if (failure.stderr) process.stderr.write(failure.stderr);\n` +
      `  process.exit(failure.status);\n` +
      `}\n`,
  );
  chmodSync(fakeNxc, 0o755);

  writeFileSync(
    join(dir, "spec.json"),
    JSON.stringify({ session: "s-1", role: "tester", message: "hi", cwd: dir, env: {}, ...spec }),
  );

  const argv = ["--import", join(dir, "register.mjs"), MAIN, "--spec", join(dir, "spec.json")];
  const env = {
    ...process.env,
    PATH: `${join(dir, "bin")}:${process.env.PATH}`,
    NXC_CALL_LOG: callLog,
    NXC_FAILURES: JSON.stringify(nxcFailures),
    NXC_STDOUT: JSON.stringify(nxcStdout),
    NXC_DELAYS: JSON.stringify(nxcDelays),
  };

  const collect = (run) => {
    const calls = readFileSync(callLog, "utf8")
      .split("\n")
      .filter((line) => line.trim() !== "")
      .map((line) => JSON.parse(line));
    const queries = existsSync(queryLog)
      ? readFileSync(queryLog, "utf8")
          .split("\n")
          .filter((line) => line.trim() !== "")
          .map((line) => JSON.parse(line))
      : [];
    const query = queries[0] ?? null;
    const consumed = existsSync(consumedLog) ? Number(readFileSync(consumedLog, "utf8")) : 0;
    return {
      status: run.status,
      signal: run.signal ?? null,
      stderr: run.stderr ?? "",
      stdout: run.stdout ?? "",
      query,
      queries,
      calls,
      consumed,
    };
  };
  return { dir, argv, env, collect };
}

/**
 * Run the REAL `main.mjs` and send it `SIGTERM` while its `query()` is waiting on the model
 * (nxf 6j6v.b9nf) — the way `SidecarWorker::stop_session` stops a session. The stub must be told
 * where to wait (`hangAt`); the signal goes out the moment it reports that it is.
 *
 * `spawn`, not `spawnSync`: a synchronous run cannot be signalled from the test that started it.
 * Same argv, same environment, same collection as {@link runSidecar} — the only added field is
 * `signal`, which is how a process that DIED of the signal (no handler) is told apart from one
 * that caught it and exited on its own terms.
 *
 * `opts.stopWhen` (a RegExp) signals when the sidecar's own STDERR says so instead, for the windows
 * that are not inside a `query()` at all — the retry backoff is one, and nothing in the stub can
 * report it.
 *
 * `opts.signalAgainWhen` (a RegExp) sends a SECOND `SIGTERM` once stderr matches it — the one the
 * engine's tick now sends a withdrawn session that still pins its claim (nxf 6j6v.27b9).
 */
async function runSidecarAndStopIt(t, opts) {
  const { dir, argv, env, collect } = scratch(t, opts);
  const child = spawn(process.execPath, argv, { env, stdio: ["pipe", "pipe", "pipe"] });
  // Whatever way this test ends — an assertion below included — no sidecar outlives it.
  t.after(() => {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  });
  child.stdin.end();
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (chunk) => (stdout += chunk));
  child.stderr.on("data", (chunk) => (stderr += chunk));
  const exited = new Promise((resolve) => child.on("close", (status, signal) => resolve({ status, signal })));

  // Wait for the moment this case is about — the stub reporting that it is inside its wait, or a
  // line of the sidecar's own stderr — then signal. Bounded, so a run that never gets there fails
  // the test rather than the suite.
  const hanging = join(dir, "hanging.txt");
  const arrived = opts.stopWhen ? () => opts.stopWhen.test(stderr) : () => existsSync(hanging);
  const missed = opts.stopWhen
    ? `stderr never matched ${opts.stopWhen}`
    : "the stub never reached its wait";
  const deadline = Date.now() + 10_000;
  while (!arrived()) {
    assert.ok(Date.now() < deadline, `${missed}; stderr so far:\n${stderr}`);
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  child.kill("SIGTERM");
  let signalledAgain = null;
  if (opts.signalAgainWhen) {
    const again = Date.now() + 10_000;
    while (!opts.signalAgainWhen.test(stderr)) {
      assert.ok(Date.now() < again, `stderr never matched ${opts.signalAgainWhen}; so far:\n${stderr}`);
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    // `true` only if the process was still there to receive it.
    signalledAgain = child.kill("SIGTERM");
  }

  // And bound the exit too: a sidecar that catches the signal and then never exits is precisely
  // the regression a handler can introduce, and it has to fail loudly here rather than hang.
  const timer = setTimeout(() => child.kill("SIGKILL"), 10_000);
  const { status, signal } = await exited;
  clearTimeout(timer);
  return { ...collect({ status, signal, stdout, stderr }), signalledAgain };
}

/** The `nxc transcript append` calls, each with its JSON-lines stdin parsed back into entries. */
function appends(calls) {
  return calls
    .filter((c) => c.argv[0] === "transcript" && c.argv[1] === "append")
    .map((c) => ({
      session: c.argv[c.argv.indexOf("--session") + 1],
      entries: c.stdin
        .split("\n")
        .filter((line) => line.trim() !== "")
        .map((line) => JSON.parse(line)),
    }));
}

const verbs = (calls) => calls.map((c) => c.argv.slice(0, 2).join(" "));

/** The `nxc reply --thread <id> --if-unanswered [--escalate] <text>` calls (ticket 8's teardown
 *  step), decoded from their argv.
 *
 *  `--escalate` is OPTIONAL and appears only when the session DIED (nxf 6j6v.mqad), so the flags are
 *  read by name and the text is the last argument rather than a fixed index — a positional decode
 *  would silently read the flag as the message body. */
function replyCalls(calls) {
  return calls
    .filter((c) => c.argv[0] === "reply" && c.argv[1] === "--thread")
    .map((c) => ({
      thread: c.argv[2],
      ifUnanswered: c.argv.includes("--if-unanswered"),
      escalate: c.argv.includes("--escalate"),
      text: c.argv[c.argv.length - 1],
    }));
}

// ---- the four cases -----------------------------------------------------------------------------

test("happy path: entries flush in order, bind precedes the final flush, exit 0", (t) => {
  const { status, stderr, calls, query, consumed } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
  });

  assert.equal(status, 0, `expected a clean exit, stderr:\n${stderr}`);
  assert.equal(consumed, A_TURN_THEN_AN_OPEN_BLOCK.length, "the whole stream was consumed");
  assert.match(stderr, /sidecar done: role=tester session=s-1 real=real-1/);
  // The `transcript=incomplete` marker is the EXCEPTION and must stay one: a run that captured
  // everything says nothing, so the marker never becomes background noise a reader learns to skip.
  assert.doesNotMatch(stderr, /transcript=incomplete/);

  // The two flags the whole capture rests on, pinned deterministically (main.mjs:40,47).
  // `includePartialMessages` is the ONLY route extended thinking travels (there is no final
  // thinking block to fall back on), and `forwardSubagentText` is what makes a subagent contribute
  // anything beyond bare tool calls — the exact gap the beads blueprint inherited by leaving it off.
  // The live smoke cannot tell either flag regressing apart from a model that just did not think or
  // delegate on that run; this can, without spending a token.
  assert.equal(query.prompt, "hi", "the spec's message is the prompt");
  assert.equal(query.options.includePartialMessages, true);
  assert.equal(query.options.forwardSubagentText, true);

  // THE ORDER IS THE CONTRACT (main.mjs's own "ORDER IS LOAD-BEARING" comment): a `result` flushes
  // the completed turn mid-run, then teardown binds the session BEFORE the final flush — losing
  // transcript entries is recoverable, losing the internal↔real mapping is not.
  assert.deepEqual(verbs(calls), [
    "transcript append",
    "session bind",
    "transcript append",
    "session ended",
  ]);
  assert.deepEqual(calls[1].argv, ["session", "bind", "s-1", "real-1"]);
  // THE LAST WORD (nxf 6j6v.10yb): every run that has an internal session announces its own end,
  // whatever else the teardown did. It has to be last — a step of a `working_tree: exclusive`
  // channel is released by it, and releasing before the reply above would hand the next step a
  // thread that is still owed an answer.
  assert.deepEqual(calls[3].argv, ["session", "ended", "s-1"]);

  const flushes = appends(calls);
  assert.equal(flushes[0].session, "s-1");
  assert.deepEqual(
    flushes[0].entries.map((e) => e.kind),
    ["session_init", "tool_use", "tool_result", "result"],
    "the completed turn flushes at its `result` boundary, in stream order",
  );
  assert.equal(flushes[0].entries[1].toolUseId, "toolu_1");
  // The trailing block was still OPEN when the stream ended: only `normalizer.flush()` at teardown
  // produces it, and it lands in the SECOND flush — never duplicated into the first (main.mjs
  // clears its buffer as part of attempting each write).
  assert.deepEqual(
    flushes[1].entries.map((e) => e.kind),
    ["assistant"],
  );
  assert.equal(flushes[1].entries[0].data.text, "trailing answer");
});

test("thinking is asked for in SUMMARIZED display, which is what makes it reach the stream at all", (t) => {
  // nxf 6j6v.w4wa. The transcript's thinking half was implemented, unit-tested and never once
  // exercised live: on this SDK, over the subscription/CLI auth path, `thinking_delta` frames
  // arrive EMPTY unless a display mode is asked for — the blocks are there, redacted, carrying
  // zero characters. Probed on the pinned 0.3.215 with one reasoning-heavy prompt, apiKeySource
  // "none", everything else exactly as shipped:
  //
  //   (as shipped, no `thinking` option)                          -> thinking 0 chars
  //   thinking: { type: 'adaptive',  display: 'summarized' }      -> thinking 829 chars
  //   thinking: { type: 'enabled', budgetTokens: 4096, display }  -> thinking 1078 chars
  //
  // `adaptive` and not `enabled`, because adaptive IS the model's own default policy ("Claude
  // decides when and how much to think") — so this asks to SEE the thinking that already happens
  // rather than buying a fixed budget of it, and `enabled`'s budgetTokens would override that
  // policy with the older fixed-budget mode. Verified live to throw on none of the three models a
  // role can pin (fable/opus/sonnet): where a model does not think, it is simply a no-op.
  const { query, status, stderr } = runSidecar(t, { messages: A_TURN_THEN_AN_OPEN_BLOCK });

  assert.equal(status, 0, `expected a clean exit, stderr:\n${stderr}`);
  assert.deepEqual(query.options.thinking, { type: "adaptive", display: "summarized" });
});

test("the engine's grant reaches the SDK's approval list without narrowing the base toolset", (t) => {
  // nxf 6j6v.kffm, at the file that actually ships. `spec-helpers.test.mjs` pins the two decision
  // functions; this pins that `main.mjs` USES them — reverting the one line back to
  // `allowedTools: spec.tools ?? []` leaves every helper test green and puts the bug straight back.
  //
  // The trap's own spec: a role with NO `tools:` key, under an obligation. It got the SDK's full
  // base toolset and an EMPTY approval list, so its own ordered `nxc reply` came back "This command
  // requires approval" and the teardown answered in its name.
  const granted = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { tools: undefined, grantedTools: ["Bash"] },
  });
  assert.equal(granted.status, 0, `expected a clean exit, stderr:\n${granted.stderr}`);
  assert.deepEqual(granted.query.options.allowedTools, ["Bash"]);
  assert.ok(
    !("tools" in granted.query.options),
    "an undeclared base toolset stays unset — folding the grant in there would narrow the session " +
      `from the SDK's full default set to exactly the grant, got ${JSON.stringify(granted.query.options.tools)}`,
  );

  // A role that DID declare a toolset keeps it, and gains the grant in both lists — which is what
  // lets an explicitly narrow role discharge an obligation its author never foresaw.
  const narrow = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { tools: ["Read"], grantedTools: ["Bash"] },
  });
  assert.equal(narrow.status, 0, `expected a clean exit, stderr:\n${narrow.stderr}`);
  assert.deepEqual(narrow.query.options.allowedTools, ["Read", "Bash"]);
  assert.deepEqual(narrow.query.options.tools, ["Read", "Bash"]);

  // And an engine older than the grant — or a trigger that demands nothing — is byte-identical to
  // what it always was.
  const ungranted = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { tools: ["Read"], grantedTools: undefined },
  });
  assert.equal(ungranted.status, 0, `expected a clean exit, stderr:\n${ungranted.stderr}`);
  assert.deepEqual(ungranted.query.options.allowedTools, ["Read"]);
  assert.deepEqual(ungranted.query.options.tools, ["Read"]);
});

test("the spec's claudePath becomes the SDK's executable, and its absence leaves the SDK's own resolution alone", (t) => {
  // nxf 6j6v.81v5. The SHIPPED sidecar is one bundled file with no `node_modules` beside it, so the
  // SDK cannot resolve the native `claude` itself — the host resolves it and states it here. This
  // is the whole reason an installed `nxc send --to <persona>` reaches a session at all, and it is
  // invisible to every other test: the stubbed `query()` never looks at the option.
  const withPath = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { claudePath: "/opt/claude/bin/claude" },
  });
  assert.equal(withPath.status, 0, `expected a clean exit, stderr:\n${withPath.stderr}`);
  assert.equal(withPath.query.options.pathToClaudeCodeExecutable, "/opt/claude/bin/claude");

  // Absent, empty, or not a string: the option must not appear AT ALL. `undefined` would be a
  // different thing to the SDK than "unset" is here only by luck; an empty string would send it
  // looking for an executable named "".
  for (const claudePath of [undefined, "", 7]) {
    const run = runSidecar(t, { messages: A_TURN_THEN_AN_OPEN_BLOCK, spec: { claudePath } });
    assert.equal(run.status, 0, `expected a clean exit, stderr:\n${run.stderr}`);
    assert.ok(
      !("pathToClaudeCodeExecutable" in run.query.options),
      `claudePath ${JSON.stringify(claudePath)} must leave the SDK's own resolution untouched, got ` +
        JSON.stringify(run.query.options.pathToClaudeCodeExecutable),
    );
  }
});

test("a hard `session bind` failure still runs the final flush and persists the open block", (t) => {
  // The exact regression introduced and caught in T1's fix round 1: the final flush used to sit
  // after an unguarded bind, so a failing bind skipped it and silently dropped the trailing block.
  const { status, stderr, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    nxcFailures: { "session bind": { status: 1, stderr: "error: database is locked\n" } },
  });

  assert.notEqual(status, 0, "a failed bind is fatal to the run");
  assert.match(stderr, /sidecar: nxc session bind failed/);
  assert.match(stderr, /database is locked/, "the underlying nxc stderr stays visible");
  assert.doesNotMatch(stderr, /sidecar done:/, "a failed run never claims success");

  assert.deepEqual(verbs(calls), [
    "transcript append",
    "session bind",
    "transcript append",
    "session ended",
  ]);
  const flushes = appends(calls);
  assert.deepEqual(
    flushes[1].entries.map((e) => e.data.text),
    ["trailing answer"],
    "the trailing open block is persisted even though the bind before it failed",
  );
});

test("a throwing stream flushes everything buffered plus the open block, and propagates", (t) => {
  // The run whose transcript is worth the most is the one that failed. The stream error is caught
  // (not left to unwind) so teardown still runs — but it must still ESCAPE, naming itself.
  const { status, stderr, calls } = runSidecar(t, {
    messages: [init, textDelta("hello "), textDelta("world")],
    throwAfter: 3,
  });

  assert.notEqual(status, 0, "a failed stream is a failed run");
  assert.match(stderr, /sidecar: the SDK stream failed/);
  assert.match(stderr, new RegExp(STREAM_ERROR), "the ORIGINAL cause is visible, not a wrapper");
  assert.doesNotMatch(stderr, /sidecar done:/);

  // No `result` arrived, so nothing flushed mid-run: bind first, then ONE teardown flush carrying
  // both the buffered entry and the block that was still open when the stream threw.
  assert.deepEqual(verbs(calls), ["session bind", "transcript append", "session ended"]);
  const flushes = appends(calls);
  assert.deepEqual(
    flushes[0].entries.map((e) => e.kind),
    ["session_init", "assistant"],
  );
  assert.equal(flushes[0].entries[1].data.text, "hello world");
});

test("an unrecognized-subcommand failure for a DIFFERENT name is named as a failure, not as a benign skip", (t) => {
  // main.mjs tolerates exactly one shape by NAME — clap exit 2 + "unrecognized subcommand
  // 'transcript'" — which existed only while `nxc transcript append` was unimplemented. That path
  // is dead now (T2 shipped the subcommand). Since 6j6v.jwgc no flush failure is fatal, so what the
  // name-match still buys is the DIAGNOSIS: a typo'd or renamed call site must be reported as a
  // failure ("the transcript flush failed"), never as the benign "this nxc predates the subcommand"
  // skip — the log line is all anyone has to tell a broken call site from an old binary.
  const { status, stderr, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    nxcFailures: {
      "transcript append": {
        status: 2,
        stderr: "error: unrecognized subcommand 'transcrpit'\n",
      },
    },
  });

  assert.equal(status, 0, "a broken transcript call site costs the transcript, not the turn");
  assert.match(stderr, /unrecognized subcommand 'transcrpit'/);
  assert.doesNotMatch(stderr, /skipped \(subcommand not implemented yet\)/);
  assert.match(stderr, /sidecar: the transcript flush failed/);
  assert.match(stderr, /sidecar done:.*transcript=incomplete/);
  // Teardown still runs after it, in the same load-bearing order — and there is no SECOND append,
  // because the latch is down for the rest of the run.
  assert.deepEqual(verbs(calls), ["transcript append", "session bind", "session ended"]);
});

test("a mid-run flush failure no longer kills the turn: the stream runs to its end", (t) => {
  // THE BUG (nxf 6j6v.jwgc). `flushTranscript()` is called from inside `for await`, so a throw used
  // to unwind out of the loop, abandon the SDK generator and end the turn — the caller got no reply
  // and waited out the workflow timeout. The realistic trigger is not exotic: `.nxs/db.sqlite` is
  // shared by flow, memory, chat AND sync, and a `nxs sync` pull applying a batch in one
  // transaction holds the write lock past `busy_timeout=5000`, which is exactly this stderr.
  const { status, stderr, consumed } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    nxcFailures: { "transcript append": { status: 1, stderr: "error: database is locked\n" } },
  });

  assert.equal(
    consumed,
    A_TURN_THEN_AN_OPEN_BLOCK.length,
    "every message after the failing flush is still consumed — the turn is not cut short",
  );
  assert.equal(status, 0, "a transcript gap is recoverable; a lost turn is not");
  assert.match(stderr, /sidecar done: role=tester session=s-1 real=real-1 transcript=incomplete/);
});

test("the first flush failure latches transcript capture off for the rest of the run", (t) => {
  // Without the latch, "record and continue" would re-attempt at EVERY later flush point, each one
  // burning up to the full 5s `busy_timeout` against the same stuck lock: a 100-flush turn would
  // add minutes of latency to a turn that has already lost its transcript either way.
  const { calls, stderr } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    nxcFailures: { "transcript append": { status: 1, stderr: "error: database is locked\n" } },
  });

  assert.deepEqual(
    verbs(calls),
    ["transcript append", "session bind", "session ended"],
    "exactly one append is ATTEMPTED: the mid-run flush that failed. The teardown flush — which " +
      "would otherwise stall on the same lock — spawns nothing at all",
  );
  assert.match(stderr, /transcript capture is now OFF for the rest of this run/);
  assert.match(stderr, /database is locked/, "the underlying nxc stderr stays visible");
});

test("an entry that cannot be serialized costs the transcript, not the turn", (t) => {
  // PR #332 review, Code Quality #2. Building the JSON-lines batch used to sit OUTSIDE
  // `flushTranscript`'s own try, so a throw from `JSON.stringify` escaped the function: mid-run it
  // reached the loop's catch and was labelled "the SDK stream failed" — pointing an on-call reader
  // at the SDK for a defect in this file — and at teardown nothing caught it at all. Nothing the
  // normalizer builds today can trigger it (every value on an entry comes from JSON the SDK
  // parsed), which is exactly why it needs pinning rather than reasoning: the guarantee should
  // survive the next change to `transcript.mjs`, and a serialization defect is a TRANSCRIPT
  // failure, so it belongs on the latch with every other one.
  const { status, stderr, calls, consumed } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    poisonAt: 1, // the `tool_use`, whose `input` the normalizer copies onto the entry verbatim
  });

  assert.equal(consumed, A_TURN_THEN_AN_OPEN_BLOCK.length, "the turn still runs to the end");
  assert.equal(status, 0, "a batch that cannot be serialized is still only telemetry");
  assert.match(stderr, /sidecar: the transcript flush failed/);
  assert.doesNotMatch(stderr, /the SDK stream failed/, "never blamed on the SDK");
  assert.match(stderr, /sidecar done:.*transcript=incomplete/);
  // It failed before `nxc` was ever reached, so the latch is down with nothing spawned at all.
  assert.deepEqual(verbs(calls), ["session bind", "session ended"]);
});

test("the latch holds across EVERY later flush point, not just the next one", (t) => {
  // The latch exists to stop repeated `busy_timeout` stalls, so one skipped flush proves little —
  // what matters is that a long turn with many flush points spawns `nxc` exactly once. The second
  // half of this test is what makes the first half mean anything: the same stream against a
  // WORKING `nxc` really does reach several flush points, so the single attempt above is the latch
  // and not an artifact of a stream too short to flush twice.
  const failing = runSidecar(t, {
    messages: MANY_TOOL_CALLS,
    nxcFailures: { "transcript append": { status: 1, stderr: "error: database is locked\n" } },
  });
  const working = runSidecar(t, { messages: MANY_TOOL_CALLS });

  assert.ok(
    appends(working.calls).length >= 3,
    `the scripted stream must reach at least 3 flush points for this test to mean anything, ` +
      `got ${appends(working.calls).length}`,
  );
  assert.equal(
    appends(failing.calls).length,
    1,
    "one attempt for the whole run, however many flush points it passes",
  );
  assert.equal(failing.consumed, MANY_TOOL_CALLS.length, "and the turn still finishes");
  assert.equal(failing.status, 0);
});

test("a failing FINAL flush is a warning too, not a failed run", (t) => {
  // The teardown flush used to `recordFatal`, so the LAST thing a wholly successful turn did could
  // still turn it into a failed run. Same judgement as the mid-run case, and the same latch: the
  // answer was already delivered from inside the session, so failing here would report a turn that
  // worked as a turn that did not — and the first consumer to act on that bit would re-run an
  // agent turn that is not idempotent.
  const { status, stderr, calls, consumed } = runSidecar(t, {
    // No `result` and well under FLUSH_THRESHOLD: the ONLY flush in this run is the teardown one.
    messages: [init, textDelta("hello "), textDelta("world")],
    nxcFailures: { "transcript append": { status: 1, stderr: "error: database is locked\n" } },
  });

  assert.equal(consumed, 3, "the stream ended on its own terms");
  assert.equal(status, 0, "the turn succeeded; only its transcript did not");
  assert.deepEqual(verbs(calls), ["session bind", "transcript append", "session ended"]);
  assert.match(stderr, /sidecar: the transcript flush failed/);
  assert.match(stderr, /sidecar done:.*transcript=incomplete/);
});

// ---- the reply-if-unanswered teardown step (nxf 6j6v.7e9d, ticket 8) ---------------------------
//
// Enforcement: tickets 5/6 made the obligation to answer a deterministic fact and told a primed
// role about it (`expects_reply_from`, the prime-block obligation); ticket 7 made `nxc reply
// --thread <id> --if-unanswered <text>` a safe, unconditional call — it posts only if the caller
// still owes an answer, and is a deliberate no-op otherwise. This step is what actually CALLS it
// from the one place that ALWAYS runs, whatever the session did: a crash, a forgetful model, a torn
// SDK stream all leave the thread with a visible failure instead of silence.

test("a trigger with a replyThread posts nxc reply --if-unanswered LAST, after bind and the final flush", (t) => {
  const { status, stderr, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
  });

  assert.equal(status, 0, `expected a clean exit, stderr:\n${stderr}`);
  // THE ORDER IS THE CONTRACT here too: the reply step comes after BOTH of the other two, never
  // between or before them — main.mjs's own "ORDER IS LOAD-BEARING" comment names why (the bind is
  // unrecoverable, the transcript is recoverable, and the failure reply is the least urgent of the
  // three, so it can never displace either).
  assert.deepEqual(verbs(calls), [
    "transcript append",
    "session bind",
    "transcript append",
    // The board read that decides whether a reminder is owed (nxf 6j6v.gh7f). The fake `nxc` writes
    // nothing to stdout here, so the read yields nothing parseable — the fail-safe direction, which
    // skips the reminder and leaves the rest of this sequence exactly as it was.
    "status --thread",
    "reply --thread",
    "session ended",
  ]);
  const [reply] = replyCalls(calls);
  assert.equal(reply.thread, "thread-77");
  assert.equal(reply.ifUnanswered, true, "the engine, not the sidecar, decides whether it posts");
  assert.match(reply.text, /ended without/i, "the text says what went wrong");
  // A session that ended CLEANLY without answering is not an escalation (nxf 6j6v.mqad): nothing
  // went wrong, the turn simply produced no reply, and marking it would tell a supervisor to stop.
  assert.equal(reply.escalate, false, "a quiet ending is not a hand-back");
});

test("a run that failed carries the collected failure into the reply text", (t) => {
  const { status, stderr, calls } = runSidecar(t, {
    messages: [init, textDelta("hello "), textDelta("world")],
    throwAfter: 3,
    spec: { replyThread: "thread-fail" },
  });

  assert.notEqual(status, 0, "the stream failure is still fatal to the run");
  assert.match(stderr, /sidecar: the SDK stream failed/);
  // The reply step runs REGARDLESS of `failures.hasFailed()` — a stream that died mid-answer is
  // exactly the case this ticket exists for, so the call happens even though the run goes on to
  // throw afterward.
  assert.deepEqual(verbs(calls), [
    "session bind",
    "transcript append",
    "status --thread",
    "reply --thread",
    "session ended",
  ]);
  const [reply] = replyCalls(calls);
  assert.equal(reply.thread, "thread-fail");
  assert.match(
    reply.text,
    new RegExp(STREAM_ERROR),
    "failures.hasFailed() is true, so the collected failure is what gets posted",
  );
  // nxf 6j6v.mqad — THE point of that item: a session that died at an error must reach the caller
  // as an ESCALATION and not as an ordinary reply. Without the flag the requester reads
  // `state: answered`, `awaiting_human: true`, `warnings: []`, and the only thing telling a crash
  // from a delivered answer is a message body that happens to start with `sidecar:`. The flag is
  // also what keeps the failure OUT of a `summarize` channel's fold, because an escalating set is
  // never folded.
  assert.equal(reply.escalate, true, "a session that DIED hands the task back, machine-readably");
});

test("an enormous SDK failure reaches the board CAPPED, not verbatim", (t) => {
  // PR #336 review, Integrity & Robustness #1. The reply text is built from an arbitrary SDK-thrown
  // `Error` and then persisted as a message body — durable, on a thread other sessions fold into
  // their own prompts — with no bound anywhere in the crate. `transcript.mjs` already caps exactly
  // this class of value (`MAX_THINKING_CHARS`, `MAX_TOOL_RESULT_CHARS`); the same discipline, the
  // same marker, applied before the text ever reaches `execFileSync`.
  const huge = `boom ${"x".repeat(120_000)}`;
  const { calls } = runSidecar(t, {
    messages: [init, textDelta("hello")],
    throwAfter: 2,
    errorText: huge,
    spec: { replyThread: "thread-huge" },
  });

  const [reply] = replyCalls(calls);
  assert.ok(reply, "the reply step still runs");
  assert.ok(
    reply.text.length <= MAX_FAILURE_REPLY_CHARS + 40,
    `the failure reply must be bounded, got ${reply.text.length} chars`,
  );
  assert.match(reply.text, /^sidecar: this session ended without answering — /, "still a diagnosis");
  assert.match(reply.text, /boom x+/, "the head of the cause survives, which is the useful part");
  assert.match(reply.text, /…\[truncated \d+ chars\]$/, "and it says what it dropped");
});

test("replyThread: null (a trigger with no obligation) skips the reply step entirely", (t) => {
  // The DoD's own words: a trigger with no thread must leave the step out — the common case (every
  // caller except the one `coordinator_commission` branch that actually declared the obligation, and the
  // ticket 6 KNOWN LIMIT: a trigger into a thread that already carries somebody else's quorum
  // neither registers nor tells the role, so `replyThread` is null there too and this is exactly
  // the right behavior).
  const { calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: null },
  });
  assert.deepEqual(verbs(calls), [
    "transcript append",
    "session bind",
    "transcript append",
    "session ended",
  ]);
});

test("a failing nxc reply --if-unanswered does not fail an otherwise-successful run, and does not touch bind or flush", (t) => {
  // DoD: "the new step cannot displace bind and transcript flush". Both already ran (in order)
  // BEFORE this call is even attempted, so there is nothing left for it to displace — and its own
  // failure must not retroactively turn a successful run into a failed one, because it ranks below
  // even the transcript flush's own recoverable, non-fatal treatment.
  const { status, stderr, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcFailures: { "reply --thread": { status: 1, stderr: "error: database is locked\n" } },
  });

  assert.equal(
    status,
    0,
    "the reply step is the LEAST urgent of the three — its own failure must not fail the run",
  );
  assert.match(stderr, /sidecar: nxc reply --if-unanswered failed/);
  assert.match(stderr, /database is locked/, "the underlying nxc stderr stays visible");
  assert.match(stderr, /sidecar done:/, "a failed reply post still lets a successful run report itself as one");
  assert.deepEqual(verbs(calls), [
    "transcript append",
    "session bind",
    "transcript append",
    "status --thread",
    "reply --thread",
    "session ended",
  ]);
});

test("an nxc that does not know --if-unanswered yet is tolerated as a benign skip, not a failure", (t) => {
  // The same tolerance `flushTranscript`/`bindRealSession` give an `nxc` that predates their own
  // subcommand, but for a FLAG: clap's shape for an unknown option is "unexpected argument", not
  // "unrecognized subcommand". A teardown that dies because the installed binary is older than
  // ticket 7 must not lose the bind or the transcript — nor, now, treat the skip as a failure.
  const { status, stderr } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcFailures: {
      "reply --thread": {
        status: 2,
        stderr: "error: unexpected argument '--if-unanswered' found\n",
      },
    },
  });

  assert.equal(status, 0);
  assert.match(stderr, /sidecar: nxc reply --if-unanswered skipped \(flag not implemented yet\)/);
  assert.doesNotMatch(stderr, /nxc reply --if-unanswered failed/);
});

test("every nxc call this sidecar makes is bounded in time", (t) => {
  // Review of PR #361, Integrity #2, kept as a SOURCE gate rather than as one assertion per call
  // site: the hazard is the PATH, not any one call, and the next call added to it would otherwise
  // inherit the omission silently. `execFileSync` without `timeout` waits forever, and a wedged
  // teardown holds the process whose EXIT a `working_tree: exclusive` channel is waiting for — so
  // an unbounded call here holds a checkout, not merely a process.
  //
  // Counting rather than parsing: five calls, five bounds. A new call with no bound moves one number
  // and not the other.
  const src = readFileSync(MAIN, "utf8");
  const calls = src.match(/execFileSync\(/g) ?? [];
  const bounds = src.match(/timeout: NXC_CALL_TIMEOUT_MS/g) ?? [];
  assert.ok(calls.length > 0, "the file still shells out at all");
  assert.equal(
    bounds.length,
    calls.length,
    `every execFileSync must carry \`timeout: NXC_CALL_TIMEOUT_MS\` — found ${calls.length} call(s) ` +
      `and ${bounds.length} bound(s)`,
  );
});

// ---- the reminder before the sidecar speaks for the agent (nxf 6j6v.gh7f) ---------------------
//
// The behaviour this replaces: a session that ended without answering had a failure posted IN ITS
// NAME — "this session ended without ever posting a reply" — which is not merely incomplete but
// MISLEADING, because a caller reading it cannot tell whether the work ran, is running, or failed.
// Measured over four hours in the proving ground 4jgn.g90w: 4 of 251 threads ended that way, and two
// of them stopped the chain until a human looked, once for over an hour, because the operation reads
// like a clean finish. The answer is to give the session its turn back and tell it, once.
//
// The board is scripted through the fake `nxc`'s stdout, because whether a debt is still open is the
// ENGINE's answer and the sidecar must not guess at it.

/** A `nxc status --thread <id> --json` payload: one operation, one thread, with `outstanding` set
 *  or empty. Everything else `StatusReport` carries is irrelevant to the two questions the teardown
 *  asks of it, and spelling more of it here would pin fields this behaviour does not read.
 *
 *  `waitingOn` is the ENGINE's own `waiting_on_sub_round` (nxf 6j6v.hw2t) — the rounds this session
 *  commissioned itself and has not got back. It is a field on the payload rather than something
 *  this fixture derives from its child rows, because that is exactly the change: the derivation
 *  moved into the engine, and the sidecar reads the answer. An older `nxc` omits the key, which is
 *  what the default here stands for. */
function board({ owes, waitingOn = [] }) {
  return JSON.stringify({
    operations: [
      {
        root: "thread-77",
        threads: [
          {
            thread_id: "thread-77",
            depth: 0,
            state: owes ? "open" : "answered",
            outstanding: owes ? ["local/tester"] : [],
            ...(waitingOn.length > 0 ? { waiting_on_sub_round: waitingOn } : {}),
          },
          ...waitingOn.map((id) => ({
            thread_id: id,
            parent: "thread-77",
            depth: 1,
            state: "open",
            outstanding: ["local/somebody"],
          })),
        ],
      },
    ],
  });
}

test("a session that owes its thread an answer is resumed and reminded before anything is posted for it", (t) => {
  const { status, stderr, calls, queries } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    // Open on the first read, discharged on the second: the reminder worked.
    nxcStdout: { "status --thread": [board({ owes: true }), board({ owes: false })] },
  });

  assert.equal(status, 0, `expected a clean exit, stderr:\n${stderr}`);
  assert.equal(queries.length, 2, "the session was given its turn back exactly once");
  const reminder = queries[1];
  assert.equal(
    reminder.options.resume,
    "real-1",
    "IN THIS PROCESS, on the same SDK session — a spawned resume would be refused by the " +
      "one-session-one-process guard (nxf 6j6v.7qtf), whose pid file names this very process",
  );
  assert.match(reminder.prompt, /thread-77/, "the reminder names the concrete thread");
  assert.match(reminder.prompt, /exactly two ways/, "and the whole set of ways to end the turn");
  assert.match(reminder.prompt, /--escalate/, "including the one a stuck session needs");
  // nxf 6j6v.s46h: this text says the same thing the session's own system prompt says
  // (`role.rs::reply_obligation`), and since that one names the STDIN form — the body passes no
  // shell quoting — a reminder still showing `"<your result>"` would hand the session the one form
  // the engine just stopped teaching, at the moment it is most likely to be obeyed literally.
  assert.match(
    reminder.prompt,
    /nxc reply --thread thread-77 - <<'EOF'/,
    "and it names the same safe form the system prompt does",
  );
  assert.doesNotMatch(
    reminder.prompt,
    /"<your result>"/,
    "the form that hands a verdict to the shell is gone from here too",
  );

  // The sidecar never spoke in the agent's name: the reply step ran, found the debt discharged, and
  // was the deliberate no-op it is designed to be — which is the engine's decision, not this file's.
  const [reply] = replyCalls(calls);
  assert.equal(reply.ifUnanswered, true);
  assert.equal(reply.escalate, false, "the session answered; nothing was handed back");
  assert.match(stderr, /sidecar done:.*reminded=answered/);
});

test("a session that has already answered is never resumed, and costs no model call", (t) => {
  // The read exists to make the reminder CONDITIONAL. Reminding unconditionally would buy one extra
  // paid turn on every run that ever ends, which is all of them.
  const { status, queries, stderr } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: false }) },
  });

  assert.equal(status, 0);
  assert.equal(queries.length, 1, "one turn, and no second one");
  assert.doesNotMatch(stderr, /reminded=/, "nothing to report about a reminder that was not owed");
});

test("a session WAITING on a round it commissioned itself is not reminded and nothing is posted for it", (t) => {
  // nxf 6j6v.hw2t — THE third case, and the whole point of the item. Measured on 2026-09-08: a role
  // that consulted four specialists ad hoc had no way to say so, so it spent `--escalate` on it and
  // the operation view read NEEDS DECISION over four working sessions. From the outside this looks
  // exactly like a session that forgot to answer; only the board can tell them apart, and it can.
  const { status, stderr, calls, queries } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: true, waitingOn: ["thread-review"] }) },
  });

  assert.equal(status, 0, `expected a clean exit, stderr:\n${stderr}`);
  assert.equal(
    queries.length,
    1,
    "no reminder round — a reminder asserts an omission, and waiting for work you commissioned " +
      "is what the engine's own obligation text now tells a session to do",
  );
  assert.deepEqual(
    replyCalls(calls),
    [],
    "AND NOTHING IS POSTED: the thread must keep owing its answer, because the session that owes " +
      "it is coming back to give one. A plain post would claim a result; an escalated one is the " +
      "false alarm this item removes",
  );
  assert.match(
    stderr,
    /waiting on a round it commissioned itself \(thread-review\)/,
    "and the teardown says why it stayed silent — an unexplained silence is indistinguishable " +
      "from the bug",
  );
  assert.match(stderr, /sidecar done:.*reminded=waiting_on_sub_round/);
});

test("the one reminder attempt is NOT spent on a waiting session", (t) => {
  // The half that is easy to lose: `reply_reminders` is ONE, so a reminder spent on a session that
  // was waiting legitimately is a reminder the session no longer has if it later really does forget.
  // Scripted so that the FIRST read is the waiting one and the second would show a plain debt — if
  // the attempt had been consumed, the loop would be over and the second read would never happen.
  const { queries, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77", replyReminders: 1 },
    nxcStdout: {
      "status --thread": [board({ owes: true, waitingOn: ["thread-review"] }), board({ owes: true })],
    },
  });

  assert.equal(queries.length, 1, "the turn, and nothing else");
  assert.equal(
    calls.filter((c) => c.argv.slice(0, 2).join(" ") === "status --thread").length,
    1,
    "the teardown left after the one read that answered it — it did not go round the loop again",
  );
});

test("a session that DIED while its own round runs still reports the failure", (t) => {
  // The carve-out, and it is deliberate: waiting is not a failure, but a torn stream is one whatever
  // else was in flight. `remindBeforeSpeakingForTheAgent` reaches the waiting branch only on a run
  // that got as far as reading its own board, and the post below is gated on `failures.hasFailed()`
  // rather than on the waiting state — so a crash still reaches the caller as an escalation.
  const { status, calls } = runSidecar(t, {
    messages: [init, textDelta("hello ")],
    throwAfter: 2,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: true, waitingOn: ["thread-review"] }) },
  });

  assert.notEqual(status, 0, "the stream failure is still fatal to the run");
  const [reply] = replyCalls(calls);
  assert.equal(reply.escalate, true, "a session that died hands the task back, waiting or not");
});

test("a reminded session that stays silent is handed back as an ESCALATION, not as an ordinary reply", (t) => {
  // DoD point 3: the fallback is today's behaviour, and the caller learns that the reminder failed
  // through a field rather than through prose. `--escalate` is the carrier that already exists — it
  // is what puts `escalated: true` on the caller's own `nxc status`, and what tells a supervisor
  // that no result to this round exists. A session that merely ended quietly is still NOT escalated;
  // one that was told and ended again is a different fact.
  const { status, stderr, calls, queries } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  assert.equal(status, 0, "a silent session is not a failed run");
  assert.equal(queries.length, 2, "reminded once — and only once, so this cannot loop");
  const [reply] = replyCalls(calls);
  assert.equal(reply.escalate, true);
  assert.match(reply.text, /was reminded/, "and says which of the two endings this was");
  assert.match(stderr, /sidecar done:.*reminded=unanswered/);
});

test("an unreadable board skips the reminder entirely and falls back to what happened before", (t) => {
  // The fail-safe direction, and the reason the read returns `null` on ANY failure — an older `nxc`,
  // a locked db, output that does not parse. Spending a paid model call on a guess about whether a
  // debt exists is the worse trade in both directions.
  const { status, stderr, calls, queries } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": "not json at all" },
  });

  assert.equal(status, 0);
  assert.equal(queries.length, 1, "no reminder was attempted");
  assert.match(stderr, /could not read the reply debt for thread-77/);
  const [reply] = replyCalls(calls);
  assert.equal(reply.ifUnanswered, true, "the pre-gh7f behaviour, unchanged");
  assert.equal(reply.escalate, false, "and NOT an escalation — nobody was reminded");
});

test("a trigger with no obligation is never resumed and never reads a board", (t) => {
  const { calls, queries } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: null },
  });

  assert.equal(queries.length, 1);
  assert.deepEqual(
    verbs(calls).filter((v) => v === "status --thread"),
    [],
    "there is no debt to ask about",
  );
});

test("the reminder round is bounded by a timer of its own", (t) => {
  // Review of PR #361, Integrity #1. The reminder runs with the SAME tools and permissions as the
  // original turn — deliberately, since what it asks for is `nxc reply` and that needs Bash — so a
  // reminded session can still act. What must not be open-ended is HOW LONG it may do so: a session
  // stalled in the SDK's own retry loop stays alive and tool-enabled while its member deadline can
  // strike underneath it, and a lapsed member is one the engine advances past. That is the overlap
  // nxf 6j6v.10yb closes, re-opened from the other end.
  //
  // What is asserted is the WIRING — an `AbortController` reaches the reminder's `query()` — and
  // that the timer on it is a real bound. The firing itself is the SDK's to honour
  // (`Options.abortController`), and a test that waited out the real bound would be a five-minute
  // test of a `setTimeout`.
  //
  // This used to claim "and the main turn is not" bounded, and verified it by the ABSENCE of an
  // abort controller on the main turn. Since nxf 6j6v.b9nf the main turn carries one too — the
  // stop door, which nothing fires on a clock — so that absence is no longer the fact, and a
  // serialized `AbortController` is `{}` either way: the SIGTERM case below asserts only that the
  // key is PRESENT, which was never the fact that mattered (fix round 3 of that item's review,
  // Test Quality #8). Absence of a TIMER on the main turn's controller is what
  // `no_clock_fires_the_main_turns_abort_controller` below pins instead, at the source.
  const { queries } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": [board({ owes: true }), board({ owes: false })] },
  });

  assert.equal(queries.length, 2);
  assert.ok(
    "abortController" in queries[1].options,
    `the reminder round is handed an abort controller, got: ${JSON.stringify(queries[1].options)}`,
  );
  assert.ok(
    Number.isFinite(REPLY_REMINDER_BOUND_MS) && REPLY_REMINDER_BOUND_MS > 0,
    "and the bound is a real one",
  );
  assert.ok(
    REPLY_REMINDER_BOUND_MS < 2 * 60 * 60 * 1000,
    "…well inside the two-hour working-tree bound it must not outlast",
  );
});

test("no clock fires the MAIN turn's abort controller (nxf 6j6v.jwgc)", () => {
  // **What this restores.** Before nxf 6j6v.b9nf the main turn was handed NO abort controller at
  // all, so `!("abortController" in queries[0].options)` was a complete proof that nothing could
  // ever fire one on a clock — there was nothing to fire. Since 6j6v.b9nf the main turn legitimately
  // carries one (the withdrawal's own stop door), so that presence check flipped to asserting the
  // OPPOSITE fact — the key is there now, on purpose — and nothing after it re-proved the fact that
  // actually mattered for 6j6v.jwgc: no clock may ever fire the MAIN turn's controller. Presence was
  // never that fact; absence of a TIMER is.
  //
  // **Why the query log cannot answer this any more.** `runSidecar`'s `queries` come from
  // `JSON.stringify`-ing each `query()` call's `arg` to a log file and reading it back — and
  // `JSON.stringify(new AbortController())` is `{}` whether or not anything is bound to fire it.
  // Once the key exists, the log has already thrown away the one fact this test is about: WHICH
  // controller it is and what, if anything, schedules a call to `.abort()` on it. A real-time wait
  // (hang the main turn, wait past a bound, assert it is still alive) is the technique the sibling
  // test above deliberately avoids for `REPLY_REMINDER_BOUND_MS` — this file's own stated reason is
  // that waiting out a real bound is a five-minute test of a `setTimeout`, and the main turn's
  // wrongly-fired defect would need the same five minutes to prove by clock.
  //
  // **So this reads the one place the fact still lives: the source.** `remindOnce` in `main.mjs` is
  // the ONLY function in the file that ever calls `setTimeout(() => <x>.abort(), …)`, and `<x>` is
  // its own function-scoped `abortController` — never `stop`, the module-level controller the main
  // turn's `query()` call is handed (`options: { ...options, abortController: stop }`, read
  // separately below). A regression that bound a timer to `stop` — the literal shape of 6j6v.jwgc,
  // restored — changes one of these two facts and turns this red.
  const src = readFileSync(new URL("../src/main.mjs", import.meta.url), "utf8");

  const timeoutAborts = [...src.matchAll(/setTimeout\(\s*\(\)\s*=>\s*([A-Za-z_$][\w$]*)\.abort\(\)/g)].map(
    (m) => m[1],
  );
  assert.deepEqual(
    timeoutAborts,
    ["abortController"],
    `exactly one clock in this file may ever call .abort(), and it must be the reminder's own \
function-scoped controller, not the module-level "stop" the main turn is handed: found \
${JSON.stringify(timeoutAborts)}`,
  );

  assert.match(
    src,
    /options:\s*\{\s*\.\.\.options,\s*abortController:\s*stop\s*\}/,
    "the main turn is handed `stop` directly — not a fresh controller a timer could be bound to " +
      "without this regex ever seeing it",
  );
});

test("a reminder whose own stream fails is a warning, and the fallback still runs", (t) => {
  // The reminder is the least urgent thing this teardown does: its failure costs the reminder, not
  // the run, and what is left is exactly the behaviour that existed before this ticket.
  const { status, stderr, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: true }) },
    reminderThrows: true,
  });

  assert.equal(status, 0, "a failed reminder must not fail a turn that worked");
  assert.match(stderr, /sidecar: the reminder resume failed/);
  const [reply] = replyCalls(calls);
  assert.equal(reply.ifUnanswered, true, "the fallback still speaks");
  assert.match(stderr, /sidecar done:.*reminded=failed/);
});

// ---- the session-end announcement (nxf 6j6v.10yb) ---------------------------------------------
//
// The fourth teardown step, and the one fact only the ending session can report: a channel that
// declares `working_tree: exclusive` opens its next step once the previous one has answered AND its
// session is over. From outside, at the moment it matters, the process is still alive — it is the
// one running this teardown — so nothing but the session itself can say it.

test("a spec with no internal session announces nothing, because there is no session to announce", (t) => {
  // A hand-written bare spec — every real run gets an internal session from `SidecarWorker::
  // trigger`. There is no row to mark and nothing that could be waiting on it, so the step is left
  // out entirely rather than called with an empty id.
  const { status, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { session: null },
  });

  assert.equal(status, 0);
  assert.deepEqual(
    verbs(calls).filter((v) => v === "session ended"),
    [],
    "nothing to announce",
  );
});

test("an nxc that does not know `session ended` yet is tolerated as a benign skip, not a failure", (t) => {
  // The tolerance is matched on `ended`, NOT on `session`: an older `nxc` HAS the `session` group
  // (it has carried `bind` since Task 2) and rejects only the new leaf, so clap's wording names
  // `ended`. Keying on the group would never match and a merely-old binary would be reported as a
  // real failure.
  const { status, stderr } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    nxcFailures: {
      "session ended": {
        status: 2,
        stderr: "error: unrecognized subcommand 'ended'\n",
      },
    },
  });

  assert.equal(status, 0);
  assert.match(stderr, /sidecar: nxc session ended skipped \(subcommand not implemented yet\)/);
  assert.doesNotMatch(stderr, /nxc session ended failed/);
});

test("a failing session-end announcement is a warning, and never fails an otherwise good run", (t) => {
  // It ranks with the failure reply rather than with the bind: what its failure costs is a DELAY —
  // the flow falls back to process liveness and then to the channel's declared `timeout:` — never a
  // wrong answer. Turning a delivered turn into a failed one over it would be the worse trade, and
  // by the time it runs there is nothing left for it to displace.
  const { status, stderr, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    nxcFailures: { "session ended": { status: 1, stderr: "error: database is locked\n" } },
  });

  assert.equal(status, 0, "the turn succeeded; only the announcement did not");
  assert.match(stderr, /sidecar: nxc session ended failed/);
  assert.match(stderr, /database is locked/, "the underlying nxc stderr stays visible");
  assert.match(stderr, /sidecar done:/, "a failed announcement still lets a good run report itself as one");
  assert.deepEqual(verbs(calls), [
    "transcript append",
    "session bind",
    "transcript append",
    "session ended",
  ]);
});

test("an unexpected-argument failure for a DIFFERENT flag is named as a failure, not a benign skip", (t) => {
  // Matched by NAME, same discipline as the subcommand tolerance: tolerating ANY "unexpected
  // argument" would silently swallow a typo'd or renamed call site as if it were an old binary.
  const { status, stderr } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77" },
    nxcFailures: {
      "reply --thread": {
        status: 2,
        stderr: "error: unexpected argument '--stream' found\n",
      },
    },
  });

  assert.equal(status, 0, "still a warning, not a failed run — see the ranking above");
  assert.doesNotMatch(stderr, /skipped \(flag not implemented yet\)/);
  assert.match(stderr, /sidecar: nxc reply --if-unanswered failed/);
  assert.match(stderr, /unexpected argument '--stream'/, "the underlying nxc stderr stays visible");
});

// ---- "did not answer" against "never got to think" (nxf 6j6v.ntp9, nxf 6j6v.553s question (b)) --
//
// Two ways a turn can fail to answer its thread that look alike from outside the process and are not
// alike at all. Only the runtime can tell them apart, and until this item it told nobody:
//
//   the runtime never produced a message  -> the agent never ran; retry, with backoff
//   the stream tore after it had          -> the turn did work; remind it, never replay it
//   the turn ended, still owing           -> the gh7f reminder, unchanged
//
// The BOUNDS on the first two are the coordinator's and travel on the spec, so a host that installs
// its own worker inherits them instead of re-deciding them.

test("a runtime that never ran the agent is started again, and a recovered run is a clean one", (t) => {
  const { status, stderr, queries, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    // Throws before yielding anything: the shape a `529` at the door takes.
    throwAfter: 0,
    spec: { replyThread: "thread-77", runtimeRetries: 1, retryBackoffMs: 1 },
    nxcStdout: { "status --thread": board({ owes: false }) },
  });

  assert.equal(queries.length, 2, "the turn was started again, once");
  assert.equal(
    queries[1].prompt,
    "hi",
    "and it is the SAME turn, from the top — a retry is not a reminder, and there is nothing to " +
      "resume from a session that never produced a frame",
  );
  assert.equal(status, 0, `the second attempt carried the turn, stderr:\n${stderr}`);
  assert.match(stderr, /the runtime never ran this session/);
  assert.match(stderr, /sidecar done:.*retried=1/, "a run that had to be retried says so");
  const [reply] = replyCalls(calls);
  assert.equal(reply.escalate, false, "nothing died in the end, so nothing is handed back");
});

test("the retries are bounded, and past the bound the round is escalated as a runtime that never ran", (t) => {
  // Without a bound this is an outage waited out forever while the working copy stays claimed. The
  // ceiling is the point rather than the number; what matters more is what follows it, which is a
  // human being told — and told WHICH of the two failures it was, because "ended without answering"
  // reads the same for a 529 at the door and for a turn that ran and crashed, and those two call for
  // opposite things from whoever reads them.
  const { status, stderr, queries, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    throwAfter: 0,
    throwEveryRound: true,
    spec: { replyThread: "thread-77", runtimeRetries: 2, retryBackoffMs: 1 },
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  assert.equal(queries.length, 3, "one attempt plus the two the terms allow, and no fourth");
  assert.notEqual(status, 0, "the run failed in the end and says so");
  const [reply] = replyCalls(calls);
  assert.equal(reply.escalate, true, "somebody has to decide what happens to this round");
  assert.match(
    reply.text,
    /the runtime never ran this session — 3 attempts/,
    `the escalation names which bound was reached, got: ${reply.text}`,
  );
  assert.match(reply.text, new RegExp(STREAM_ERROR), "without swallowing the cause");
});

test("a resumed session whose runtime never ran it is NOT reminded — that would spend the one attempt on a session that broke no rule", (t) => {
  // The exact case nxf 6j6v.553s asks about. A resumed spec carries a session id BEFORE the first
  // frame arrives, so the reminder's own "is there a session to resume" guard did not catch this: it
  // fired, resumed straight back into the same outage, and burned the attempt before the escalation
  // went out. The condition is "did a message ever arrive", not "is there a session id".
  const { queries, stderr, calls } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    throwAfter: 0,
    spec: { replyThread: "thread-77", resume: "real-9", runtimeRetries: 0 },
    // The board says the debt is real — so the ONLY thing keeping the reminder away is the
    // distinction this test is about.
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  assert.equal(queries.length, 1, "no second round: not a retry (none allowed) and not a reminder");
  assert.match(stderr, /not reminding session .* the runtime never ran it/, "and it says why");
  const [reply] = replyCalls(calls);
  assert.equal(reply.escalate, true);
  assert.match(reply.text, /the runtime never ran this session — 1 attempt\b/);
});

test("a stream that tears MID-turn is reminded and never replayed, because that turn already did work", (t) => {
  // The other side of the line, and the reason it is drawn at "did a message arrive" rather than at
  // "did the stream fail": an agent turn is not idempotent. Replaying one that already ran would
  // redo its files, its messages and its pull requests. What such a session needs is to be asked for
  // its result — which is exactly what the reminder is.
  const { queries, stderr } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    throwAfter: 2,
    spec: { replyThread: "thread-77", runtimeRetries: 2, retryBackoffMs: 1 },
    nxcStdout: { "status --thread": [board({ owes: true }), board({ owes: false })] },
  });

  assert.equal(queries.length, 2, "one main turn and one reminder — not a retry");
  assert.doesNotMatch(stderr, /retrying in/, "a turn that ran is never started again");
  assert.match(queries[1].prompt, /thread-77/, "round two is the reminder, not the task");
  assert.match(queries[1].prompt, /exactly two ways/, "the reminder in full, not a retry of 'hi'");
  assert.doesNotMatch(
    stderr,
    /the runtime never ran it/,
    "and it is NOT excused as a runtime failure: this session ran, and owes an answer for it",
  );
});

test("the reminder bound is the coordinator's: a spec that allows none costs no model call", (t) => {
  // `TERMS` replaced a constant in this file. The value of moving it is only real if the spec can
  // actually change the behaviour, which is what this pins — and `0` is the value a host would set
  // to turn the reminder off for a runtime where resuming means something else.
  const { queries, status } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77", replyReminders: 0 },
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  assert.equal(status, 0);
  assert.equal(queries.length, 1, "the board says it owes, and no reminder was spent on it");
});

test("a spec from an older engine carries no terms, and reads as the behaviour that existed then", (t) => {
  // Forward compatibility in the direction that actually happens: an `nxc` upgraded past a sidecar,
  // or a sidecar past an `nxc`. Silence must mean "one reminder, no retry" — the pre-6j6v.ntp9
  // behaviour — never "unbounded".
  const { queries, status } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    throwAfter: 0,
    throwEveryRound: true,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  assert.equal(queries.length, 1, "no terms on the spec means no retry, exactly as before");
  assert.notEqual(status, 0);
});

// ---- the shape a real 529 actually takes (review of PR #378, Integrity #1) ---------------------
//
// The first cut of the retry above was tested only against a stream that THREW before yielding
// anything — a shape the real SDK produces when `claude` cannot be started at all. A retryable API
// failure is not that: per the pinned `sdk.d.ts` the SDK retries internally, reports each attempt as
// `system/api_retry`, and when it gives up ends the stream NORMALLY with a `result` whose `is_error`
// is true. Two consequences, both of which made the feature inert for the case it exists for:
//
//   * `for await` ends without throwing, so a `catch`-only loop never decided anything;
//   * `system/init` arrives first, so a flag set from "any message" was already true.
//
// These cases are written against the SDK's own declared shapes so that they fail if either
// assumption is ever reintroduced.

test("a 529 at the door is retried — the SDK ends the stream normally, and init is not the agent thinking", (t) => {
  const { status, stderr, queries, calls } = runSidecar(t, {
    messages: A_529_AT_THE_DOOR,
    // Round two gets through. Without the fix there is no round two at all.
    laterRoundMessages: A_TURN_THEN_AN_OPEN_BLOCK,
    spec: { replyThread: "thread-77", runtimeRetries: 1, retryBackoffMs: 1 },
    nxcStdout: { "status --thread": board({ owes: false }) },
  });

  assert.equal(
    queries.length,
    2,
    `the turn was started again. This is THE regression test for the first cut, which saw neither ` +
      `half of this stream: no throw to catch, and 'init' had already set the thinking flag. ` +
      `stderr:\n${stderr}`,
  );
  assert.equal(queries[1].prompt, "hi", "and it is the same task, from the top");
  assert.match(stderr, /the runtime never ran this session/);
  assert.match(stderr, /api status 529/, "the status that names the outage survives into the log");
  assert.equal(status, 0, `the second attempt carried the turn, stderr:\n${stderr}`);
  const [reply] = replyCalls(calls);
  assert.equal(reply.escalate, false, "nothing died in the end");
});

test("a 529 that outlasts the bound is a FAILED run, not a session that quietly said nothing", (t) => {
  // The other half of the same defect. A stream that ends with `is_error` used to leave
  // `failures.hasFailed()` false — so the process exited 0, and the requester got the
  // non-escalating "this session ended without ever posting a reply" for a session that died at the
  // door. Nobody was asked to decide the round; that is the guarantee this ticket adds.
  const { status, stderr, queries, calls } = runSidecar(t, {
    messages: A_529_AT_THE_DOOR,
    spec: { replyThread: "thread-77", runtimeRetries: 0 },
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  assert.equal(queries.length, 1, "no retry was allowed, and no reminder was spent either");
  assert.notEqual(status, 0, `a run that never ran is a failed run, stderr:\n${stderr}`);
  const [reply] = replyCalls(calls);
  assert.equal(reply.escalate, true, "somebody has to decide what happens to this round");
  assert.match(reply.text, /the runtime never ran this session — 1 attempt\b/);
  assert.match(reply.text, /api status 529/, "naming the outage, not just 'it ended'");
});

test("model output followed by an error is a mid-turn tear: reminded, never replayed", (t) => {
  // `toolUse`/`toolResult` are in this stream, so something may already have happened on disk.
  // Replaying the turn would redo it. The flag has to be set by MODEL OUTPUT for this to hold —
  // under the first cut it was set by `init` and this case was indistinguishable from the one above.
  const { queries, stderr } = runSidecar(t, {
    messages: MODEL_OUTPUT_THEN_AN_ERROR,
    spec: { replyThread: "thread-77", runtimeRetries: 3, retryBackoffMs: 1 },
    nxcStdout: { "status --thread": [board({ owes: true }), board({ owes: false })] },
  });

  assert.equal(queries.length, 2, "one task round and one reminder — never a second task round");
  assert.doesNotMatch(stderr, /retrying in/, "a turn that produced output is never started again");
  assert.match(queries[1].prompt, /exactly two ways/, "round two is the reminder");
});

test("the thinking flag is cumulative: an attempt that gets further is never replayed a third time", (t) => {
  // The safety argument of the whole loop, and it needs a per-round script to state at all: attempt
  // one produces nothing and fails, attempt two reaches the model and then tears. Three attempts are
  // allowed; only two may be taken.
  const { queries, stderr } = runSidecar(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    // Round 0: throw before yielding anything. Round 1: yield `init` + the assistant `tool_use`,
    // then throw. Round 2 (if it ever ran) would replay the whole turn.
    throwAfter: [0, 2],
    spec: { replyThread: "thread-77", runtimeRetries: 3, retryBackoffMs: 1 },
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  const taskRounds = queries.filter((q) => q.prompt === "hi").length;
  assert.equal(
    taskRounds,
    2,
    `exactly one retry: the first attempt produced nothing, the second produced model output and ` +
      `may have had side effects. stderr:\n${stderr}`,
  );
  assert.match(stderr, /retrying in 1ms \(attempt 2 of 4\)/, "the first retry, and only it");
  assert.doesNotMatch(stderr, /attempt 3 of 4/, "the second attempt thought, so there is no third");
});

test("an empty but CLEAN stream is still reminded — nothing says the runtime failed there", (t) => {
  // The narrow case that separates `runtimeFailed && !agentEverThought` from `!agentEverThought`
  // alone (review of PR #378, Code Quality #2 / Test Quality #5). A resumed spec carries a session
  // id before the first frame, so this is reachable — and excusing it as "never ran" would silently
  // drop nxf 6j6v.gh7f for this shape and leave the round unescalated.
  const { queries, status, stderr } = runSidecar(t, {
    messages: [],
    spec: { replyThread: "thread-77", resume: "real-9", runtimeRetries: 2, retryBackoffMs: 1 },
    nxcStdout: { "status --thread": [board({ owes: true }), board({ owes: false })] },
  });

  assert.equal(status, 0, `nothing failed, so nothing is reported as failed, stderr:\n${stderr}`);
  assert.equal(queries.length, 2, "the reminder ran — it is owed, and no retry is warranted");
  assert.doesNotMatch(stderr, /the runtime never ran it/, "there is no runtime failure to excuse");
  assert.doesNotMatch(stderr, /retrying in/, "and nothing to retry: the attempt did not fail");
  assert.match(stderr, /sidecar done:.*reminded=answered/);
});

// ---- the availability boundary (nxf 6j6v.npy3) ---------------------------------------------------

/** The `nxc session interrupted <id> …` call, decoded from its argv. */
function interruptedCalls(calls) {
  return calls
    .filter((c) => c.argv[0] === "session" && c.argv[1] === "interrupted")
    .map((c) => ({
      session: c.argv[2],
      until: c.argv.includes("--until") ? c.argv[c.argv.indexOf("--until") + 1] : null,
      limit: c.argv.includes("--limit") ? c.argv[c.argv.indexOf("--limit") + 1] : null,
      detail: c.argv.includes("--detail") ? c.argv[c.argv.indexOf("--detail") + 1] : null,
    }));
}

/** A quota rejection: the runtime states the class on the assistant message and the INSTANT in its
 *  own typed event, and the turn then ends as an error. Modelled on the measured case in nxf
 *  6j6v.npy3 — the `__synth__` session that ran into the weekly window 0.6 s after its `nxc reply`. */
const rateLimitEvent = {
  type: "rate_limit_event",
  session_id: "real-1",
  rate_limit_info: { status: "rejected", resetsAt: 1789000000, rateLimitType: "seven_day" },
};

const quotaError = {
  type: "assistant",
  session_id: "real-1",
  parent_tool_use_id: null,
  error: "rate_limit",
  message: { content: [] },
};

const quotaResult = {
  type: "result",
  subtype: "error_during_execution",
  session_id: "real-1",
  is_error: true,
  num_turns: 2,
  errors: ["You've hit your weekly limit · resets 2pm (Europe/Berlin)"],
};

/** The measured shape: the turn DID work (a tool call and its result), and then the window closed.
 *  A mid-turn tear, so nothing here may be replayed. */
const WORK_THEN_A_QUOTA_WALL = [init, toolUse, toolResult, rateLimitEvent, quotaError, quotaResult];

test("an availability boundary is announced, not escalated (nxf 6j6v.npy3)", async (t) => {
  await t.test("`nxc session interrupted` carries the class and the runtime's own instant", (t) => {
    const { calls, stderr } = runSidecar(t, {
      messages: WORK_THEN_A_QUOTA_WALL,
      spec: { replyThread: "m-thread-1" },
      nxcStdout: { "status --thread": JSON.stringify({ operations: [{ threads: [{ thread_id: "m-thread-1", outstanding: ["tester"] }] }] }) },
    });
    assert.deepEqual(interruptedCalls(calls), [
      {
        session: "s-1",
        until: "2026-09-10T00:26:40.000Z",
        limit: "seven_day",
        detail: "You've hit your weekly limit · resets 2pm (Europe/Berlin)",
      },
    ], `stderr:\n${stderr}`);
  });

  // THE POINT OF THE WHOLE ITEM. The substitute post says "I cannot carry this out" in the agent's
  // name, which is false — nothing is broken but the availability of the model — and it settles the
  // thread's debt, so the round reads finished and the operation stops on NEEDS DECISION.
  await t.test("NO substitute escalation is posted in the session's name", (t) => {
    const { calls } = runSidecar(t, {
      messages: WORK_THEN_A_QUOTA_WALL,
      spec: { replyThread: "m-thread-1" },
      nxcStdout: { "status --thread": JSON.stringify({ operations: [{ threads: [{ thread_id: "m-thread-1", outstanding: ["tester"] }] }] }) },
    });
    assert.deepEqual(replyCalls(calls), [], "the thread must keep owing its answer");
  });

  // A reminder asks a session to obey a rule it broke. This one broke no rule, and resuming it
  // would spend a paid model call going straight back into the same closed window.
  await t.test("the session is NOT reminded into the same closed window", (t) => {
    const { calls, stderr } = runSidecar(t, {
      messages: WORK_THEN_A_QUOTA_WALL,
      spec: { replyThread: "m-thread-1" },
      nxcStdout: { "status --thread": JSON.stringify({ operations: [{ threads: [{ thread_id: "m-thread-1", outstanding: ["tester"] }] }] }) },
    });
    assert.match(stderr, /not reminding session s-1 .* availability boundary \(seven_day\)/);
    assert.equal(verbs(calls).filter((v) => v === "status --thread").length, 0,
      "it should not even read the board: there is no reminder to decide about");
  });

  // The process IS over, whatever ended it — the liveness gate and the delivery queue both rest on
  // this announcement, and an interrupted session must not silently stop making it.
  await t.test("`nxc session ended` still runs, and AFTER the interruption is recorded", (t) => {
    const { calls } = runSidecar(t, { messages: WORK_THEN_A_QUOTA_WALL });
    const order = verbs(calls);
    assert.ok(order.includes("session ended"), `verbs: ${order.join(", ")}`);
    assert.ok(
      order.indexOf("session interrupted") < order.indexOf("session ended"),
      `the hold must be recorded before the end is announced; verbs: ${order.join(", ")}`,
    );
  });

  await t.test("one greppable line names the boundary, the window and the reason", (t) => {
    const { stderr } = runSidecar(t, { messages: WORK_THEN_A_QUOTA_WALL });
    assert.match(
      stderr,
      /session s-1 is ON HOLD at an availability boundary \(seven_day\), until 2026-09-10T00:26:40\.000Z: You've hit your weekly limit/,
    );
  });

  await t.test("with no instant, the line says so rather than going quiet about it", (t) => {
    const { stderr } = runSidecar(t, { messages: [init, toolUse, toolResult, quotaError, quotaResult] });
    assert.match(stderr, /ON HOLD at an availability boundary \(rate_limit\), with no reset instant stated/);
  });

  // **The guard against putting a session that ANSWERED on hold.** The SDK retries a retryable API
  // failure itself, so a stream can carry the rejection and then finish the turn — and arming a
  // return for work that is already done is worse than not arming one at all.
  await t.test("a run that saw a rejection and then SUCCEEDED is not on hold", (t) => {
    const { status, calls, stderr } = runSidecar(t, {
      messages: [init, rateLimitEvent, quotaError, toolUse, toolResult, result],
    });
    assert.equal(status, 0, `expected a clean exit, stderr:\n${stderr}`);
    assert.deepEqual(interruptedCalls(calls), []);
    assert.doesNotMatch(stderr, /ON HOLD/);
  });

  // Without an instant there is no automatic way back, so the flag is ABSENT rather than carrying
  // a moment nobody stated.
  await t.test("a boundary with no reset instant passes no --until", (t) => {
    const { calls } = runSidecar(t, {
      messages: [init, toolUse, toolResult, quotaError, quotaResult],
    });
    assert.deepEqual(interruptedCalls(calls).map((c) => c.until), [null]);
  });

  // The neighbouring classes must be untouched: a crash is still a crash.
  await t.test("an ordinary runtime failure announces NO interruption and still escalates", (t) => {
    const { calls } = runSidecar(t, {
      messages: MODEL_OUTPUT_THEN_AN_ERROR,
      spec: { replyThread: "m-thread-1" },
      nxcStdout: { "status --thread": JSON.stringify({ operations: [{ threads: [{ thread_id: "m-thread-1", outstanding: [] }] }] }) },
    });
    assert.deepEqual(interruptedCalls(calls), []);
    assert.deepEqual(replyCalls(calls).map((r) => r.escalate), [true]);
  });

  // Forward compatibility, the shape `session bind` and `reply --if-unanswered` already have: an
  // older `nxc` does not have this leaf, and a teardown must not turn that into a failed run.
  await t.test("an nxc without the subcommand is skipped, not fatal", (t) => {
    const { status, stderr } = runSidecar(t, {
      messages: WORK_THEN_A_QUOTA_WALL,
      nxcFailures: {
        "session interrupted": { status: 2, stderr: "error: unrecognized subcommand 'interrupted'" },
      },
    });
    assert.match(stderr, /session interrupted skipped \(subcommand not implemented yet\)/);
    assert.notEqual(status, null);
  });
});

// A first attempt that hits the quota wall at the DOOR — no assistant message at all, so the model
// produced nothing and the runtime retry is allowed to fire.
const A_QUOTA_WALL_AT_THE_DOOR = [init, rateLimitEvent, quotaResult];

/** A second attempt that gets further and then dies of something else entirely. */
const A_DIFFERENT_FAILURE_ON_THE_RETRY = [init, toolUse, toolResult, overloadedResult];

test("the boundary is a fact about the attempt that ENDED the run (nxf 6j6v.npy3)", async (t) => {
  // **The stale-signal bug** (independent review of PR #476, Code Quality #3 / Integrity #3).
  // `lastAssistantError`, `lastResultText` and the normalizer's rate-limit latch lived across the
  // whole retry loop. An abandoned first attempt that saw a rejection would therefore still be
  // speaking when a LATER, unrelated failure ended the run — so a genuinely broken run would be
  // recorded as a harmless "on hold", the human escalation it needs would be suppressed, and a
  // return would be armed for a boundary that was not the reason this run ended.
  await t.test("a stale rejection from an abandoned attempt does not classify a later failure", (t) => {
    const { calls, stderr } = runSidecar(t, {
      messages: A_QUOTA_WALL_AT_THE_DOOR,
      laterRoundMessages: A_DIFFERENT_FAILURE_ON_THE_RETRY,
      spec: { replyThread: "m-thread-1", runtimeRetries: 1, retryBackoffMs: 1 },
      nxcStdout: {
        "status --thread": JSON.stringify({
          operations: [{ threads: [{ thread_id: "m-thread-1", outstanding: ["tester"] }] }],
        }),
      },
    });
    assert.match(stderr, /retrying in/, "the first attempt must actually have been retried");
    assert.deepEqual(
      interruptedCalls(calls),
      [],
      "the run did not end at an availability boundary, so nothing may be put on hold",
    );
    assert.deepEqual(
      replyCalls(calls).map((r) => r.escalate),
      [true],
      "…and the escalation a broken run owes its caller still goes out",
    );
  });

  // The other direction, so the reset cannot be "fixed" by simply never latching anything.
  await t.test("a rejection on the attempt that DID end the run still counts", (t) => {
    const { calls } = runSidecar(t, {
      messages: A_QUOTA_WALL_AT_THE_DOOR,
      laterRoundMessages: WORK_THEN_A_QUOTA_WALL,
      spec: { replyThread: "m-thread-1", runtimeRetries: 1, retryBackoffMs: 1 },
      nxcStdout: {
        "status --thread": JSON.stringify({
          operations: [{ threads: [{ thread_id: "m-thread-1", outstanding: ["tester"] }] }],
        }),
      },
    });
    assert.deepEqual(interruptedCalls(calls).map((c) => c.limit), ["seven_day"]);
    assert.deepEqual(replyCalls(calls), [], "and no escalation is posted in the agent's name");
  });
});

// ---- a session that is told to stop (nxf 6j6v.b9nf) ---------------------------------------------
//
// `SidecarWorker::stop_session` sends this process SIGTERM — not SIGKILL, so that what follows can
// run. Without a handler the default disposition applies and the process simply dies: no bind, no
// final flush, no `nxc session ended`, and the engine's liveness gate has to notice the corpse on
// its own clock. With it, the running turn is aborted through the SDK's own door and the teardown
// runs.
//
// ONE step is skipped on the signal: the REMINDER, which would spend a model call asking a session
// for an answer it was just told not to give.
//
// THE SUBSTITUTE POST IS NOT (fix round 2 of this item's review, Integrity #3). A SIGTERM is not
// only a withdrawal — it is a system shutdown, a logout, a `docker stop`, a person's `kill <pid>` —
// and only the BOARD knows whether the thread was settled. Skipping the post on the signal turned
// every one of those from a hard death the dead-holder sweep freed in about a minute into a TIDY
// silent end: the debt unsettled, the caller unwoken, and the fast path off until the two-hour
// bound, because the teardown now announces the session's end. So the two cases below are the two
// answers the board can give, and they are the whole of the rule.
//
// THE EXIT CODE IS 143 (`STOPPED_EXIT_CODE`): the number a shell reports for a process that died
// of SIGTERM (128 + 15), kept for a process that CAUGHT it, so a log reader sees one number for
// "stopped" whether or not the teardown ran — and never the 0 of a finished turn or the 1 of a
// failed one. Nothing reads it today (`SidecarWorker::trigger` spawns detached); it is stated so
// the first consumer that does is not left guessing.

test("a withdrawal's stop posts nothing in the session's name — the thread is already discharged", async (t) => {
  const { status, signal, stderr, calls, queries } = await runSidecarAndStopIt(t, {
    // The agent has thought (`toolUse`) and is mid-turn when the stop arrives — the shape a
    // withdrawal of a running round has, and the one where a retry would replay work.
    messages: [init, toolUse, toolResult],
    hangAt: 3,
    spec: { replyThread: "thread-77" },
    // `withdraw` discharges the thread BEFORE it signals, so this is what the board says by the
    // time the teardown reads it. A substitute post here would speak as the agent into a round the
    // engine has just taken back.
    nxcStdout: { "status --thread": board({ owes: false }) },
  });

  assert.equal(signal, null, `the process must not DIE of the signal — it caught it; stderr:\n${stderr}`);
  assert.equal(status, STOPPED_EXIT_CODE, `stderr:\n${stderr}`);
  assert.equal(STOPPED_EXIT_CODE, 143, "128 + SIGTERM, the number a shell would report");
  assert.match(stderr, /sidecar: session s-1 was told to stop \(SIGTERM\)/);
  assert.match(stderr, /sidecar stopped: role=tester session=s-1 real=real-1/);
  assert.doesNotMatch(stderr, /sidecar done:/, "a stopped turn is not a finished one");

  // The abort reached the SDK through the same door the reminder bound uses — the turn was not
  // retried (the stop is not a runtime failure) and not waited out.
  assert.equal(queries.length, 1, "one `query()`: the turn was neither retried nor reminded");
  assert.ok(
    "abortController" in queries[0].options,
    `the MAIN turn is handed the stop door, got: ${JSON.stringify(queries[0].options)}`,
  );
  assert.doesNotMatch(stderr, /retrying in/);

  // THE TEARDOWN THAT RAN, in its order — and the one step that did not.
  assert.deepEqual(
    verbs(calls),
    ["session bind", "transcript append", "status --thread", "session ended"],
    `stderr:\n${stderr}`,
  );
  assert.deepEqual(calls[0].argv, ["session", "bind", "s-1", "real-1"], "the runtime session is bound, so a resume can find it");
  assert.deepEqual(
    appends(calls)[0].entries.map((e) => e.kind),
    ["session_init", "tool_use", "tool_result"],
    "everything the aborted turn produced is flushed",
  );
  assert.equal(
    verbs(calls).filter((v) => v === "status --thread").length,
    1,
    "the board IS read, once, and by the step that has a decision to make: the reminder is skipped " +
      "on the signal, the substitute post is decided by what the register says",
  );
  assert.deepEqual(replyCalls(calls), [], "nothing is posted in the session's name");
  assert.match(stderr, /not reminding session s-1 .* told to stop/);
  assert.match(stderr, /posting nothing for thread thread-77 .* already discharged/);
  assert.deepEqual(calls[calls.length - 1].argv, ["session", "ended", "s-1"], "and the end is announced last");
});

test("a second SIGTERM during the stop neither kills the process nor starts a second teardown", async (t) => {
  // nxf 6j6v.27b9: the engine's tick now sends ONE further SIGTERM to a withdrawn session that is
  // still pinning its claim five minutes after the stop — automatically, not only when a person
  // types `kill`. So what this handler does with a second signal is load-bearing: it must not start
  // a second teardown (the transcript flushed twice, `session ended` announced twice, concurrently).
  const { status, signal, stderr, calls, signalledAgain } = await runSidecarAndStopIt(t, {
    messages: [init, toolUse, toolResult],
    hangAt: 3,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: false }) },
    // The board read takes a while, so the process is still mid-teardown when the second signal
    // arrives — the shape of a sidecar the tick finds still there.
    nxcDelays: { "status --thread": 800 },
    signalAgainWhen: /was told to stop \(SIGTERM\)/,
  });

  assert.equal(signalledAgain, true, "the second signal reached a live, mid-teardown process");
  assert.equal(signal, null, `it did not die of either signal; stderr:\n${stderr}`);
  assert.equal(status, STOPPED_EXIT_CODE, `stderr:\n${stderr}`);
  // The handler's own "told to stop again — already stopping" line is NOT asserted: the teardown's
  // `nxc` calls are synchronous, so a signal that lands inside one is only handed to JavaScript if
  // the event loop polls again before the process ends — which is timing, not behaviour. What is
  // behaviour, and pinned here, is that the second signal neither killed the process (its teardown
  // would have been cut short) nor started a second teardown.
  assert.equal(
    (stderr.match(/was told to stop \(SIGTERM\)/g) ?? []).length,
    1,
    "the stop began once",
  );
  assert.deepEqual(
    verbs(calls),
    ["session bind", "transcript append", "status --thread", "session ended"],
    `one teardown, in its order; stderr:\n${stderr}`,
  );
});

test("a SIGTERM that settled nothing still speaks for a thread that owes an answer", async (t) => {
  // The other answer the board can give, and the case this branch had broken: a system shutdown, a
  // logout, a `docker stop`, a person's `kill <pid>`. Nobody discharged anything, so the thread is
  // still waiting on this session — and the teardown now announces the session's end, which the
  // engine reads as a tidy shutdown rather than a hard death. If this step stayed silent the round
  // would hang until the two-hour bound with nothing saying why.
  const { status, signal, stderr, calls, queries } = await runSidecarAndStopIt(t, {
    messages: [init, toolUse, toolResult],
    hangAt: 3,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  assert.equal(signal, null, `the process must not DIE of the signal — it caught it; stderr:\n${stderr}`);
  assert.equal(status, STOPPED_EXIT_CODE, `stderr:\n${stderr}`);
  assert.equal(queries.length, 1, "the reminder is STILL skipped: a stopped session is spent no model call");
  assert.match(stderr, /not reminding session s-1 .* told to stop/);
  assert.deepEqual(
    replyCalls(calls),
    [
      {
        thread: "thread-77",
        ifUnanswered: true,
        escalate: false,
        text: "sidecar: this session ended without ever posting a reply",
      },
    ],
    "the thread keeps its debt unless something settles it, and the engine's --if-unanswered gate " +
      `is what decides whether this lands; stderr:\n${stderr}`,
  );
  assert.match(stderr, /this SIGTERM settled nothing/);
  assert.deepEqual(
    verbs(calls),
    ["session bind", "transcript append", "status --thread", "reply --thread", "session ended"],
    `stderr:\n${stderr}`,
  );
});

test("a stop that arrives during the retry backoff does not start one more round", async (t) => {
  // The window between two attempts is the one place in this loop where nothing is listening: the
  // abort door only reaches a `query()` that is already running, and `sleep` is a bare timer. A
  // SIGTERM that lands here used to be seen for the first time at the BOTTOM of the next attempt —
  // after `query()` had been entered again with an already-aborted controller, which is a round
  // started for a turn somebody had just ended. That is the one thing this file's own rule says a
  // stop may not do, so the door is read at the top of the loop as well.
  const { status, signal, stderr, queries, calls } = await runSidecarAndStopIt(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    // Throws before yielding anything: the model produced nothing, which is the retry's own
    // condition — a backoff that a stop can arrive in.
    throwAfter: 0,
    spec: { replyThread: "thread-77", runtimeRetries: 1, retryBackoffMs: 1500 },
    // Discharged, so the teardown's substitute post stays silent and this case keeps saying only
    // what it is about: the backoff. (What happens when the board says the thread still owes is
    // the sibling case above.)
    nxcStdout: { "status --thread": board({ owes: false }) },
    stopWhen: /retrying in 1500ms/,
  });

  assert.equal(signal, null, `the process must not DIE of the signal — it caught it; stderr:\n${stderr}`);
  assert.equal(status, STOPPED_EXIT_CODE, `stderr:\n${stderr}`);
  assert.equal(
    queries.length,
    1,
    `the backoff ended in a stop, not in another round; stderr:\n${stderr}`,
  );
  assert.match(stderr, /the runtime never ran this session/, "the premise: it was about to retry");
  assert.match(stderr, /sidecar stopped:/);
  assert.doesNotMatch(stderr, /sidecar done:/, "a stopped run is not a finished one");
  assert.deepEqual(
    replyCalls(calls),
    [],
    "and nothing is posted in the session's name: the board says the thread is discharged",
  );
});

test("a stop that arrives during the reminder round is honoured there too", async (t) => {
  // The reminder is a second `query()` with its own bound; a stop while it runs has to end THAT
  // wait, or the process outlives the withdrawal by up to the reminder bound with its pid file
  // still saying "running".
  const { status, signal, stderr, calls, queries } = await runSidecarAndStopIt(t, {
    messages: A_TURN_THEN_AN_OPEN_BLOCK,
    hangAt: 0,
    hangInRound: 1,
    spec: { replyThread: "thread-77" },
    nxcStdout: { "status --thread": board({ owes: true }) },
  });

  assert.equal(signal, null, `stderr:\n${stderr}`);
  assert.equal(status, STOPPED_EXIT_CODE, `stderr:\n${stderr}`);
  assert.equal(queries.length, 2, "the main turn and the reminder round it was stopped in");
  assert.match(stderr, /the reminder round was abandoned because the session was told to stop/);
  // The reminder read the board and found the thread OWING, which is why it resumed at all; the
  // stop ended that round without an answer, and the debt is therefore still there when the
  // substitute post asks the same register a moment later. It goes out — for the reason the
  // "SIGTERM that settled nothing" case above states — and it is not an ESCALATION: `remindOutcome`
  // is `stopped`, not `unanswered`, so nothing here claims the agent said it could not go on.
  assert.deepEqual(
    replyCalls(calls),
    [
      {
        thread: "thread-77",
        ifUnanswered: true,
        escalate: false,
        text: "sidecar: this session ended without ever posting a reply",
      },
    ],
    `stderr:\n${stderr}`,
  );
  assert.deepEqual(
    verbs(calls),
    [
      "transcript append",
      "session bind",
      "transcript append",
      "status --thread",
      "status --thread",
      "reply --thread",
      "session ended",
    ],
    `stderr:\n${stderr}`,
  );
  assert.match(stderr, /sidecar stopped: role=tester session=s-1 real=real-1 reminded=stopped/);
});
