// Unit tests for the pure transcript normalizer (nxf 0dbp). Every mapping rule of the T1↔T2 wire
// contract gets a case here: the normalizer is deliberately I/O-free so the whole contract is
// verifiable without an SDK session, a child process, or a database.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  createNormalizer,
  subagentTag,
  stringifyToolResultContent,
  capToolResult,
  capThinking,
  availabilityBoundary,
  MAX_THINKING_CHARS,
  MAX_TOOL_RESULT_CHARS,
  MAX_BOUNDARY_DETAIL_CHARS,
} from "../src/transcript.mjs";

/** Deterministic clock: every call returns the next whole second of 2026-07-26T00:00:XX.000Z, so
 *  a test can assert both the value and how many times the normalizer stamped a timestamp. */
function stubClock() {
  let n = 0;
  return () => `2026-07-26T00:00:${String(n++).padStart(2, "0")}.000Z`;
}

const AT0 = "2026-07-26T00:00:00.000Z";
const AT1 = "2026-07-26T00:00:01.000Z";
const AT2 = "2026-07-26T00:00:02.000Z";

const initMsg = (over = {}) => ({
  type: "system",
  subtype: "init",
  session_id: "sdk-session-1",
  model: "claude-opus-5",
  apiKeySource: "none",
  tools: ["Bash", "Read"],
  skills: ["brainstorming"],
  slash_commands: ["/help", "/clear", "/next"],
  permissionMode: "acceptEdits",
  ...over,
});

const textDelta = (text, over = {}) => ({
  type: "stream_event",
  event: { type: "content_block_delta", delta: { type: "text_delta", text } },
  parent_tool_use_id: null,
  ...over,
});

const thinkingDelta = (thinking, over = {}) => ({
  type: "stream_event",
  event: { type: "content_block_delta", delta: { type: "thinking_delta", thinking } },
  parent_tool_use_id: null,
  ...over,
});

const blockStop = (over = {}) => ({
  type: "stream_event",
  event: { type: "content_block_stop" },
  parent_tool_use_id: null,
  ...over,
});

const assistantMsg = (content, over = {}) => ({
  type: "assistant",
  message: { content },
  parent_tool_use_id: null,
  ...over,
});

const userMsg = (content, over = {}) => ({
  type: "user",
  message: { content },
  parent_tool_use_id: null,
  ...over,
});

const resultMsg = (over = {}) => ({
  type: "result",
  subtype: "success",
  is_error: false,
  total_cost_usd: 0.0123,
  num_turns: 3,
  result: "the answer",
  ...over,
});

test("session_init", async (t) => {
  await t.test("system/init maps to one session_init entry with counted collections", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(initMsg()), [
      {
        kind: "session_init",
        at: AT0,
        data: {
          sdkSessionId: "sdk-session-1",
          model: "claude-opus-5",
          apiKeySource: "none",
          tools: 2,
          skills: 1,
          slashCommands: 3,
          permissionMode: "acceptEdits",
        },
      },
    ]);
  });

  await t.test("absent collections are read defensively as 0", () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(
      initMsg({ tools: undefined, skills: undefined, slash_commands: undefined }),
    );
    assert.deepEqual(entry.data.tools, 0);
    assert.deepEqual(entry.data.skills, 0);
    assert.deepEqual(entry.data.slashCommands, 0);
  });

  await t.test("other system subtypes produce nothing", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push({ type: "system", subtype: "status", session_id: "s" }), []);
  });

  await t.test("session_init is never subagent-tagged", () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(initMsg({ parent_tool_use_id: "toolu_parent" }));
    assert.equal("parentToolUseId" in entry, false);
    assert.equal("subagentType" in entry, false);
  });
});

test("delta coalescing", async (t) => {
  await t.test("many text deltas coalesce into ONE assistant entry at flush()", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(textDelta("Hel")), []);
    assert.deepEqual(n.push(textDelta("lo, ")), []);
    assert.deepEqual(n.push(textDelta("world")), []);
    assert.deepEqual(n.flush(), [
      { kind: "assistant", at: AT0, data: { text: "Hello, world" } },
    ]);
  });

  await t.test("thinking and text stay separate blocks; the kind switch closes the open one", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(thinkingDelta("let me "));
    n.push(thinkingDelta("think"));
    assert.deepEqual(n.push(textDelta("answer")), [
      { kind: "thinking", at: AT0, data: { text: "let me think" } },
    ]);
    assert.deepEqual(n.flush(), [{ kind: "assistant", at: AT1, data: { text: "answer" } }]);
  });

  await t.test("a non-delta entry closes the open block first, in order", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("calling a tool"));
    assert.deepEqual(
      n.push(assistantMsg([{ type: "tool_use", id: "toolu_1", name: "Bash", input: { cmd: "ls" } }])),
      [
        { kind: "assistant", at: AT0, data: { text: "calling a tool" } },
        {
          kind: "tool_use",
          at: AT1,
          toolUseId: "toolu_1",
          data: { name: "Bash", input: { cmd: "ls" } },
        },
      ],
    );
  });

  await t.test("result closes the open block, result entry last", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("trailing"));
    const entries = n.push(resultMsg());
    assert.equal(entries.length, 2);
    assert.deepEqual(entries[0], { kind: "assistant", at: AT0, data: { text: "trailing" } });
    assert.equal(entries[1].kind, "result");
  });

  await t.test("an open block with empty accumulated text emits nothing", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta(""));
    assert.deepEqual(n.flush(), []);
  });

  await t.test("flush() with no open block emits nothing, and is idempotent", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("x"));
    assert.deepEqual(n.flush(), [{ kind: "assistant", at: AT0, data: { text: "x" } }]);
    assert.deepEqual(n.flush(), []);
  });

  await t.test("non-delta stream events produce nothing and leave the open block open", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("keep "));
    assert.deepEqual(n.push({ type: "stream_event", event: { type: "message_stop" }, parent_tool_use_id: null }), []);
    assert.deepEqual(
      n.push({
        type: "stream_event",
        event: { type: "content_block_delta", delta: { type: "signature_delta", signature: "sig" } },
        parent_tool_use_id: null,
      }),
      [],
    );
    n.push(textDelta("open"));
    assert.deepEqual(n.flush(), [{ kind: "assistant", at: AT0, data: { text: "keep open" } }]);
  });

  await t.test("an entry's `at` is when the block OPENED, not when it closed", () => {
    // A clock the test advances by hand, so wall time passes WHILE the block streams: a
    // stamp-on-close implementation would report 00:09 here and red, which a tick-counting stub
    // could not tell apart (it consumes no tick until the close either way).
    let second = 0;
    const n = createNormalizer({ now: () => `2026-07-26T00:00:${String(second).padStart(2, "0")}.000Z` });
    n.push(textDelta("a"));
    second = 5;
    n.push(textDelta("b"));
    second = 9;
    assert.deepEqual(n.flush(), [{ kind: "assistant", at: AT0, data: { text: "ab" } }]);
  });

  await t.test("content_block_stop closes the run, so two same-kind blocks stay two entries", () => {
    // PR #258 review, Code Quality #1. Coalescing used to break only on a kind or provenance
    // change, so two SEPARATE thinking blocks — the shape a model produces when it reasons either
    // side of a tool call — fused into one entry carrying only the first block's timestamp. The
    // block's own `content_block_stop` is the real boundary; without this the two texts below
    // would come back as a single "firstsecond" at AT0.
    const n = createNormalizer({ now: stubClock() });
    n.push(thinkingDelta("first"));
    assert.deepEqual(n.push(blockStop()), [{ kind: "thinking", at: AT0, data: { text: "first" } }]);
    n.push(thinkingDelta("second"));
    assert.deepEqual(n.flush(), [{ kind: "thinking", at: AT1, data: { text: "second" } }]);
  });

  await t.test("content_block_stop with no open block emits nothing", () => {
    // The stop that closes a tool_use block (which never opens a coalescing run) must not
    // manufacture an empty entry.
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(blockStop()), []);
    assert.deepEqual(n.flush(), []);
  });
});

test("assistant messages", async (t) => {
  await t.test("final text is emitted only when NO text delta was seen this turn", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(assistantMsg([{ type: "text", text: "no partials arrived" }])), [
      { kind: "assistant", at: AT0, data: { text: "no partials arrived" } },
    ]);
  });

  await t.test("sawDelta dedup: partials arrived, so the final text block is dropped", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("streamed"));
    // The final message adds nothing (it would duplicate the coalesced block), so it produces no
    // entry — and producing none leaves the streamed block open for the next event.
    assert.deepEqual(n.push(assistantMsg([{ type: "text", text: "streamed" }])), []);
    // …which means the answer is still emitted exactly once, on the turn's `result`.
    assert.deepEqual(n.push(resultMsg())[0], {
      kind: "assistant",
      at: AT0,
      data: { text: "streamed" },
    });
  });

  await t.test("sawDelta resets on result, so the next turn's fallback text is emitted", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("turn one"));
    n.push(resultMsg());
    const entries = n.push(assistantMsg([{ type: "text", text: "turn two" }]));
    assert.equal(entries.length, 1);
    assert.deepEqual(entries[0].data, { text: "turn two" });
  });

  await t.test("a subagent's deltas do NOT suppress the main conversation's fallback text", () => {
    // sawDelta is per PROVENANCE, not global: with `forwardSubagentText` on, a subagent streams
    // its own deltas into the same stream. Those must not make the main conversation's final
    // `text` block look like a duplicate — that would silently swallow the actual answer.
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("subagent streamed", { parent_tool_use_id: "toolu_task" }));
    const entries = n.push(assistantMsg([{ type: "text", text: "the main answer" }]));
    assert.deepEqual(entries, [
      { kind: "assistant", at: AT0, parentToolUseId: "toolu_task", data: { text: "subagent streamed" } },
      { kind: "assistant", at: AT1, data: { text: "the main answer" } },
    ]);
  });

  await t.test("the main conversation's deltas do NOT suppress a subagent's fallback text", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("main streamed"));
    const entries = n.push(
      assistantMsg([{ type: "text", text: "the subagent's reply" }], {
        parent_tool_use_id: "toolu_task",
        subagent_type: "code-reviewer",
      }),
    );
    assert.deepEqual(entries, [
      { kind: "assistant", at: AT0, data: { text: "main streamed" } },
      {
        kind: "assistant",
        at: AT1,
        parentToolUseId: "toolu_task",
        subagentType: "code-reviewer",
        data: { text: "the subagent's reply" },
      },
    ]);
  });

  await t.test("dedup still applies WITHIN one provenance (subagent partials + subagent text)", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("streamed by the subagent", { parent_tool_use_id: "toolu_task" }));
    assert.deepEqual(
      n.push(
        assistantMsg([{ type: "text", text: "streamed by the subagent" }], {
          parent_tool_use_id: "toolu_task",
        }),
      ),
      [],
    );
  });

  await t.test("thinking deltas do NOT set sawDelta", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(thinkingDelta("hmm"));
    const entries = n.push(assistantMsg([{ type: "text", text: "the answer" }]));
    assert.deepEqual(entries, [
      { kind: "thinking", at: AT0, data: { text: "hmm" } },
      { kind: "assistant", at: AT1, data: { text: "the answer" } },
    ]);
  });

  await t.test("content blocks are emitted in order; unknown block types are skipped", () => {
    const n = createNormalizer({ now: stubClock() });
    const entries = n.push(
      assistantMsg([
        { type: "thinking", thinking: "not emitted here — thinking rides the partials" },
        { type: "tool_use", id: "toolu_a", name: "Read", input: { file: "a" } },
        { type: "text", text: "and then some prose" },
        { type: "tool_use", id: "toolu_b", name: "Read", input: { file: "b" } },
      ]),
    );
    assert.deepEqual(
      entries.map((e) => [e.kind, e.toolUseId]),
      [
        ["tool_use", "toolu_a"],
        ["assistant", undefined],
        ["tool_use", "toolu_b"],
      ],
    );
  });

  await t.test("non-array content is guarded", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(assistantMsg("a bare string")), []);
    assert.deepEqual(n.push(assistantMsg(undefined)), []);
    assert.deepEqual(n.push({ type: "assistant", parent_tool_use_id: null }), []);
  });

  await t.test("assistant text is NOT capped on the fallback path (it is the actual answer)", () => {
    const n = createNormalizer({ now: stubClock() });
    const huge = "z".repeat(MAX_THINKING_CHARS + MAX_TOOL_RESULT_CHARS + 10);
    const [entry] = n.push(assistantMsg([{ type: "text", text: huge }]));
    assert.equal(entry.data.text.length, huge.length);
  });

  await t.test("assistant text is NOT capped on the COALESCED path either", () => {
    // Separate line from the fallback path (the cap lives in a ternary on the close path), so a
    // long streamed answer needs its own case — hoisting the cap out of that ternary would
    // silently truncate real answers otherwise.
    const n = createNormalizer({ now: stubClock() });
    const huge = "z".repeat(MAX_THINKING_CHARS + 500);
    n.push(textDelta(huge.slice(0, 1000)));
    n.push(textDelta(huge.slice(1000)));
    const [entry] = n.flush();
    assert.equal(entry.data.text.length, huge.length);
    assert.equal(entry.data.text, huge);
  });
});

test("assistant errors", async (t) => {
  await t.test('"authentication_failed" classifies as auth', () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(assistantMsg([], { error: "authentication_failed" })), [
      { kind: "error", at: AT0, data: { kind: "auth", message: "authentication_failed" } },
    ]);
  });

  await t.test('"oauth_org_not_allowed" classifies as auth', () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(assistantMsg([], { error: "oauth_org_not_allowed" }));
    assert.equal(entry.data.kind, "auth");
  });

  // `rate_limit` stood here until nxf 6j6v.npy3 gave it a class of its own; see "the availability
  // class" below, which pins both sides of that line.
  await t.test("anything else classifies as runtime", () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(assistantMsg([], { error: "invalid_request" }));
    assert.deepEqual(entry.data, { kind: "runtime", message: "invalid_request" });
  });

  await t.test("an error closes the open block first and drops the content blocks", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("partial"));
    const entries = n.push(
      assistantMsg([{ type: "text", text: "ignored" }], { error: "server_error" }),
    );
    assert.deepEqual(entries, [
      { kind: "assistant", at: AT0, data: { text: "partial" } },
      { kind: "error", at: AT1, data: { kind: "runtime", message: "server_error" } },
    ]);
  });

  await t.test("error is never subagent-tagged", () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(
      assistantMsg([], { error: "server_error", parent_tool_use_id: "toolu_parent" }),
    );
    assert.equal("parentToolUseId" in entry, false);
  });
});

test("tool results", async (t) => {
  await t.test("a tool_result block maps to a tool_result entry", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(
      n.push(userMsg([{ type: "tool_result", tool_use_id: "toolu_1", content: "ok" }])),
      [
        {
          kind: "tool_result",
          at: AT0,
          toolUseId: "toolu_1",
          data: { content: "ok", isError: false },
        },
      ],
    );
  });

  await t.test("is_error true rides through as isError", () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(
      userMsg([{ type: "tool_result", tool_use_id: "toolu_1", content: "boom", is_error: true }]),
    );
    assert.equal(entry.data.isError, true);
  });

  await t.test("a REPLAYED user message emits nothing (SDKUserMessageReplay.isReplay)", () => {
    // A resume that replayed history would otherwise re-append every historical tool_result on
    // every resume. `isReplay: true` (sdk.d.ts:4601) is a required literal on the replay variant
    // and absent from the live one, so the guard is exact.
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(
      n.push(
        userMsg([{ type: "tool_result", tool_use_id: "toolu_old", content: "from a past turn" }], {
          isReplay: true,
        }),
      ),
      [],
    );
  });

  await t.test("a replayed message does not close an open block either", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("live "));
    n.push(userMsg([{ type: "tool_result", tool_use_id: "toolu_old", content: "old" }], { isReplay: true }));
    n.push(textDelta("stream"));
    assert.deepEqual(n.flush(), [{ kind: "assistant", at: AT0, data: { text: "live stream" } }]);
  });

  await t.test("a LIVE user message (no isReplay) still emits its tool_result", () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(userMsg([{ type: "tool_result", tool_use_id: "toolu_1", content: "live" }]));
    assert.equal(entry.kind, "tool_result");
    assert.equal(entry.data.content, "live");
  });

  await t.test("a real user turn (no tool_result blocks) emits nothing", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(userMsg([{ type: "text", text: "hello agent" }])), []);
  });

  await t.test("non-array user content is guarded", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(userMsg("plain string turn")), []);
    assert.deepEqual(n.push({ type: "user", parent_tool_use_id: null }), []);
  });

  await t.test("a tool_result closes the open block first", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("before the result"));
    const entries = n.push(userMsg([{ type: "tool_result", tool_use_id: "t", content: "x" }]));
    assert.equal(entries.length, 2);
    assert.equal(entries[0].kind, "assistant");
    assert.equal(entries[1].kind, "tool_result");
  });
});

test("stringifyToolResultContent", async (t) => {
  await t.test("a string passes through", () => {
    assert.equal(stringifyToolResultContent("plain"), "plain");
  });

  await t.test("an array concatenates text blocks and names other block types", () => {
    assert.equal(
      stringifyToolResultContent([
        { type: "text", text: "a" },
        { type: "image", source: {} },
        { type: "text" },
        { type: "text", text: "b" },
      ]),
      "a[image]b",
    );
  });

  await t.test("non-object array members contribute nothing", () => {
    assert.equal(stringifyToolResultContent([null, 42, { type: "text", text: "c" }]), "c");
  });

  await t.test("anything else is the empty string", () => {
    assert.equal(stringifyToolResultContent(undefined), "");
    assert.equal(stringifyToolResultContent(null), "");
    assert.equal(stringifyToolResultContent({ type: "text", text: "x" }), "");
  });
});

test("caps", async (t) => {
  await t.test("tool result exactly at the cap is untouched", () => {
    const text = "x".repeat(MAX_TOOL_RESULT_CHARS);
    assert.equal(capToolResult(text), text);
  });

  await t.test("tool result one char over the cap is truncated with a note", () => {
    const text = "x".repeat(MAX_TOOL_RESULT_CHARS + 1);
    assert.equal(
      capToolResult(text),
      `${"x".repeat(MAX_TOOL_RESULT_CHARS)}\n…[truncated 1 chars]`,
    );
  });

  await t.test("the tool result cap is applied inside the normalizer", () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(
      userMsg([
        { type: "tool_result", tool_use_id: "t", content: "y".repeat(MAX_TOOL_RESULT_CHARS + 25) },
      ]),
    );
    assert.equal(
      entry.data.content,
      `${"y".repeat(MAX_TOOL_RESULT_CHARS)}\n…[truncated 25 chars]`,
    );
  });

  await t.test("thinking exactly at the cap is untouched", () => {
    const text = "t".repeat(MAX_THINKING_CHARS);
    assert.equal(capThinking(text), text);
  });

  await t.test("thinking one char over the cap is truncated with a note", () => {
    assert.equal(
      capThinking("t".repeat(MAX_THINKING_CHARS + 1)),
      `${"t".repeat(MAX_THINKING_CHARS)}\n…[truncated 1 chars]`,
    );
  });

  await t.test("the thinking cap is applied to the COALESCED block, not per delta", () => {
    const n = createNormalizer({ now: stubClock() });
    // Two deltas, each well under the cap, that together overflow it by 10 chars.
    n.push(thinkingDelta("a".repeat(MAX_THINKING_CHARS - 5)));
    n.push(thinkingDelta("b".repeat(15)));
    const [entry] = n.flush();
    assert.equal(entry.data.text.length, MAX_THINKING_CHARS + "\n…[truncated 10 chars]".length);
    assert.match(entry.data.text, /\n…\[truncated 10 chars\]$/);
  });

  await t.test("the caps are the contract's values", () => {
    assert.equal(MAX_THINKING_CHARS, 4000);
    assert.equal(MAX_TOOL_RESULT_CHARS, 8000);
  });
});

test("result entries", async (t) => {
  await t.test("carries subtype, isError, cost, turns and the result text", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push(resultMsg()), [
      {
        kind: "result",
        at: AT0,
        data: {
          subtype: "success",
          isError: false,
          totalCostUsd: 0.0123,
          numTurns: 3,
          resultText: "the answer",
        },
      },
    ]);
  });

  await t.test("resultText is absent when the message has no `result` field", () => {
    // SDKResultError carries no `result` — the key must not appear at all (not as undefined).
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push({
      type: "result",
      subtype: "error_during_execution",
      is_error: true,
      total_cost_usd: 0,
      num_turns: 1,
    });
    assert.equal("resultText" in entry.data, false);
    assert.deepEqual(entry.data, {
      subtype: "error_during_execution",
      isError: true,
      totalCostUsd: 0,
      numTurns: 1,
    });
  });

  await t.test("result is never subagent-tagged", () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(resultMsg({ parent_tool_use_id: "toolu_parent" }));
    assert.equal("parentToolUseId" in entry, false);
    assert.equal("subagentType" in entry, false);
  });
});

test("subagentTag", async (t) => {
  await t.test("absent parent_tool_use_id -> null", () => {
    assert.equal(subagentTag({ type: "result" }), null);
  });

  await t.test("null parent_tool_use_id -> null", () => {
    assert.equal(subagentTag({ type: "assistant", parent_tool_use_id: null }), null);
  });

  await t.test("parent only -> parentToolUseId, no subagentType key", () => {
    assert.deepEqual(subagentTag({ type: "assistant", parent_tool_use_id: "toolu_p" }), {
      parentToolUseId: "toolu_p",
    });
  });

  await t.test("parent + subagent_type -> both", () => {
    assert.deepEqual(
      subagentTag({ type: "assistant", parent_tool_use_id: "toolu_p", subagent_type: "code-reviewer" }),
      { parentToolUseId: "toolu_p", subagentType: "code-reviewer" },
    );
  });
});

test("subagent tagging in the stream", async (t) => {
  await t.test("a subagent's coalesced text carries the tag", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("sub", { parent_tool_use_id: "toolu_task" }));
    n.push(textDelta("agent", { parent_tool_use_id: "toolu_task" }));
    assert.deepEqual(n.flush(), [
      {
        kind: "assistant",
        at: AT0,
        parentToolUseId: "toolu_task",
        data: { text: "subagent" },
      },
    ]);
  });

  await t.test("tool_use and tool_result inside a subagent carry the tag + subagentType", () => {
    const n = createNormalizer({ now: stubClock() });
    const [use] = n.push(
      assistantMsg([{ type: "tool_use", id: "toolu_x", name: "Grep", input: {} }], {
        parent_tool_use_id: "toolu_task",
        subagent_type: "code-reviewer",
      }),
    );
    assert.deepEqual(use, {
      kind: "tool_use",
      at: AT0,
      toolUseId: "toolu_x",
      parentToolUseId: "toolu_task",
      subagentType: "code-reviewer",
      data: { name: "Grep", input: {} },
    });
    const [res] = n.push(
      userMsg([{ type: "tool_result", tool_use_id: "toolu_x", content: "hit" }], {
        parent_tool_use_id: "toolu_task",
        subagent_type: "code-reviewer",
      }),
    );
    assert.deepEqual(res, {
      kind: "tool_result",
      at: AT1,
      toolUseId: "toolu_x",
      parentToolUseId: "toolu_task",
      subagentType: "code-reviewer",
      data: { content: "hit", isError: false },
    });
  });

  await t.test("a subagent thinking block carries the tag", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(thinkingDelta("sub-reasoning", { parent_tool_use_id: "toolu_task" }));
    assert.deepEqual(n.flush(), [
      { kind: "thinking", at: AT0, parentToolUseId: "toolu_task", data: { text: "sub-reasoning" } },
    ]);
  });

  await t.test("main-conversation entries carry no tag keys at all", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("main"));
    const [entry] = n.flush();
    assert.deepEqual(Object.keys(entry), ["kind", "at", "data"]);
  });

  await t.test("a subagent delta closes the open MAIN block instead of merging into it", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("main text "));
    assert.deepEqual(n.push(textDelta("sub text", { parent_tool_use_id: "toolu_task" })), [
      { kind: "assistant", at: AT0, data: { text: "main text " } },
    ]);
    assert.deepEqual(n.flush(), [
      { kind: "assistant", at: AT1, parentToolUseId: "toolu_task", data: { text: "sub text" } },
    ]);
  });

  await t.test("switching between two different subagents closes the block too", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("from a", { parent_tool_use_id: "toolu_a" }));
    assert.deepEqual(n.push(textDelta("from b", { parent_tool_use_id: "toolu_b" })), [
      { kind: "assistant", at: AT0, parentToolUseId: "toolu_a", data: { text: "from a" } },
    ]);
    assert.deepEqual(n.flush(), [
      { kind: "assistant", at: AT1, parentToolUseId: "toolu_b", data: { text: "from b" } },
    ]);
  });

  await t.test("deltas under the SAME parent keep coalescing", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("one ", { parent_tool_use_id: "toolu_a" }));
    n.push(textDelta("block", { parent_tool_use_id: "toolu_a" }));
    assert.deepEqual(n.flush(), [
      { kind: "assistant", at: AT0, parentToolUseId: "toolu_a", data: { text: "one block" } },
    ]);
  });
});

test("unhandled message types", async (t) => {
  await t.test("status/hook/progress messages produce nothing", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.deepEqual(n.push({ type: "status", session_id: "s" }), []);
    assert.deepEqual(n.push({ type: "hook_started", session_id: "s" }), []);
    assert.deepEqual(n.push({ type: "tool_progress", session_id: "s" }), []);
  });

  await t.test("an unhandled type does not close an open block", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("still "));
    n.push({ type: "status", session_id: "s" });
    n.push(textDelta("streaming"));
    assert.deepEqual(n.flush(), [
      { kind: "assistant", at: AT0, data: { text: "still streaming" } },
    ]);
  });
});

test("the injected clock", async (t) => {
  await t.test("createNormalizer() with no argument uses a real ISO-8601 UTC clock", () => {
    const n = createNormalizer();
    const [entry] = n.push(initMsg());
    assert.match(entry.at, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/);
  });

  await t.test("every entry is stamped from the injected clock, in emission order", () => {
    const n = createNormalizer({ now: stubClock() });
    const entries = [
      ...n.push(initMsg()),
      ...n.push(assistantMsg([{ type: "text", text: "hi" }])),
      ...n.push(resultMsg()),
    ];
    assert.deepEqual(
      entries.map((e) => e.at),
      [AT0, AT1, AT2],
    );
  });
});

test("a full ordered run", async (t) => {
  await t.test("session_init → thinking → tool_use → tool_result → assistant → result", () => {
    const n = createNormalizer({ now: stubClock() });
    const entries = [
      ...n.push(initMsg()),
      ...n.push(thinkingDelta("plan it")),
      ...n.push(assistantMsg([{ type: "tool_use", id: "toolu_1", name: "Bash", input: { cmd: "ls" } }])),
      ...n.push(userMsg([{ type: "tool_result", tool_use_id: "toolu_1", content: "a.txt" }])),
      ...n.push(textDelta("Found ")),
      ...n.push(textDelta("one file.")),
      ...n.push(resultMsg()),
      ...n.flush(),
    ];
    assert.deepEqual(
      entries.map((e) => e.kind),
      ["session_init", "thinking", "tool_use", "tool_result", "assistant", "result"],
    );
    assert.equal(entries[4].data.text, "Found one file.");
  });
});

// ── nxf 6j6v.npy3: the third error class, and the structured boundary behind it ─────────────────

test("the availability class", async (t) => {
  await t.test('"rate_limit" classifies as unavailable, not runtime', () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(assistantMsg([], { error: "rate_limit" }));
    assert.deepEqual(entry.data, { kind: "unavailable", message: "rate_limit" });
  });

  await t.test('"overloaded" classifies as unavailable', () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(assistantMsg([], { error: "overloaded" }));
    assert.equal(entry.data.kind, "unavailable");
  });

  // The subset is NAMED, and these two are the boundary of it in both directions.
  await t.test('"billing_error" stays a RUNTIME failure — a human repairs it by acting', () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(assistantMsg([], { error: "billing_error" }));
    assert.equal(entry.data.kind, "runtime");
  });

  await t.test('"server_error" stays runtime', () => {
    const n = createNormalizer({ now: stubClock() });
    const [entry] = n.push(assistantMsg([], { error: "server_error" }));
    assert.equal(entry.data.kind, "runtime");
  });

  await t.test("the two auth errors are untouched by the new class", () => {
    const n = createNormalizer({ now: stubClock() });
    assert.equal(n.push(assistantMsg([], { error: "authentication_failed" }))[0].data.kind, "auth");
    assert.equal(n.push(assistantMsg([], { error: "oauth_org_not_allowed" }))[0].data.kind, "auth");
  });
});

test("rate_limit_event", async (t) => {
  const rateLimitEvent = (info) => ({ type: "rate_limit_event", rate_limit_info: info });

  await t.test("a REJECTED event is remembered and produces no transcript entry of its own", () => {
    const n = createNormalizer({ now: stubClock() });
    const info = { status: "rejected", resetsAt: 1789000000, rateLimitType: "seven_day" };
    assert.deepEqual(n.push(rateLimitEvent(info)), []);
    assert.deepEqual(n.rateLimit(), info);
  });

  // `status: 'allowed'` fires routinely whenever the utilization changes; remembering those would
  // report an availability boundary on every healthy run.
  await t.test("an ALLOWED event is not remembered", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(rateLimitEvent({ status: "allowed", utilization: 12 }));
    assert.equal(n.rateLimit(), null);
  });

  await t.test("an allowed event does NOT erase a rejection already seen", () => {
    const n = createNormalizer({ now: stubClock() });
    const rejected = { status: "rejected", resetsAt: 1789000000 };
    n.push(rateLimitEvent(rejected));
    n.push(rateLimitEvent({ status: "allowed" }));
    assert.deepEqual(n.rateLimit(), rejected);
  });

  await t.test("it does not close an open coalescing block", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(textDelta("half "));
    assert.deepEqual(n.push(rateLimitEvent({ status: "rejected" })), []);
    n.push(textDelta("a sentence"));
    assert.deepEqual(n.flush(), [
      { kind: "assistant", at: AT0, data: { text: "half a sentence" } },
    ]);
  });

  await t.test("no rate limit seen at all reads as null", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push(initMsg());
    assert.equal(n.rateLimit(), null);
  });

  await t.test("a malformed event is ignored rather than remembered", () => {
    const n = createNormalizer({ now: stubClock() });
    n.push({ type: "rate_limit_event" });
    n.push({ type: "rate_limit_event", rate_limit_info: null });
    n.push({ type: "rate_limit_event", rate_limit_info: "rejected" });
    assert.equal(n.rateLimit(), null);
  });
});

test("availabilityBoundary", async (t) => {
  const rejected = (over = {}) => ({ status: "rejected", resetsAt: 1789000000, ...over });

  await t.test("a healthy run has no boundary", () => {
    assert.equal(availabilityBoundary({ assistantError: null, rateLimit: null }), null);
  });

  await t.test("a runtime error that is not an availability class is no boundary", () => {
    assert.equal(availabilityBoundary({ assistantError: "server_error", rateLimit: null }), null);
  });

  // EITHER signal is enough, and both are structured: the assistant error names the class, the
  // rate-limit event names the instant. A run can produce one without the other.
  await t.test("the assistant error alone is a boundary, with no instant", () => {
    const b = availabilityBoundary({ assistantError: "rate_limit", rateLimit: null });
    assert.equal(b.limit, "rate_limit");
    assert.equal(b.until, null);
  });

  await t.test("a rejected rate limit alone is a boundary", () => {
    const b = availabilityBoundary({ assistantError: null, rateLimit: rejected() });
    assert.equal(b.until, "2026-09-10T00:26:40.000Z");
  });

  await t.test("rateLimitType names WHICH window, in preference to the error name", () => {
    const b = availabilityBoundary({
      assistantError: "rate_limit",
      rateLimit: rejected({ rateLimitType: "seven_day" }),
    });
    assert.equal(b.limit, "seven_day");
  });

  await t.test("`overloaded` is a boundary too", () => {
    assert.ok(availabilityBoundary({ assistantError: "overloaded", rateLimit: null }));
  });

  // `resetsAt?: number` does not say which unit. Both are accepted, and the split is at a value no
  // real reset instant can be on the wrong side of.
  await t.test("resetsAt in SECONDS is read as seconds", () => {
    const b = availabilityBoundary({ assistantError: null, rateLimit: rejected({ resetsAt: 1789000000 }) });
    assert.equal(b.until, "2026-09-10T00:26:40.000Z");
  });

  await t.test("resetsAt in MILLISECONDS is read as milliseconds", () => {
    const b = availabilityBoundary({ assistantError: null, rateLimit: rejected({ resetsAt: 1789000000000 }) });
    assert.equal(b.until, "2026-09-10T00:26:40.000Z");
  });

  await t.test("an unusable resetsAt yields a boundary with NO instant, never an invented one", () => {
    for (const resetsAt of [undefined, null, 0, -5, NaN, Infinity, "2pm", {}]) {
      const b = availabilityBoundary({ assistantError: "rate_limit", rateLimit: rejected({ resetsAt }) });
      assert.equal(b.until, null, `resetsAt=${String(resetsAt)}`);
    }
  });

  await t.test("the runtime's own sentence is carried as the detail", () => {
    const b = availabilityBoundary({
      assistantError: "rate_limit",
      rateLimit: rejected(),
      resultText: "You've hit your weekly limit · resets 2pm (Europe/Berlin)",
    });
    assert.equal(b.detail, "You've hit your weekly limit · resets 2pm (Europe/Berlin)");
  });

  await t.test("a detail is always a non-empty sentence, even with nothing to quote", () => {
    const b = availabilityBoundary({ assistantError: "overloaded", rateLimit: null });
    assert.ok(b.detail.length > 0);
  });

  // It is persisted and rendered on a status line, so it is bounded like every other piece of
  // SDK-derived text this sidecar keeps.
  await t.test("the detail is capped", () => {
    const b = availabilityBoundary({
      assistantError: "rate_limit",
      rateLimit: null,
      resultText: "x".repeat(MAX_BOUNDARY_DETAIL_CHARS + 500),
    });
    assert.ok(b.detail.length < MAX_BOUNDARY_DETAIL_CHARS + 100);
    assert.ok(b.detail.includes("truncated"));
  });
});
