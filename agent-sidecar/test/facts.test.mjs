// The sidecar's facts mode (§9, epic DoD 21). `main.mjs` runs as a child process against an SDK stub that replays a
// scripted stream, the way `main-teardown.test.mjs` does it, and the streams worth trusting are recorded from a real
// SDK run (`fixtures/sdk-stream-*.jsonl`). Timing is never asserted on: a child is watched through the files it writes,
// and the minute of the coalescing is a method the tracker's tests call themselves.

import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createFactTracker, createTranscriptWriter, programOf } from "../src/facts.mjs";

const MAIN = fileURLToPath(new URL("../src/main.mjs", import.meta.url));
const fixture = (name) =>
  readFileSync(fileURLToPath(new URL(`./fixtures/sdk-stream-${name}.jsonl`, import.meta.url)), "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));

const CANARY = "SECRETCANARY";

// ---- the tracker, with its minute in the test's hand ------------------------------------------

function tracker() {
  const facts = [];
  let now = 0;
  const t = createFactTracker({ now: () => now, emit: (fact) => facts.push(fact) });
  return { facts, t, advance: (ms) => (now += ms), kinds: () => facts.map((f) => f.beat ?? f.kind) };
}

const toolUse = (id, parent = null) => ({
  type: "assistant",
  parent_tool_use_id: parent,
  message: { content: [{ type: "tool_use", id, name: "Bash", input: { command: "x" } }] },
});
const toolResult = (id, parent = null) => ({
  type: "user",
  parent_tool_use_id: parent,
  message: { content: [{ type: "tool_result", tool_use_id: id, content: "ok" }] },
});
const result = (extra = {}) => ({ type: "result", subtype: "success", is_error: false, ...extra });

test("two overlapping calls where the first closes stay one busy period", () => {
  const { t, kinds } = tracker();
  t.message(toolUse("a"));
  t.message(toolUse("b"));
  t.message(toolResult("a"));
  t.tick();
  assert.deepEqual(kinds().filter((k) => k.startsWith("calls_")), ["calls_open"]);
});

test("a busy period closes at the end of a minute it was empty at, and not before", () => {
  const { t, kinds } = tracker();
  t.message(toolUse("a"));
  t.message(toolResult("a"));
  assert.deepEqual(kinds().filter((k) => k.startsWith("calls_")), ["calls_open"], "closed at once");
  t.tick();
  assert.deepEqual(kinds().filter((k) => k.startsWith("calls_")), ["calls_open", "calls_closed"]);
  t.tick();
  assert.equal(kinds().filter((k) => k === "calls_closed").length, 1, "said twice");
});

test("a call that opens and closes again inside the minute does not end the period", () => {
  const { t, kinds } = tracker();
  t.message(toolUse("a"));
  t.message(toolResult("a"));
  t.message(toolUse("b"));
  t.tick();
  assert.deepEqual(kinds().filter((k) => k.startsWith("calls_")), ["calls_open"]);
});

test("100 overlapping calls an hour stay at the per-minute rate", () => {
  const { t, facts, advance } = tracker();
  for (let minute = 0; minute < 60; minute += 1) {
    for (let call = 0; call < 2; call += 1) {
      const id = `m${minute}c${call}`;
      t.message({ type: "stream_event", event: {} });
      t.message(toolUse(id));
      t.message(toolUse(`${id}-over`));
      t.message(toolResult(id));
      t.message(toolResult(`${id}-over`));
      advance(30_000);
    }
    t.tick();
  }
  // A busy period per minute at the very most, and one model_output a minute.
  assert.ok(facts.length <= 60 * 3, `${facts.length} facts`);
  assert.equal(facts.filter((f) => f.beat === "model_output").length, 60);
});

test("the turn's result empties the set, so an interrupted call cannot pin it", () => {
  const { t, kinds } = tracker();
  t.message(toolUse("a"));
  t.message(result());
  t.tick();
  assert.deepEqual(kinds().filter((k) => k.startsWith("calls_")), ["calls_open", "calls_closed"]);
});

test("a sub-agent's call is a member of the set, by its own id", () => {
  const { t, kinds } = tracker();
  t.message(toolUse("task"));
  t.message(toolUse("inner", "task"));
  t.message(toolResult("task"));
  t.tick();
  assert.equal(kinds().filter((k) => k === "calls_closed").length, 0, "the sub-agent's call still runs");
  t.message(toolResult("inner", "task"));
  t.tick();
  assert.equal(kinds().filter((k) => k === "calls_closed").length, 1);
});

test("a live background task keeps the busy period open past the result, by the SDK's level signal", () => {
  const { t, kinds } = tracker();
  t.message(toolUse("a"));
  t.message({ type: "system", subtype: "background_tasks_changed", tasks: [{ task_id: "b1" }] });
  t.message(toolResult("a"));
  t.message(result());
  t.tick();
  assert.equal(kinds().filter((k) => k === "calls_closed").length, 0, "the background call still runs");
  t.message({ type: "system", subtype: "background_tasks_changed", tasks: [] });
  t.tick();
  assert.equal(kinds().filter((k) => k === "calls_closed").length, 1);
});

test("runtime heartbeats never count as life; the model's output does, once a minute", () => {
  const { t, facts, advance } = tracker();
  t.message({ type: "tool_progress", tool_use_id: "a", elapsed_time_seconds: 30 });
  t.message({ type: "system", subtype: "status", status: "requesting" });
  assert.deepEqual(facts, []);
  t.message({ type: "stream_event", event: {} });
  t.message({ type: "assistant", message: { content: [{ type: "text", text: "x" }] } });
  assert.equal(facts.length, 1);
  advance(60_000);
  t.message({ type: "stream_event", event: {} });
  assert.deepEqual(facts.map((f) => f.beat), ["model_output", "model_output"]);
});

test("started comes once from the first init, whatever a later turn's init says", () => {
  const { t, facts } = tracker();
  t.message({ type: "system", subtype: "init", session_id: "s-1" });
  t.message({ type: "system", subtype: "init", session_id: "s-1" });
  assert.equal(facts.filter((f) => f.kind === "started").length, 1);
  assert.equal(facts[0].backend_session_id, "s-1");
});

test("usage comes from the result, in the contract's shape", () => {
  const { t, facts } = tracker();
  t.message({
    type: "result",
    subtype: "success",
    is_error: false,
    num_turns: 3,
    usage: { input_tokens: 1, output_tokens: 2, cache_read_input_tokens: 3, cache_creation_input_tokens: 4 },
    modelUsage: { "claude-x": {} },
  });
  assert.deepEqual(
    facts.find((f) => f.kind === "usage"),
    { kind: "usage", model: "claude-x", input_tokens: 1, output_tokens: 2, cache_read_tokens: 3, cache_write_tokens: 4, turns: 3 },
  );
});

// ---- recorded streams -------------------------------------------------------------------------

function replay(name) {
  const { t, facts } = tracker();
  for (const message of fixture(name)) t.message(message);
  return { t, facts };
}

test("recorded: the sub-agent run's nested call opens a period and the result empties it", () => {
  const { t, facts } = replay("subagent");
  t.tick();
  const calls = facts.filter((f) => f.beat?.startsWith("calls_")).map((f) => f.beat);
  assert.equal(calls[0], "calls_open");
  assert.equal(calls[calls.length - 1], "calls_closed");
  assert.equal(facts.filter((f) => f.kind === "started").length, 1);
  assert.equal(facts.filter((f) => f.kind === "permission_refused").length, 0);
});

test("recorded: an interrupted call does not pin the set", () => {
  const { t, facts } = replay("interrupted");
  t.tick();
  assert.equal(facts.filter((f) => f.beat === "calls_closed").length, 1);
});

test("recorded: a background sub-agent holds the period until the level signal drops", () => {
  const { t, facts } = replay("background");
  t.tick();
  assert.equal(facts.filter((f) => f.beat === "calls_closed").length, 1);
  const first = facts.findIndex((f) => f.beat === "calls_open");
  assert.ok(first >= 0);
});

test("recorded: a refused command names its program and nothing of its arguments", () => {
  const { facts } = replay("denied");
  const refused = facts.filter((f) => f.kind === "permission_refused");
  assert.deepEqual(refused, [{ kind: "permission_refused", rule: "Bash", program: "curl" }]);
  assert.ok(!JSON.stringify(facts).includes(CANARY), "the canary reached a fact");
});

test("one refused call is one fact, however often the SDK reports it", () => {
  const { t, facts } = tracker();
  const denial = { tool_name: "Bash", tool_use_id: "a", tool_input: { command: "rm x" } };
  t.message(result({ permission_denials: [denial] }));
  t.message(result({ permission_denials: [denial] }));
  assert.equal(facts.filter((f) => f.kind === "permission_refused").length, 1);
});

// ---- the program-name rule --------------------------------------------------------------------

test("the program is the basename of the first word after leading assignments", () => {
  assert.equal(programOf("Bash", { command: "git push" }), "git");
  assert.equal(programOf("Bash", { command: "/usr/bin/git push" }), "git");
  assert.equal(programOf("Bash", { command: `TOKEN=${CANARY} A=1 curl -H ${CANARY}` }), "curl");
  assert.equal(programOf("Bash", { command: "  ./x.sh arg" }), "x.sh");
});

test("anything the rule cannot read is <unparsed>, never a guess that could carry a secret", () => {
  for (const command of [
    `$(echo ${CANARY}) x`,
    `"${CANARY} and more" x`,
    `'quoted' x`,
    `a;b${CANARY}`,
    `\`${CANARY}\``,
    `TOKEN=${CANARY}`,
    "",
    "   ",
    `A="${CANARY} two words" curl`,
  ]) {
    const program = programOf("Bash", { command });
    assert.equal(program, "<unparsed>", command);
  }
  assert.equal(programOf("Bash", {}), "<unparsed>");
  assert.equal(programOf("Bash", { command: 7 }), "<unparsed>");
});

test("a non-shell tool is named by the tool", () => {
  assert.equal(programOf("Write", { file_path: `/x/${CANARY}` }), "Write");
});

// ---- the transcript writer --------------------------------------------------------------------

test("a failed transcript flush keeps its entries and the next one delivers them, in order", () => {
  const dir = mkdtempSync(join(tmpdir(), "nxc-facts-"));
  try {
    const path = join(dir, "transcript.jsonl");
    const writer = createTranscriptWriter(path);
    // A directory at the path is a write that fails for as long as it stands.
    mkdirSync(path);
    writer.push([{ kind: "a" }]);
    assert.equal(writer.flush(), false);
    writer.push([{ kind: "b" }]);
    assert.equal(writer.flush(), false, "capture is still being tried");
    rmSync(path, { recursive: true });
    writer.push([{ kind: "c" }]);
    assert.equal(writer.flush(), true);
    const lines = readFileSync(path, "utf8").trim().split("\n").map((l) => JSON.parse(l).kind);
    assert.deepEqual(lines, ["a", "b", "c"]);
    assert.equal(writer.flush(), true, "nothing is written twice");
    assert.equal(readFileSync(path, "utf8").trim().split("\n").length, 3);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ---- the whole sidecar, as a child ------------------------------------------------------------

/** The SDK stub: it reads the prompt stream and replays one scripted round per user message, recording what it was
 *  asked. `hangAt` stops it before that index until the abort fires, and a `{ __wait: name }` step until a file exists. */
function sandbox(t, { rounds, hangAt = null, holdBeforeStream = null, spec = {}, env = {} }) {
  const dir = mkdtempSync(join(realpathSync(tmpdir()), "nxc-facts-child-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const ledger = join(dir, "ledger");
  mkdirSync(ledger);
  writeFileSync(
    join(dir, "sdk-stub.mjs"),
    `import { existsSync, writeFileSync, appendFileSync } from "node:fs";
const untilFile = async (name) => { while (!existsSync(${JSON.stringify(dir)} + "/" + name)) await new Promise((r) => setTimeout(r, 2)); };
const HOLD = ${JSON.stringify(holdBeforeStream)};
const ROUNDS = ${JSON.stringify(rounds)};
const HANG_AT = ${JSON.stringify(hangAt)};
const DIR = ${JSON.stringify(dir)};
export function query({ prompt, options }) {
  writeFileSync(DIR + "/options.json", JSON.stringify(options, (k, v) => (k === "abortController" || k === "env" ? undefined : v)));
  writeFileSync(DIR + "/resolved-cwd", process.cwd());
  return (async function* () {
    if (HOLD) await untilFile(HOLD);
    // The real SDK reads its input while a turn runs, so this does too: an ask yielded into a running turn is seen.
    const queue = [];
    let inputOver = false;
    (async () => {
      for await (const user of prompt) {
        appendFileSync(DIR + "/turns.jsonl", JSON.stringify(user.message.content) + "\\n");
        queue.push(user);
      }
      inputOver = true;
    })();
    let round = 0;
    for (;;) {
      if (queue.length === 0) {
        if (inputOver) return;
        await new Promise((r) => setTimeout(r, 2));
        continue;
      }
      queue.shift();
      const script = ROUNDS[round] ?? [];
      for (let i = 0; i < script.length; i += 1) {
        if (i === HANG_AT) {
          writeFileSync(DIR + "/hanging", "");
          await new Promise((resolve) => options.abortController.signal.addEventListener("abort", resolve));
          throw new Error("Operation aborted");
        }
        if (script[i].__wait) {
          writeFileSync(DIR + "/waiting", script[i].__wait);
          await untilFile(script[i].__wait);
          continue;
        }
        yield script[i];
      }
      round += 1;
    }
  })();
}
`,
  );
  writeFileSync(
    join(dir, "hook.mjs"),
    `const STUB = ${JSON.stringify(pathToFileURL(join(dir, "sdk-stub.mjs")).href)};
export async function resolve(specifier, context, next) {
  if (specifier === "@anthropic-ai/claude-agent-sdk") return { url: STUB, shortCircuit: true };
  return next(specifier, context);
}
`,
  );
  writeFileSync(
    join(dir, "register.mjs"),
    `import { register } from "node:module";
register(${JSON.stringify(pathToFileURL(join(dir, "hook.mjs")).href)});
`,
  );
  // A fake `nxc` on the PATH that records every call: facts mode must make none.
  const bin = join(dir, "bin");
  mkdirSync(bin);
  writeFileSync(join(bin, "nxc"), `#!/bin/sh\necho called >> "${dir}/nxc-calls"\n`, { mode: 0o755 });
  const cwd = join(dir, "work");
  mkdirSync(cwd);
  const specPath = join(ledger, "spec.json");
  writeFileSync(
    specPath,
    JSON.stringify({
      v: 1,
      session_key: "run1/build/v1/coder/a1",
      cwd,
      env: [{ name: "NXC_SESSION", value: "the-session" }],
      prompt: { system_prompt: "be good", use_claude_code_preset: true, declaration_hash: "h" },
      turn: { thread: "m-1", text: "the first turn" },
      resume: null,
      model: null,
      tools: ["Bash"],
      ledger_dir: ledger,
      ...spec,
    }),
  );
  const child = spawn(process.execPath, ["--import", join(dir, "register.mjs"), MAIN, "--facts", "--spec", specPath], {
    cwd,
    env: { PATH: `${bin}:${process.env.PATH}`, HOME: dir, ...env },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => (stderr += chunk));
  child.stdout.resume();
  const ended = new Promise((resolve) => child.on("close", (code, signal) => resolve({ code, signal, stderr })));
  t.after(() => child.kill("SIGKILL"));
  const facts = () =>
    existsSync(join(ledger, "facts.jsonl"))
      ? readFileSync(join(ledger, "facts.jsonl"), "utf8").trim().split("\n").filter(Boolean).map((l) => JSON.parse(l))
      : [];
  return { child, dir, ledger, ended, facts };
}

async function until(what, done) {
  const limit = Date.now() + 120_000;
  while (!done()) {
    assert.ok(Date.now() < limit, `never happened: ${what}`);
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

const init = { type: "system", subtype: "init", session_id: "real-1" };

test("facts mode writes child.json first, calls no nxc, runs in the spec's cwd and ends with the raw outcome", async (t) => {
  const box = sandbox(t, {
    rounds: [[init, toolUse("a"), toolResult("a"), result({ result: "the words" })]],
  });
  await box.ended;
  const record = JSON.parse(readFileSync(join(box.ledger, "child.json"), "utf8"));
  assert.equal(typeof record.pid, "number");
  assert.equal(typeof record.start_time, "string");
  assert.equal(record.handlers, true);
  assert.ok(!existsSync(join(box.dir, "nxc-calls")), "facts mode called nxc");
  const facts = box.facts();
  assert.deepEqual(facts[0], { kind: "started", backend_session_id: "real-1", pid: record.pid });
  const ended = facts[facts.length - 1];
  assert.equal(ended.kind, "ended");
  assert.equal(ended.raw.kind, "result");
  assert.equal(ended.raw.subtype, "success");
  assert.equal(ended.raw.is_error, false);
  const options = JSON.parse(readFileSync(join(box.dir, "options.json"), "utf8"));
  assert.equal(options.cwd, readFileSync(join(box.dir, "resolved-cwd"), "utf8"));
  assert.deepEqual(options.systemPrompt, { type: "preset", preset: "claude_code", append: "be good" });
  assert.deepEqual(options.allowedTools, ["Bash"]);
  assert.equal(options.includePartialMessages, true);
  assert.equal(options.forwardSubagentText, true);
});

test("facts mode captures the transcript beside the facts", async (t) => {
  const box = sandbox(t, { rounds: [[init, toolUse("a"), toolResult("a"), result()]] });
  await box.ended;
  const kinds = readFileSync(join(box.ledger, "transcript.jsonl"), "utf8")
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line).kind);
  assert.ok(kinds.includes("tool_use") && kinds.includes("result"), kinds.join());
});

test("the spec's fields reach the session: resume, model, the permission mode and the claude it was given", async (t) => {
  const box = sandbox(t, {
    rounds: [[init, result()]],
    spec: { resume: "s-0", model: "claude-x", permissions: "acceptEdits", claude_path: "/opt/claude" },
  });
  await box.ended;
  const options = JSON.parse(readFileSync(join(box.dir, "options.json"), "utf8"));
  assert.equal(options.resume, "s-0");
  assert.equal(options.model, "claude-x");
  assert.equal(options.permissionMode, "acceptEdits");
  assert.equal(options.pathToClaudeCodeExecutable, "/opt/claude");
});

test("a persona's declared tools restrict the base toolset, and the grant stays the approval inside it", async (t) => {
  // 6j6v.547w: as the old mode's resolveToolsOption/resolveAllowedTools, with `tools` the grant.
  const declared = sandbox(t, { rounds: [[init, result()]], spec: { tools: ["Bash(nxc reply:*)"], declared_tools: ["Read", "Grep"] } });
  await declared.ended;
  const options = JSON.parse(readFileSync(join(declared.dir, "options.json"), "utf8"));
  assert.deepEqual(options.tools, ["Read", "Grep", "Bash"]);
  assert.deepEqual(options.allowedTools, ["Read", "Grep", "Bash(nxc reply:*)"]);
});

test("an empty declaration narrows the base toolset to the grant's tool; no declaration leaves it whole", async (t) => {
  const empty = sandbox(t, { rounds: [[init, result()]], spec: { tools: ["Bash(nxc reply:*)"], declared_tools: [] } });
  await empty.ended;
  const narrowed = JSON.parse(readFileSync(join(empty.dir, "options.json"), "utf8"));
  assert.deepEqual(narrowed.tools, ["Bash"]);
  assert.deepEqual(narrowed.allowedTools, ["Bash(nxc reply:*)"]);
  const none = sandbox(t, { rounds: [[init, result()]], spec: { tools: ["Bash(nxc reply:*)"] } });
  await none.ended;
  const whole = JSON.parse(readFileSync(join(none.dir, "options.json"), "utf8"));
  assert.equal("tools" in whole, false, "no declaration sets no base toolset");
  assert.deepEqual(whole.allowedTools, ["Bash(nxc reply:*)"]);
});

test("SIGUSR2 is survived by a child that has written child.json", async (t) => {
  const box = sandbox(t, { rounds: [[init, { __wait: "release" }, result()]] });
  await until("the record and the wait", () => existsSync(join(box.ledger, "child.json")) && existsSync(join(box.dir, "waiting")));
  box.child.kill("SIGUSR2");
  writeFileSync(join(box.dir, "release"), "");
  const { code, signal } = await box.ended;
  assert.equal(signal, null);
  assert.equal(code, 0);
  assert.equal(box.facts().at(-1).raw.kind, "result", "it did not live to say how it ended");
});

test("the signal handlers stand before child.json: a signal at the first look does not kill it", async (t) => {
  // Nothing about order can be read from outside except this: from the instant the record exists, a SIGUSR2 is safe.
  const box = sandbox(t, { rounds: [[init, result()]], holdBeforeStream: "release" });
  await until("the record", () => existsSync(join(box.ledger, "child.json")));
  box.child.kill("SIGUSR2");
  writeFileSync(join(box.dir, "release"), "");
  const { signal } = await box.ended;
  assert.equal(signal, null);
  assert.equal(box.facts().at(-1).kind, "ended");
});

test("a nudge is delivered at the turn's yield as the next turn, and its file is gone", async (t) => {
  const box = sandbox(t, {
    rounds: [[init, { __wait: "release" }, result()], [result()]],
  });
  await until("the wait", () => existsSync(join(box.dir, "waiting")));
  writeFileSync(join(box.ledger, "ask.0001.json"), JSON.stringify({ text: "are you there?" }));
  box.child.kill("SIGUSR2");
  writeFileSync(join(box.dir, "release"), "");
  await box.ended;
  const turns = readFileSync(join(box.dir, "turns.jsonl"), "utf8").trim().split("\n").map((l) => JSON.parse(l));
  assert.deepEqual(turns, ["the first turn", "are you there?"]);
  assert.ok(!existsSync(join(box.ledger, "ask.0001.json")));
});

test("a nudge waits for the result: nothing reaches a running turn", async (t) => {
  const box = sandbox(t, { rounds: [[init, { __wait: "release" }, { __wait: "release2" }, result()], [result()]] });
  await until("the wait", () => existsSync(join(box.dir, "waiting")));
  writeFileSync(join(box.ledger, "ask.0001.json"), JSON.stringify({ text: "are you there?" }));
  box.child.kill("SIGUSR2");
  writeFileSync(join(box.dir, "release"), "");
  // The stub has gone on to its second wait, inside the same turn, with the ask known.
  await until("the second wait", () => readFileSync(join(box.dir, "waiting"), "utf8") === "release2");
  // Time for an ask to reach the stream if it were going to: a correct sidecar is never early, whatever this waits.
  await new Promise((resolve) => setTimeout(resolve, 200));
  const during = readFileSync(join(box.dir, "turns.jsonl"), "utf8").trim().split("\n");
  assert.equal(during.length, 1, "an ask reached a running turn");
  assert.ok(existsSync(join(box.ledger, "ask.0001.json")));
  writeFileSync(join(box.dir, "release2"), "");
  await box.ended;
  assert.ok(box.facts().some((f) => f.kind === "started"));
});

test("a stop aborts the turn, says so in the raw outcome and exits with the stop's code", async (t) => {
  const box = sandbox(t, { rounds: [[init, toolUse("a"), result()]], hangAt: 2 });
  await until("the hang", () => existsSync(join(box.dir, "hanging")));
  box.child.kill("SIGTERM");
  const { code, signal, stderr } = await box.ended;
  assert.equal(signal, null, stderr);
  assert.equal(code, 143, stderr);
  const ended = box.facts().find((f) => f.kind === "ended");
  assert.equal(ended.raw.kind, "stopped");
});

test("a stream that throws ends with its raw outcome, and the raw message never leaves the ended line", async (t) => {
  const box = sandbox(t, {
    rounds: [[init, { type: "assistant", error: "rate_limit", message: { content: [] } }, result({ is_error: true, subtype: "error_during_execution", result: `limit ${CANARY}` })]],
  });
  await box.ended;
  const facts = box.facts();
  const ended = facts[facts.length - 1];
  assert.equal(ended.raw.error, "rate_limit");
  assert.equal(ended.raw.is_error, true);
  assert.ok(facts.slice(0, -1).every((f) => !JSON.stringify(f).includes(CANARY)));
});

test("a rejected rate limit event travels in the raw outcome with its instant", async (t) => {
  const box = sandbox(t, {
    rounds: [[init, { type: "rate_limit_event", rate_limit_info: { status: "rejected", resetsAt: 1800000000, rateLimitType: "five_hour" } }, result({ is_error: true })]],
  });
  await box.ended;
  const ended = box.facts().at(-1);
  assert.equal(ended.raw.rate_limit.status, "rejected");
  assert.equal(ended.raw.rate_limit.resetsAt, 1800000000);
});

test("the refusal of the last turn travels with the end, so the adapter can say the end came right after it", async (t) => {
  const box = sandbox(t, {
    rounds: [[init, result({ permission_denials: [{ tool_name: "Bash", tool_use_id: "a", tool_input: { command: `curl ${CANARY}` } }] })]],
  });
  await box.ended;
  const facts = box.facts();
  assert.deepEqual(facts.at(-1).raw.refused, { rule: "Bash", program: "curl" });
  assert.ok(!JSON.stringify(facts).includes(CANARY));
});
