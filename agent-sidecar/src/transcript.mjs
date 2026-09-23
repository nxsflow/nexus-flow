// Pure SDK-stream → transcript-entry normalizer (nxf 0dbp). The sidecar used to consume the whole
// `query()` stream for its `session_id` alone and drop everything else on the floor; this turns each
// `SDKMessage` into the durable wire entries `nxc transcript append` stores.
//
// Everything here is side-effect-free — no `query()`, no child processes, no clock of its own (the
// clock is injected) — so `test/transcript.test.mjs` exercises the entire wire contract with
// `node --test`, no SDK session required. Same split as `spec-helpers.mjs`: main.mjs owns the I/O.
//
// The wire contract (load-bearing — Task 2's `nxc transcript append` parses exactly this):
//
//   { "kind": …, "at": …, "toolUseId"?: …, "parentToolUseId"?: …, "subagentType"?: …, "data": { } }
//
//   kind  — session_init | assistant | thinking | tool_use | tool_result | result | error
//   at    — ISO-8601 UTC from OUR clock (never Claude Code's own JSONL timestamps)
//   data  — kind-specific payload, opaque to the store
//
// `seq` is deliberately NOT on the wire: the store assigns it, so a `resume` that starts a second
// sidecar process for the same internal session continues the transcript instead of colliding.
//
// Blueprint: beads-dashboard's `sidecar/src/messages.ts` (`forwardMessage`, `subagentTag`). The one
// deliberate divergence: beads streams raw per-token deltas to a live UI, while we persist, so we
// COALESCE each contiguous run of deltas into ONE entry — a row per token would be unreadable and
// would balloon the table.

// `truncate` — the one overflow marker this sidecar emits — lives in `spec-helpers.mjs` beside the
// third cap that shares it (`capFailureReply`, PR #336 review); its doc there says why.
import { truncate } from "./spec-helpers.mjs";

/** Max characters of extended-thinking text kept per coalesced block. Reasoning can run long and is
 *  the least re-read part of a transcript; bound it so one turn cannot dominate the table. */
export const MAX_THINKING_CHARS = 4000;

/** Max characters of a tool result kept per entry (beads `MAX_TOOL_RESULT_CHARS`). A big file read
 *  or grep is unbounded; the head plus a truncation note is what a reader actually needs. */
export const MAX_TOOL_RESULT_CHARS = 8000;

/** Cap a tool result to {@link MAX_TOOL_RESULT_CHARS}. Pure — unit-tested. */
export function capToolResult(text) {
  return truncate(text, MAX_TOOL_RESULT_CHARS);
}

/** Cap a thinking block to {@link MAX_THINKING_CHARS}. Applied to the COALESCED block rather than
 *  to each delta — capping per delta would let a long block through in 20-char slices. */
export function capThinking(text) {
  return truncate(text, MAX_THINKING_CHARS);
}

/** Flatten an Anthropic `tool_result` block's `content` (a string, or an array of content blocks)
 *  into plain text. Non-text blocks (images/documents) are noted by a short `[<type>]` placeholder
 *  rather than dumped verbatim. Anything else yields "". Pure — unit-tested. */
export function stringifyToolResultContent(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content
    .map((block) => {
      if (!block || typeof block !== "object" || !("type" in block)) return "";
      return block.type === "text" ? (block.text ?? "") : `[${block.type}]`;
    })
    .join("");
}

/** Read a message's subagent provenance (beads `subagentTag`). A message the SDK produced inside a
 *  `Task`-spawned subagent carries a non-null `parent_tool_use_id` (the spawning Task tool's id)
 *  and, on assistant/user messages, a `subagent_type`. Returns null for the main conversation.
 *  Pure — unit-tested. */
export function subagentTag(msg) {
  const parentToolUseId = msg?.parent_tool_use_id ?? null;
  if (!parentToolUseId) return null;
  const subagentType = msg.subagent_type;
  return { parentToolUseId, ...(subagentType ? { subagentType } : {}) };
}

/** The two assistant errors an operator fixes by LOGGING IN. Beads `classifyAssistantError`. */
const AUTH_ERRORS = new Set(["authentication_failed", "oauth_org_not_allowed"]);

/** **The two an operator fixes by WAITING** (nxf 6j6v.npy3) — the model is not available to this
 *  session for a while, and nobody did anything wrong.
 *
 *  A NAMED SUBSET, never a catch-all, and the SDK is what makes that possible: `sdk.d.ts` declares
 *  `SDKAssistantMessageError` as a closed union, so this list is a choice among enumerated values
 *  rather than a guess about an open string.
 *
 *  **`billing_error` is deliberately NOT here.** "Out of credits · add funds" is repaired by a
 *  human ACTING, which is what the `auth` family means, and waiting for it changes nothing — so
 *  classifying it as an availability boundary would arm a resume for a boundary that never falls.
 *  `server_error` is not here either: something IS broken there, which is exactly what `runtime`
 *  says. */
const UNAVAILABLE_ERRORS = new Set(["rate_limit", "overloaded"]);

/** Which of the three classes an assistant error belongs to: `auth` (log in), `unavailable` (wait),
 *  or `runtime` (something is broken).
 *
 *  The third class is nxf 6j6v.npy3's, and the whole item exists because there were two: an
 *  exhausted quota landed in "everything else" and was therefore indistinguishable from a crash —
 *  so a session that ended at an availability boundary got an escalation posted in its name saying
 *  "I cannot carry this out", which is false, and the operation stopped. */
function classifyAssistantError(error) {
  if (AUTH_ERRORS.has(error)) return "auth";
  if (UNAVAILABLE_ERRORS.has(error)) return "unavailable";
  return "runtime";
}

/** Max characters of the runtime's own sentence about an availability boundary that is kept.
 *
 *  It is SDK-derived text with no size of its own, and it is PERSISTED — it reaches `nxc status` as
 *  the reason a session is on hold — which is the class `MAX_THINKING_CHARS` and
 *  `MAX_TOOL_RESULT_CHARS` above already bound. Small, because this is one clause on one line and
 *  not content: "You've hit your weekly limit · resets 2pm (Europe/Berlin)" is 56 characters. */
export const MAX_BOUNDARY_DETAIL_CHARS = 200;

/** Below this, a `resetsAt` is read as SECONDS; at or above it, as milliseconds.
 *
 *  `SDKRateLimitInfo.resetsAt` is declared `number` and says no more than that, so the unit has to
 *  be decided here. `1e11` is the one split that cannot be wrong in practice: as seconds it is the
 *  year 5138, and as milliseconds it is 1973 — no reset instant a runtime hands out is on the wrong
 *  side of either. */
const EPOCH_SECONDS_CEILING = 1e11;

/** `resetsAt` as an ISO-8601 UTC instant, or null when it is not a usable one.
 *
 *  NULL RATHER THAN A GUESS, and that is the whole contract of this function: an absent or
 *  nonsensical reset time means the way back is the human's verb, which the operational view can
 *  say. An INVENTED one would arm an automatic resume for a moment the runtime never named. */
function resetInstant(resetsAt) {
  if (typeof resetsAt !== "number" || !Number.isFinite(resetsAt) || resetsAt <= 0) return null;
  const ms = resetsAt < EPOCH_SECONDS_CEILING ? resetsAt * 1000 : resetsAt;
  const at = new Date(ms);
  return Number.isNaN(at.getTime()) ? null : at.toISOString();
}

/**
 * **The availability boundary this run ended at, or null** (nxf 6j6v.npy3).
 *
 * The ONE place that decides whether a finished run is "the model was not available to this session
 * for a while" as opposed to "something is broken" — so the teardown branches on a fact rather than
 * on a re-reading of the stream, and so the decision is testable without an SDK session.
 *
 * **Two structured signals, and EITHER is enough**, because a run can produce one without the
 * other:
 *
 * * `assistantError` — the last `SDKAssistantMessage.error`, which names the CLASS. A member of the
 *   closed union's availability subset ({@link UNAVAILABLE_ERRORS}) is a boundary by itself.
 * * `rateLimit` — the `rate_limit_info` of a REJECTED `SDKRateLimitEvent`, which names the INSTANT
 *   and the window. A rejection is a boundary by itself: it is the runtime saying, in a typed
 *   field, that it refused this session.
 *
 * Neither is prose. That matters more than it looks: the first cut of this item was going to read
 * "resets 2pm (Europe/Berlin)" out of the result text, which needs a timezone database and breaks
 * the day the runtime rewords a sentence.
 *
 * `resultText` is used for the DETAIL only — the sentence a human recognises on a status line — and
 * never to decide anything. Nothing branches on it, so a reworded sentence costs a nicer message
 * and no behaviour.
 *
 * @param {{assistantError?: string|null, rateLimit?: object|null, resultText?: string|null}} seen
 * @returns {{limit: string, until: string|null, detail: string} | null}
 */
export function availabilityBoundary({ assistantError, rateLimit, resultText } = {}) {
  const rejected = rateLimit?.status === "rejected" ? rateLimit : null;
  const named = UNAVAILABLE_ERRORS.has(assistantError) ? assistantError : null;
  if (!rejected && !named) return null;
  return {
    // WHICH window, when the runtime says — `seven_day` is a different wait from `five_hour`, and
    // a human reading "on hold" wants to know which. The error name is the fallback, and it is
    // still an enumerated value rather than free text.
    limit: rejected?.rateLimitType ?? named ?? "rate_limit",
    until: resetInstant(rejected?.resetsAt),
    detail: truncate(
      typeof resultText === "string" && resultText.trim() !== ""
        ? resultText.trim()
        : "the model was not available to this session",
      MAX_BOUNDARY_DETAIL_CHARS,
    ),
  };
}

/**
 * Build a stateful normalizer over one SDK stream.
 *
 * @param {{ now?: () => string }} [options] - `now` is the injected clock returning an ISO-8601 UTC
 *   string; tests pass a deterministic stub. Defaults to the real clock so main.mjs can call
 *   `createNormalizer()` with no argument.
 * @returns {{ push: (msg: unknown) => object[], flush: () => object[], rateLimit: () => object|null }}
 *   `push` feeds one `SDKMessage` and returns zero or more COMPLETED entries (an in-progress
 *   coalescing block is held back until it closes); `flush` closes any open block at stream end;
 *   `rateLimit` is the availability boundary this stream ran into, or null (nxf 6j6v.npy3).
 */
export function createNormalizer({ now = () => new Date().toISOString() } = {}) {
  // The currently-accumulating delta block, or null: `{ kind, text, at, tag }`. `at` is stamped
  // when the block OPENS — a streamed answer belongs to the moment the model started saying it,
  // not to whenever the closing message happened to arrive.
  let open = null;
  // Which provenances streamed text via partial deltas during the CURRENT TURN? If one did, that
  // conversation's final `assistant` message `text` blocks duplicate what we already coalesced and
  // must be dropped (beads `sawDelta`). If it did not (partials off, or a provider that doesn't
  // stream), the final text is the only copy of the answer.
  //
  // Per TURN *and* per PROVENANCE — a Set of `parentToolUseId` (null = main conversation) rather
  // than beads' single boolean. With `forwardSubagentText` on, a subagent's deltas interleave with
  // the main conversation's in one stream; a global flag would let a subagent's streaming suppress
  // the MAIN conversation's fallback text block (and vice versa), silently swallowing an answer
  // that was never streamed. Cleared on every `result`, i.e. per turn. Thinking deltas do NOT
  // record anything here — a different block kind never duplicates a `text` block.
  const sawDelta = new Set();

  /** **The availability boundary this stream ran into** (nxf 6j6v.npy3) — the `rate_limit_info` of
   *  the first `SDKRateLimitEvent` that came back `rejected`, or null.
   *
   *  It is REMEMBERED rather than turned into an entry, and both halves of that are deliberate.
   *
   *  Remembered, because it is what answers *when does this lift* — `resetsAt` is a number the
   *  runtime states, so the engine never has to read "resets 2pm (Europe/Berlin)" out of a
   *  sentence. Before this, that fact reached the process and was dropped on the floor with every
   *  other message type the `default:` arm ignores, and the whole of nxf 6j6v.npy3's measured gap
   *  is that nothing downstream could say a limit had been hit at all.
   *
   *  `rejected` only, because `allowed` / `allowed_warning` fire ROUTINELY — the SDK emits one
   *  whenever utilization changes — so latching every event would report an availability boundary
   *  on every healthy run. And FIRST-wins rather than last: an `allowed` arriving afterwards (the
   *  window lifting mid-stream, a second event for another window) must not erase the rejection
   *  that ended this turn.
   *
   *  **First-wins also against a SECOND rejection, and that is a decision rather than an
   *  oversight** (independent review of PR #476, Code Quality #4). Two rejections in one attempt —
   *  a five-hour window and then a seven-day one — would name two different waits, and the later,
   *  longer one is arguably the binding one. Taking the first costs at most one early return: the
   *  resume fires at the nearer instant, the runtime refuses it again, and that refusal records a
   *  fresh hold with the longer window and arms a new return. The mistake is self-correcting and
   *  costs a refused start; reaching for the maximum instead would mean ranking window kinds here,
   *  which is the runtime's vocabulary and not this file's.
   *
   *  It is scoped to ONE ATTEMPT — `forgetRateLimit` clears it at the top of each retry — so the
   *  window in which any of this can happen is a single turn.
   *
   *  Read defensively for `session_init`'s reason: these fields are declared on a wire we do not
   *  control, and a shape that is not an object with `status: "rejected"` is no evidence of
   *  anything. */
  let rejectedRateLimit = null;

  const entry = (kind, data, extra) => ({ kind, at: now(), ...extra, data });

  /** Close the open block and return its entry, or [] when there is none / it accumulated nothing
   *  (an empty block is not worth a row). */
  const closeOpen = () => {
    if (!open) return [];
    const { kind, text, at, tag } = open;
    open = null;
    if (text === "") return [];
    return [{ kind, at, ...(tag ?? {}), data: { text: kind === "thinking" ? capThinking(text) : text } }];
  };

  /** Append a delta to the open block, closing it first when the incoming delta belongs to a
   *  different block: a different kind (thinking vs text), or a different provenance. The
   *  provenance check is what keeps a subagent's streamed text from being glued onto the main
   *  conversation's — the two interleave in one stream. `parent_tool_use_id` is the whole
   *  provenance here: a partial assistant message carries no `subagent_type` (sdk.d.ts). */
  const appendDelta = (kind, text, tag) => {
    const sameParent = (open?.tag?.parentToolUseId ?? null) === (tag?.parentToolUseId ?? null);
    const closed = open && (open.kind !== kind || !sameParent) ? closeOpen() : [];
    if (!open) open = { kind, text: "", at: now(), tag };
    open.text += text;
    return closed;
  };

  return {
    push(msg) {
      const tag = subagentTag(msg);
      switch (msg?.type) {
        case "system": {
          if (msg.subtype !== "init") return [];
          // Counts, not the lists themselves: the transcript records what the session was
          // configured with, and the full arrays would dwarf the entry. Read defensively — these
          // fields are declared non-optional but come off a wire we do not control.
          return [
            ...closeOpen(),
            entry("session_init", {
              sdkSessionId: msg.session_id,
              model: msg.model,
              apiKeySource: msg.apiKeySource,
              tools: msg.tools?.length ?? 0,
              skills: msg.skills?.length ?? 0,
              slashCommands: msg.slash_commands?.length ?? 0,
              permissionMode: msg.permissionMode,
            }),
          ];
        }
        case "stream_event": {
          // Token-level streaming off the partial assistant message (`includePartialMessages`).
          // Extended thinking is taken from here and ONLY here. The completed `assistant` message
          // does also carry a `thinking` block with the very same text — measured live on the
          // pinned SDK, 829 chars of deltas against 829 chars of block, character for character
          // (nxf 6j6v.w4wa) — so it is not that there is nothing else to read: the `assistant` arm
          // below deliberately does not read it, which is what keeps thinking from being counted
          // twice. Do not "fix" that arm by handling `thinking` blocks there too.
          const ev = msg.event;
          // A content block ENDING closes the coalescing run (PR #258 review, Code Quality #1).
          // Without this, coalescing only ever breaks on a kind or provenance change, so two
          // SEPARATE same-kind blocks in one message — two thinking blocks around a tool call, two
          // text blocks — would silently fuse into a single entry with the first block's timestamp.
          // Anthropic streaming always brackets deltas with content_block_start/stop, so keying the
          // boundary on the block's own end makes an entry exactly one content block rather than
          // resting on the assumption that same-kind blocks are never adjacent.
          if (ev?.type === "content_block_stop") return closeOpen();
          if (ev?.type !== "content_block_delta") return [];
          if (ev.delta?.type === "text_delta") {
            sawDelta.add(tag?.parentToolUseId ?? null);
            return appendDelta("assistant", ev.delta.text, tag);
          }
          if (ev.delta?.type === "thinking_delta") return appendDelta("thinking", ev.delta.thinking, tag);
          return [];
        }
        case "assistant": {
          // An auth/billing failure surfaces as an error on the assistant message. Session-level,
          // so it is never subagent-tagged.
          if (msg.error) {
            return [
              ...closeOpen(),
              entry("error", { kind: classifyAssistantError(msg.error), message: msg.error }),
            ];
          }
          const content = msg.message?.content;
          if (!Array.isArray(content)) return [];
          const entries = [];
          for (const block of content) {
            if (block?.type === "tool_use") {
              entries.push(entry("tool_use", { name: block.name, input: block.input }, { toolUseId: block.id, ...tag }));
              // Dedup is scoped to THIS message's own provenance: a subagent streaming its reply
              // says nothing about whether the main conversation streamed its own.
            } else if (block?.type === "text" && !sawDelta.has(tag?.parentToolUseId ?? null)) {
              // Fallback path only: partials never arrived this turn, so this is the sole copy of
              // the answer. Not capped — it IS the answer.
              entries.push(entry("assistant", { text: block.text }, tag));
            }
          }
          // Producing an entry closes the open block; producing none (every text block deduped)
          // leaves it open for the next event or flush().
          return entries.length > 0 ? [...closeOpen(), ...entries] : [];
        }
        case "user": {
          // REPLAYED history is not transcript. `SDKUserMessageReplay` carries `isReplay: true`
          // (sdk.d.ts:4601) — a required literal the live `SDKUserMessage` does not have — so the
          // two ARE distinguishable, and this one-line guard is the defense: without it, a resume
          // that replays the conversation would re-append every historical `tool_result` as a
          // fresh entry, duplicating the whole prior transcript into the store on every resume.
          // (Secondary: beads established on this same topology — beads issue 34w — that `resume`
          // does not in fact replay, which is why we do not ALSO try to reconstruct history from a
          // replay. The guard does not depend on that holding.)
          if (msg.isReplay) return [];
          // The SDK injects each tool's RESULT as a `user` message carrying `tool_result` blocks.
          // A real user turn (our own prompt) has none and emits nothing.
          const content = msg.message?.content;
          if (!Array.isArray(content)) return [];
          const entries = [];
          for (const block of content) {
            if (block?.type !== "tool_result") continue;
            entries.push(
              entry(
                "tool_result",
                {
                  content: capToolResult(stringifyToolResultContent(block.content)),
                  isError: block.is_error === true,
                },
                { toolUseId: block.tool_use_id, ...tag },
              ),
            );
          }
          return entries.length > 0 ? [...closeOpen(), ...entries] : [];
        }
        case "result": {
          const closed = closeOpen();
          sawDelta.clear(); // a turn ended; the next one starts with no partials seen anywhere
          return [
            ...closed,
            entry("result", {
              subtype: msg.subtype,
              isError: msg.is_error,
              totalCostUsd: msg.total_cost_usd,
              numTurns: msg.num_turns,
              // SDKResultError carries no `result`; omit the key entirely rather than write null.
              ...("result" in msg ? { resultText: msg.result } : {}),
            }),
          ];
        }
        case "rate_limit_event": {
          // NO entry of its own, and it does NOT close an open block — it is the runtime talking
          // about the account, not the model producing output, so it can arrive between two deltas
          // of one contiguous run exactly like the types the `default:` arm ignores. What the
          // transcript records about this turn is the `error` entry above, classified
          // `unavailable`; what this arm is for is the INSTANT, which no entry carries.
          const info = msg.rate_limit_info;
          if (!rejectedRateLimit && info && typeof info === "object" && info.status === "rejected") {
            rejectedRateLimit = info;
          }
          return [];
        }
        default:
          // Status, hooks, tool progress, compact boundaries, … — not part of the transcript. They
          // do NOT close an open block: they can arrive mid-stream between two deltas of the same
          // contiguous run.
          return [];
      }
    },

    flush() {
      return closeOpen();
    },

    /** The availability boundary this stream ran into, or null — see `rejectedRateLimit`. */
    rateLimit() {
      return rejectedRateLimit;
    },

    /** **Forget it, for the next ATTEMPT of the same turn** (independent review of PR #476,
     *  Integrity #3).
     *
     *  One normalizer spans every attempt of a run — deliberately, because the transcript of a turn
     *  is one transcript. The latch is not like that: it says what ended THIS attempt, and a
     *  rejection carried over from an abandoned earlier attempt would let a later, unrelated
     *  failure be reported as an availability boundary. Called only by the retry loop, at the top
     *  of each attempt; nothing else has any business clearing it. */
    forgetRateLimit() {
      rejectedRateLimit = null;
    },
  };
}
