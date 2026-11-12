// The sidecar's facts mode (spec 6j6v.zjyv §9). With `--facts` it makes no call-back into `nxc` and takes no decision: it
// runs one SDK session from the v2 start spec, appends what happens to `<ledger>/facts.jsonl`, one `write` per line, and
// the Rust adapter reads them from there. Whatever the session should do about a fact is the reducer's.
//
// What the stream yields was recorded from a real SDK run (`test/fixtures/sdk-stream-*.jsonl`, tv58's first step), and
// what the process tree does around it is on that ticket too.

import { appendFileSync, readdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import {
  resolveAllowedTools,
  resolveModelOption,
  resolveSettingsSourcesOption,
  resolveToolsOption,
  STOPPED_EXIT_CODE,
} from "./spec-helpers.mjs";
import { createNormalizer } from "./transcript.mjs";

const MINUTE_MS = 60_000;
const SHELL_TOOLS = new Set(["Bash"]);
const SAFE_NAME = /^[A-Za-z0-9._+-]+$/;
const ASSIGNMENT = /^[A-Za-z_][A-Za-z0-9_]*=[^\s"'`$\;&|<>()]*$/;
/** Anything that makes the words of a command mean something a split on spaces cannot read. */
const SHELL_SYNTAX = /["'`$\;&|<>()]/;

/** **The program of a refused command**: the basename of the first word after leading `VAR=` assignments, or
 *  `<unparsed>`. There is no shell parser here and that is the point: it fails closed, so a secret in an argument, in an
 *  assignment's value or in a word the rule could not read never reaches a fact. A non-shell tool is named by the tool. */
export function programOf(toolName, input) {
  if (!SHELL_TOOLS.has(toolName)) return toolName;
  const command = input?.command;
  if (typeof command !== "string") return "<unparsed>";
  const words = command.trim().split(/\s+/);
  let index = 0;
  while (index < words.length && ASSIGNMENT.test(words[index])) index += 1;
  const word = words[index];
  if (!word || SHELL_SYNTAX.test(command.slice(0, command.indexOf(word) + word.length))) return "<unparsed>";
  const base = word.slice(word.lastIndexOf("/") + 1);
  return SAFE_NAME.test(base) ? base : "<unparsed>";
}

/** The facts of one stream. `emit` writes one, and `tick` is called at the end of every minute. No clock of its own and
 *  no timer, so a test owns the minute. */
export function createFactTracker({ now, emit }) {
  const openCalls = new Set();
  let backgroundTasks = 0;
  let inPeriod = false;
  let started = false;
  let lastOutput = null;
  const seenDenials = new Set();
  let lastRefusal = null;

  const busy = () => openCalls.size > 0 || backgroundTasks > 0;
  const heartbeat = (beat) => emit({ kind: "heartbeat", beat });
  const opened = () => {
    if (!inPeriod && busy()) {
      inPeriod = true;
      heartbeat("calls_open");
    }
  };

  function modelOutput() {
    const at = now();
    if (lastOutput !== null && at - lastOutput < MINUTE_MS) return;
    lastOutput = at;
    heartbeat("model_output");
  }

  function blocks(message) {
    const content = message.message?.content;
    return Array.isArray(content) ? content : [];
  }

  function usage(message) {
    const u = message.usage;
    if (!u) return;
    emit({
      kind: "usage",
      model: Object.keys(message.modelUsage ?? {})[0] ?? "",
      input_tokens: u.input_tokens ?? 0,
      output_tokens: u.output_tokens ?? 0,
      cache_read_tokens: u.cache_read_input_tokens ?? 0,
      cache_write_tokens: u.cache_creation_input_tokens ?? 0,
      turns: message.num_turns ?? 0,
    });
  }

  function refusals(message) {
    lastRefusal = null;
    for (const denial of message.permission_denials ?? []) {
      const refusal = { rule: denial.tool_name, program: programOf(denial.tool_name, denial.tool_input) };
      lastRefusal = refusal;
      if (seenDenials.has(denial.tool_use_id)) continue;
      seenDenials.add(denial.tool_use_id);
      emit({ kind: "permission_refused", ...refusal });
    }
  }

  return {
    message(message) {
      switch (message?.type) {
        case "system":
          if (message.subtype === "init" && !started) {
            started = true;
            emit({ kind: "started", backend_session_id: message.session_id, pid: process.pid });
          } else if (message.subtype === "background_tasks_changed") {
            // A level signal with replace semantics, and the CLI does emit it (measured on both platforms).
            backgroundTasks = (message.tasks ?? []).length;
            opened();
          }
          return;
        case "stream_event":
          modelOutput();
          return;
        case "assistant":
          modelOutput();
          for (const block of blocks(message)) {
            if (block.type === "tool_use" && block.id) openCalls.add(block.id);
          }
          opened();
          return;
        case "user":
          for (const block of blocks(message)) {
            if (block.type === "tool_result") openCalls.delete(block.tool_use_id);
          }
          return;
        case "result":
          // An interrupted call never gets a tool_result of its own, so the turn's end empties the set.
          openCalls.clear();
          usage(message);
          refusals(message);
          return;
        default:
          // `tool_progress`, `api_retry`, `keep_alive`: a runtime alive while its model produces nothing is exactly
          // what must not read as life.
          return;
      }
    },
    /** The end of a minute: one busy period is one pair of facts, whatever the call rate. */
    tick() {
      if (inPeriod && !busy()) {
        inPeriod = false;
        heartbeat("calls_closed");
      }
    },
    /** The refusal of the last turn's result, for the end to carry. */
    lastRefusal: () => lastRefusal,
  };
}

/** The normalised transcript, in `<ledger>/transcript.jsonl`. Captured always (LV1): a failed flush keeps its entries
 *  and the next one tries again, because a write that fails now says nothing about the next (the old mode's first-failure
 *  latch was for a database lock, and cost the rest of a run's transcript under "database is locked" on 6j6v.m8we). */
export function createTranscriptWriter(path) {
  const pending = [];
  let warned = false;
  return {
    push(entries) {
      pending.push(...entries);
    },
    /** Whether everything pending is on the disk now. */
    flush() {
      if (pending.length === 0) return true;
      const text = pending.map((entry) => `${JSON.stringify(entry)}\n`).join("");
      try {
        appendFileSync(path, text);
      } catch (err) {
        if (!warned) {
          warned = true;
          console.error(`sidecar: the transcript could not be written, tried again at each flush: ${err?.message ?? err}`);
        }
        return false;
      }
      pending.length = 0;
      warned = false;
      return true;
    },
  };
}

function askFiles(ledger) {
  let names;
  try {
    names = readdirSync(ledger);
  } catch {
    return [];
  }
  return names.filter((name) => name.startsWith("ask.") && name.endsWith(".json")).sort();
}

/** Every ask waiting in the ledger, oldest first, and each file gone: an ask is delivered once. */
function drainAsks(ledger) {
  const texts = [];
  for (const name of askFiles(ledger)) {
    const path = join(ledger, name);
    try {
      const { text } = JSON.parse(readFileSync(path, "utf8"));
      unlinkSync(path);
      if (typeof text === "string") texts.push(text);
    } catch (err) {
      console.error(`sidecar: the ask ${name} could not be read: ${err?.message ?? err}`);
    }
  }
  return texts;
}

/** The session's own input: the first turn, then every ask, each at a turn boundary. A user message yielded into a
 *  running turn is attached to its next tool result and ignored (measured on the old mode, 2026-10-25). It closes at the
 *  first `result` with no ask waiting. */
function turnStream({ first, ledger, stop, sessionId, wakeOn }) {
  let results = 0;
  let yielded = 0;
  let notify = null;
  const wake = () => {
    const pending = notify;
    notify = null;
    if (pending) pending();
  };
  stop.signal.addEventListener("abort", wake, { once: true });
  wakeOn(wake);
  const message = (text) => {
    yielded += 1;
    return { type: "user", message: { role: "user", content: text }, parent_tool_use_id: null, session_id: sessionId() };
  };
  async function* prompt() {
    yield message(first);
    for (;;) {
      if (stop.signal.aborted) return;
      if (results >= yielded) {
        const asks = drainAsks(ledger);
        if (asks.length === 0) return;
        for (const text of asks) yield message(text);
        continue;
      }
      await new Promise((resolve) => {
        notify = resolve;
      });
    }
  }
  return {
    prompt: prompt(),
    resultArrived() {
      results += 1;
      wake();
    },
  };
}

function sessionOptions(spec, stop) {
  const systemPrompt = spec.prompt.use_claude_code_preset
    ? { type: "preset", preset: "claude_code", append: spec.prompt.system_prompt }
    : spec.prompt.system_prompt;
  const env = { ...process.env };
  for (const { name, value } of spec.env ?? []) env[name] = value;
  // The options of the old mode (`main.mjs`), which a session has to find the same.
  const options = {
    systemPrompt,
    abortController: stop,
    // `declared_tools` is what the persona declares and `tools` what the engine grants (6j6v.547w): the old mode's
    // `tools` and `grantedTools`, under the same two rules.
    allowedTools: resolveAllowedTools(spec.declared_tools, spec.tools),
    permissionMode: spec.permissions ?? "default",
    cwd: spec.cwd,
    env,
    settingSources: resolveSettingsSourcesOption(),
    includePartialMessages: true,
    thinking: { type: "adaptive", display: "summarized" },
    forwardSubagentText: true,
  };
  if (spec.resume) options.resume = spec.resume;
  const tools = resolveToolsOption(spec.declared_tools, spec.tools);
  if (tools !== undefined) options.tools = tools;
  const model = resolveModelOption(spec.model);
  if (model !== undefined) options.model = model;
  if (typeof spec.claude_path === "string" && spec.claude_path !== "") {
    options.pathToClaudeCodeExecutable = spec.claude_path;
  }
  return options;
}

/** The one write of one line. A fact that cannot be written is kept and goes first next time: the file's lines are all
 *  the adapter ever learns, and a fact is not life or death enough to end a session over. */
function factWriter(path) {
  const pending = [];
  let warned = false;
  return (fact) => {
    pending.push(`${JSON.stringify(fact)}\n`);
    while (pending.length > 0) {
      try {
        appendFileSync(path, pending[0]);
        pending.shift();
      } catch (err) {
        if (!warned) {
          warned = true;
          console.error(`sidecar: the facts file could not be written: ${err?.message ?? err}`);
        }
        return;
      }
    }
  };
}

/** Runs the session and exits. Never returns to the old mode's code below its call in `main.mjs`. */
export async function runFactsMode({ query, specPath }) {
  const spec = JSON.parse(readFileSync(specPath, "utf8"));
  const ledger = spec.ledger_dir;

  // BEFORE `child.json`: Node's default for SIGUSR2 terminates the process, and that file is what says it may be sent.
  const stop = new AbortController();
  let wakeForAsk = () => {};
  process.on("SIGTERM", () => stop.abort());
  process.on("SIGUSR2", () => wakeForAsk());

  const temp = join(ledger, "child.json.tmp");
  const start = new Date(Date.now() - process.uptime() * 1000).toISOString();
  writeFileSync(temp, JSON.stringify({ pid: process.pid, start_time: start, handlers: true }));
  renameSync(temp, join(ledger, "child.json"));

  const write = factWriter(join(ledger, "facts.jsonl"));
  const tracker = createFactTracker({ now: () => Date.now(), emit: write });
  const transcript = createTranscriptWriter(join(ledger, "transcript.jsonl"));
  const normalizer = createNormalizer();
  // Not unref'd: this timer also keeps the loop from emptying while the session waits on nothing of Node's, which would
  // end the process at the unsettled await with no `ended` line. The end below is always an explicit exit.
  const minute = setInterval(() => tracker.tick(), MINUTE_MS);

  let sessionId = "";
  const stream = turnStream({
    first: spec.turn.text,
    ledger,
    stop,
    sessionId: () => sessionId,
    wakeOn: (wake) => {
      wakeForAsk = wake;
    },
  });

  // The raw outcome of the session, for the adapter's fixture table: nothing here says what it means.
  const raw = { kind: "stream_ended", subtype: null, is_error: false, message: null, error: null, rate_limit: null, refused: null };
  try {
    for await (const message of query({ prompt: stream.prompt, options: sessionOptions(spec, stop) })) {
      if (message?.type === "system" && message.subtype === "init") sessionId = message.session_id;
      tracker.message(message);
      if (message?.type === "assistant" && message.error) raw.error = message.error;
      if (message?.type === "rate_limit_event" && message.rate_limit_info?.status === "rejected") {
        raw.rate_limit = message.rate_limit_info;
      }
      transcript.push(normalizer.push(message));
      if (message?.type === "result") {
        raw.kind = "result";
        raw.subtype = message.subtype;
        raw.is_error = message.is_error === true;
        raw.message = message.result ?? null;
        raw.refused = tracker.lastRefusal();
        transcript.flush();
        stream.resultArrived();
      }
    }
  } catch (err) {
    if (!stop.signal.aborted) {
      raw.kind = "threw";
      raw.is_error = true;
      raw.message = String(err?.message ?? err);
    }
  }
  if (stop.signal.aborted) raw.kind = "stopped";
  clearInterval(minute);
  tracker.tick();
  try {
    transcript.push(normalizer.flush());
  } catch (err) {
    console.error(`sidecar: the transcript's last block could not be closed: ${err?.message ?? err}`);
  }
  transcript.flush();
  write({ kind: "ended", raw });
  process.exit(stop.signal.aborted ? STOPPED_EXIT_CODE : 0);
}
