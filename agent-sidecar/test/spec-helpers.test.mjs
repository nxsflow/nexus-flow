import { test } from "node:test";
import assert from "node:assert/strict";
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
  MAX_RUNTIME_RETRIES,
  MAX_RETRY_DELAY_MS,
  MAX_FAILURE_REPLY_CHARS,
} from "../src/spec-helpers.mjs";

test("resolveModelOption", async (t) => {
  // The engine resolves the declared alias (fable/opus/sonnet) to an SDK model id and puts it in
  // the spec; this side only decides whether to set `options.model` at all. "Not declared" has to
  // stay distinguishable from a declared value, or every role without a `model:` would silently
  // get pinned to whatever default this file happened to name.
  await t.test("model absent (undefined) -> leave unset (undefined)", () => {
    assert.equal(resolveModelOption(undefined), undefined);
  });

  await t.test("model null -> leave unset (undefined)", () => {
    // This is the shape the Rust side actually writes for an undeclared model: JSON `null`.
    assert.equal(resolveModelOption(null), undefined);
  });

  await t.test("model empty string -> leave unset (undefined)", () => {
    // An empty string would be a malformed spec; passing it through would make the SDK reject the
    // session with an opaque model error instead of just running on its default.
    assert.equal(resolveModelOption(""), undefined);
  });

  await t.test("model: 'claude-opus-5' -> stays the exact string", () => {
    assert.equal(resolveModelOption("claude-opus-5"), "claude-opus-5");
  });

  await t.test("a non-string is not passed through", () => {
    // Defensive against a malformed spec: only a non-empty string is a usable model id.
    assert.equal(resolveModelOption(42), undefined);
    assert.equal(resolveModelOption({ id: "claude-opus-5" }), undefined);
  });
});

test("resolveToolsOption", async (t) => {
  await t.test("tools absent (undefined) -> leave unset (undefined)", () => {
    assert.equal(resolveToolsOption(undefined), undefined);
  });

  await t.test("tools null -> leave unset (undefined)", () => {
    assert.equal(resolveToolsOption(null), undefined);
  });

  await t.test("tools: [] -> stays [] (not coerced to unset)", () => {
    const result = resolveToolsOption([]);
    assert.deepEqual(result, []);
    assert.notEqual(result, undefined);
  });

  await t.test('tools: ["Bash"] -> stays ["Bash"]', () => {
    assert.deepEqual(resolveToolsOption(["Bash"]), ["Bash"]);
  });

  await t.test('tools: ["Bash", "Read"] -> stays the exact array', () => {
    assert.deepEqual(resolveToolsOption(["Bash", "Read"]), ["Bash", "Read"]);
  });

  // ---- the engine's grant (nxf 6j6v.kffm) ----------------------------------------------------

  await t.test("an undeclared toolset stays unset even when something is granted", () => {
    // The load-bearing half of the split. The SDK's default set already contains everything a
    // grant can name, so folding the grant in here would turn "the full default toolset" into
    // "exactly Bash" for every role that declares no `tools:` — a strictly worse session than the
    // one this fix exists to repair.
    assert.equal(resolveToolsOption(undefined, ["Bash"]), undefined);
    assert.equal(resolveToolsOption(null, ["Bash"]), undefined);
  });

  await t.test("an explicit zero-tools role gets exactly what was granted", () => {
    // `tools: []` disables every built-in tool, so a role declared that way genuinely cannot run
    // the `nxc reply` its obligation demands. It gets the means for the obligation and nothing
    // else — the narrow scoping nxf 6j6v.04es applied to the synthesizer by hand.
    assert.deepEqual(resolveToolsOption([], ["Bash"]), ["Bash"]);
  });

  await t.test("a declared toolset keeps its own order and gains only what is missing", () => {
    assert.deepEqual(resolveToolsOption(["Read"], ["Bash"]), ["Read", "Bash"]);
    assert.deepEqual(resolveToolsOption(["Bash", "Read"], ["Bash"]), ["Bash", "Read"]);
  });

  await t.test("an older engine sends no grant at all", () => {
    assert.deepEqual(resolveToolsOption(["Read"], undefined), ["Read"]);
  });
});

test("resolveAllowedTools", async (t) => {
  // THE defect nxf 6j6v.kffm is about: `spec.tools ?? []`. A role that declares no `tools:` — the
  // default, and what every role written before the role runtime does — got the SDK's full base
  // toolset and an EMPTY approval list, so every tool call it made was refused with "This command
  // requires approval", its own obligatory `nxc reply` included.
  await t.test("an undeclared toolset approves what was granted", () => {
    assert.deepEqual(resolveAllowedTools(undefined, ["Bash"]), ["Bash"]);
    assert.deepEqual(resolveAllowedTools(null, ["Bash"]), ["Bash"]);
  });

  await t.test("nothing declared and nothing granted approves nothing", () => {
    // The pre-existing behaviour for a trigger that demands nothing: unchanged, to the byte.
    assert.deepEqual(resolveAllowedTools(undefined, undefined), []);
    assert.deepEqual(resolveAllowedTools(undefined, []), []);
  });

  await t.test("the declaration is approved as it always was", () => {
    assert.deepEqual(resolveAllowedTools(["Bash"], undefined), ["Bash"]);
    assert.deepEqual(resolveAllowedTools(["Bash", "Read"], undefined), ["Bash", "Read"]);
  });

  await t.test("declaration and grant are unioned, declaration first, no duplicates", () => {
    assert.deepEqual(resolveAllowedTools(["Read"], ["Bash"]), ["Read", "Bash"]);
    assert.deepEqual(resolveAllowedTools(["Bash"], ["Bash"]), ["Bash"]);
  });

  await t.test("a malformed grant cannot inject a non-string into the list", () => {
    assert.deepEqual(resolveAllowedTools(["Read"], "Bash"), ["Read"]);
    assert.deepEqual(resolveAllowedTools(["Read"], [42, "Bash"]), ["Read", "Bash"]);
  });
});

test("isSubcommandNotImplementedYet", async (t) => {
  await t.test("clap shape with single quotes -> true", () => {
    assert.equal(
      isSubcommandNotImplementedYet(2, "error: unrecognized subcommand 'session'"),
      true,
    );
  });

  await t.test("clap shape with double quotes -> true", () => {
    assert.equal(
      isSubcommandNotImplementedYet(2, 'error: unrecognized subcommand "session"'),
      true,
    );
  });

  await t.test("different exit code, same text -> false", () => {
    assert.equal(
      isSubcommandNotImplementedYet(1, "error: unrecognized subcommand 'session'"),
      false,
    );
  });

  await t.test("same exit code, different text -> false", () => {
    assert.equal(isSubcommandNotImplementedYet(2, "error: some other failure"), false);
  });

  await t.test("status 2 with empty stderr -> false", () => {
    assert.equal(isSubcommandNotImplementedYet(2, ""), false);
  });

  await t.test("status undefined -> false", () => {
    assert.equal(
      isSubcommandNotImplementedYet(undefined, "error: unrecognized subcommand 'session'"),
      false,
    );
  });

  // The transcript flush (0dbp) reuses this check for its own not-yet-existing subcommand, so the
  // name is a parameter — the tolerance must stay narrow to the ONE subcommand being called.
  await t.test("named subcommand: matches the name it was asked about", () => {
    assert.equal(
      isSubcommandNotImplementedYet(2, "error: unrecognized subcommand 'transcript'", "transcript"),
      true,
    );
  });

  await t.test("named subcommand: a DIFFERENT subcommand's error -> false", () => {
    assert.equal(
      isSubcommandNotImplementedYet(2, "error: unrecognized subcommand 'session'", "transcript"),
      false,
    );
    assert.equal(
      isSubcommandNotImplementedYet(2, "error: unrecognized subcommand 'transcript'", "session"),
      false,
    );
  });

  await t.test("named subcommand: double quotes too", () => {
    assert.equal(
      isSubcommandNotImplementedYet(2, 'error: unrecognized subcommand "transcript"', "transcript"),
      true,
    );
  });
});

test("isFlagNotImplementedYet", async (t) => {
  // The teardown's `nxc reply --if-unanswered` (ticket 8) needs the same tolerance
  // `isSubcommandNotImplementedYet` gives `nxc session bind`/`nxc transcript append`, but for a
  // FLAG an older `nxc reply` does not know yet, rather than a whole subcommand: clap's shape for
  // that is "unexpected argument", not "unrecognized subcommand".

  await t.test("clap shape with single quotes -> true", () => {
    assert.equal(
      isFlagNotImplementedYet(2, "error: unexpected argument '--if-unanswered' found", "--if-unanswered"),
      true,
    );
  });

  await t.test("clap shape with double quotes -> true", () => {
    assert.equal(
      isFlagNotImplementedYet(2, 'error: unexpected argument "--if-unanswered" found', "--if-unanswered"),
      true,
    );
  });

  await t.test("different exit code, same text -> false", () => {
    assert.equal(
      isFlagNotImplementedYet(1, "error: unexpected argument '--if-unanswered' found", "--if-unanswered"),
      false,
    );
  });

  await t.test("same exit code, different text -> false", () => {
    assert.equal(isFlagNotImplementedYet(2, "error: some other failure", "--if-unanswered"), false);
  });

  await t.test("status 2 with empty stderr -> false", () => {
    assert.equal(isFlagNotImplementedYet(2, "", "--if-unanswered"), false);
  });

  await t.test("status undefined -> false", () => {
    assert.equal(
      isFlagNotImplementedYet(
        undefined,
        "error: unexpected argument '--if-unanswered' found",
        "--if-unanswered",
      ),
      false,
    );
  });

  // Matched by NAME, same discipline as `isSubcommandNotImplementedYet`: a DIFFERENT unrecognized
  // flag must not be tolerated as if it were this one — that would silently swallow a typo'd or
  // renamed call site instead of reporting it as a failure.
  await t.test("a DIFFERENT flag's error -> false", () => {
    assert.equal(
      isFlagNotImplementedYet(2, "error: unexpected argument '--stream' found", "--if-unanswered"),
      false,
    );
  });
});

test("createFailureTracker", async (t) => {
  await t.test("a fresh tracker has not failed and has nothing to throw", () => {
    const failures = createFailureTracker();
    assert.equal(failures.hasFailed(), false);
    assert.equal(failures.toError(), null);
  });

  await t.test("a recorded Error is the thing that escapes", () => {
    const failures = createFailureTracker();
    const boom = new Error("boom");
    failures.record(boom);
    assert.equal(failures.hasFailed(), true);
    assert.equal(failures.toError(), boom);
  });

  // THE property this exists for: `if (fatal) throw fatal` silently reported success when the
  // stream rejected with a falsy reason (a bare `reject()` on an abort path, `throw null`). A run
  // whose stream died mid-answer was then handed to SidecarWorker as a success.
  await t.test("a falsy failure still marks the run failed", () => {
    for (const falsy of [undefined, null, 0, "", false, NaN]) {
      const failures = createFailureTracker();
      failures.record(falsy);
      assert.equal(failures.hasFailed(), true, `hasFailed() for ${String(falsy)}`);
      const err = failures.toError();
      assert.ok(err, `toError() must be truthy for ${String(falsy)}`);
      assert.ok(err instanceof Error, `toError() must be an Error for ${String(falsy)}`);
      assert.match(err.message, /falsy/);
      assert.match(err.message, new RegExp(String(falsy)));
    }
  });

  await t.test("the FIRST failure is the one that escapes; later ones do not overwrite it", () => {
    const failures = createFailureTracker();
    const first = new Error("the stream died");
    const second = new Error("and then the flush failed");
    failures.record(first);
    failures.record(second);
    assert.equal(failures.toError(), first);
  });

  await t.test("a falsy first failure is not overwritten by a later real Error either", () => {
    // Consistency with the rule above: the falsy rejection IS the root event; the later error was
    // already written to stderr by the caller, so it is not lost.
    const failures = createFailureTracker();
    failures.record(undefined);
    failures.record(new Error("later"));
    assert.equal(failures.hasFailed(), true);
    assert.match(failures.toError().message, /falsy/);
  });

  await t.test("recording nothing at all leaves the run successful", () => {
    const failures = createFailureTracker();
    assert.equal(failures.hasFailed(), false);
    assert.equal(failures.toError(), null);
  });
});

test("capFailureReply", async (t) => {
  // PR #336 review, Integrity & Robustness #1. The automated failure reply is built from an
  // arbitrary SDK-thrown Error and then PERSISTED as a message body other sessions later fold into
  // their own prompts — the same class `MAX_THINKING_CHARS`/`MAX_TOOL_RESULT_CHARS` already bound
  // for the transcript, so it is bounded the same way, with the same overflow marker.
  await t.test("text within the cap is returned verbatim", () => {
    assert.equal(capFailureReply("sidecar: this session ended"), "sidecar: this session ended");
    const exact = "x".repeat(MAX_FAILURE_REPLY_CHARS);
    assert.equal(capFailureReply(exact), exact, "the boundary itself is not truncated");
  });

  await t.test("text past the cap is cut to the cap and says how much was dropped", () => {
    const capped = capFailureReply("y".repeat(MAX_FAILURE_REPLY_CHARS + 500));
    assert.ok(
      capped.startsWith("y".repeat(MAX_FAILURE_REPLY_CHARS)),
      "the head — which carries the diagnosis — is what is kept",
    );
    assert.match(capped, /…\[truncated 500 chars\]$/, "the SAME marker the transcript caps use");
  });

  await t.test("the result is bounded whatever arrives", () => {
    for (const size of [MAX_FAILURE_REPLY_CHARS * 10, 200_000]) {
      const capped = capFailureReply("z".repeat(size));
      assert.ok(
        capped.length < MAX_FAILURE_REPLY_CHARS + 40,
        `a ${size}-char failure must not reach the board at ${capped.length} chars`,
      );
    }
  });
});

test("resolveSettingsSourcesOption", async (t) => {
  await t.test("always returns empty array (SDK isolation mode)", () => {
    assert.deepEqual(resolveSettingsSourcesOption(), []);
  });

  await t.test("returns empty array unconditionally, regardless of spec parameter", () => {
    const result = resolveSettingsSourcesOption({ tools: ["Bash"] });
    assert.deepEqual(result, []);
  });

  await t.test("returned array is not undefined or null", () => {
    const result = resolveSettingsSourcesOption();
    assert.notEqual(result, undefined);
    assert.notEqual(result, null);
    assert.strictEqual(Array.isArray(result), true);
  });
});

// ---- resolveTurnTerms (nxf 6j6v.ntp9) ---------------------------------------------------------

test("resolveTurnTerms", async (t) => {
  await t.test("reads the coordinator's terms off the spec", () => {
    assert.deepEqual(
      resolveTurnTerms({ replyReminders: 2, runtimeRetries: 3, retryBackoffMs: 250 }),
      { replyReminders: 2, runtimeRetries: 3, retryBackoffMs: 250 },
    );
  });

  await t.test("a spec from an older engine reads as the behaviour that existed then", () => {
    // Silence must mean "one reminder, no retry" — never "unbounded", and never "no reminder
    // either", which would silently drop nxf 6j6v.gh7f on a version mismatch.
    assert.deepEqual(resolveTurnTerms({}), {
      replyReminders: 1,
      runtimeRetries: 0,
      retryBackoffMs: 5000,
    });
    assert.deepEqual(resolveTurnTerms(undefined), {
      replyReminders: 1,
      runtimeRetries: 0,
      retryBackoffMs: 5000,
    });
  });

  await t.test("zero is a real bound and is honoured", () => {
    // The one value that must NOT fall through to the fallback: `0` reminders is a host turning the
    // reminder off, and reading it as `1` would spend a model call nobody asked for.
    const terms = resolveTurnTerms({ replyReminders: 0, runtimeRetries: 0 });
    assert.equal(terms.replyReminders, 0);
    assert.equal(terms.runtimeRetries, 0);
  });

  await t.test("a value that is not a bound reads as the fallback, never as unbounded", () => {
    // All THREE fields, including the one whose resolution differs (review of PR #378, Test Quality
    // #8): `retryBackoffMs` carries an extra `|| 5000` clause, so leaving it out of this loop left
    // the one field with its own arm untested against every bad shape.
    for (const bad of [-1, 1.5, "2", null, NaN, Infinity]) {
      const terms = resolveTurnTerms({
        replyReminders: bad,
        runtimeRetries: bad,
        retryBackoffMs: bad,
      });
      assert.equal(terms.replyReminders, 1, `replyReminders for ${String(bad)}`);
      assert.equal(terms.runtimeRetries, 0, `runtimeRetries for ${String(bad)}`);
      assert.equal(terms.retryBackoffMs, 5000, `retryBackoffMs for ${String(bad)}`);
    }
  });

  await t.test("a huge value is clamped, not honoured — the bound is the point", () => {
    // Review of PR #378, Integrity #2. The first cut checked `>= 0` only, so a host (or a corrupted
    // spec file, which sits inside the agent's own cwd) could ask for a spawn loop while holding the
    // working copy. Both ceilings are asserted against the exported constants, because a clamp that
    // silently changed value would otherwise pass.
    const terms = resolveTurnTerms({
      runtimeRetries: 100_000,
      retryBackoffMs: 4_294_967_296,
    });
    assert.equal(terms.runtimeRetries, MAX_RUNTIME_RETRIES);
    assert.equal(terms.retryBackoffMs, MAX_RETRY_DELAY_MS);
  });
});

test("retryDelay", async (t) => {
  await t.test("doubles per attempt already spent", () => {
    const terms = resolveTurnTerms({ retryBackoffMs: 5000 });
    assert.deepEqual(
      [0, 1, 2].map((attempt) => retryDelay(terms, attempt)),
      [5000, 10000, 20000],
    );
  });

  await t.test("is clamped below the ceiling where setTimeout silently collapses", () => {
    // THE reason this function exists rather than an inline `*= 2`: past `2**31 - 1` Node warns
    // `TimeoutOverflowWarning` and sets the delay to 1 ms, so an unclamped doubling backoff stops
    // backing off altogether — the bounded retry becomes a hot spawn loop while the process holds
    // the `working_tree: exclusive` claim. Verified against the real `setTimeout` ceiling, not a
    // number typed here.
    const terms = resolveTurnTerms({ retryBackoffMs: MAX_RETRY_DELAY_MS });
    for (let attempt = 0; attempt <= MAX_RUNTIME_RETRIES; attempt++) {
      const wait = retryDelay(terms, attempt);
      assert.ok(wait <= MAX_RETRY_DELAY_MS, `attempt ${attempt} waits ${wait}ms`);
      assert.ok(wait < 2 ** 31 - 1, `attempt ${attempt} stays under the setTimeout ceiling`);
    }
  });

  await t.test("a zero backoff is refused, because backing off is the point of it", () => {
    // Unlike the two counts, `0` here is not a meaningful setting: it turns a retry into an
    // immediate re-attempt against a runtime that has had no time to recover.
    assert.equal(resolveTurnTerms({ retryBackoffMs: 0 }).retryBackoffMs, 5000);
    assert.equal(resolveTurnTerms({ retryBackoffMs: 1 }).retryBackoffMs, 1);
  });
});
