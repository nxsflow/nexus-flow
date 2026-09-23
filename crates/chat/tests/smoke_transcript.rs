//! The live SDK smoke for the agent-transcript epic (nxf epic 6wt2, ticket 5bym — the epic's own
//! DoD: "a live run whose transcript, INCLUDING at least one subagent, is observed round-tripping
//! capture → store → read"). Every other test in this epic is deterministic and offline: T1's
//! normalizer is pure, T2's store is SQL, T3's read is in-memory assembly. This file is the one
//! place where a real Claude Agent SDK session (`NXC_WORKER=sidecar`) actually produces the stream
//! — real subscription usage, real wall-clock time (tens of seconds to a few minutes). `#[ignore]`d
//! so `cargo test --all` never runs it; run it by hand with:
//!
//!   cargo test -p nexus-chat --test smoke_transcript -- --ignored --nocapture --test-threads=1
//!
//! `--nocapture` matters here: the test prints the human `nxc transcript show` timeline of the live
//! run at the end, which is the artifact worth reading.
//!
//! The harness is `tests/smoke_v3.rs`'s, deliberately re-derived rather than reinvented — read that
//! file's module doc for the full rationale of each piece. In short: `target/debug` is prepended to
//! `PATH` so this test's own `nxc`/`nxs` calls AND every nested in-session `nxc` call the spawned
//! role makes (including the sidecar's own `nxc transcript append` callback — the whole point here)
//! resolve the freshly built binary; `NXC_WORKER=sidecar` + `NXC_SIDECAR=<repo>/agent-sidecar/src/
//! main.mjs` wire the real sidecar; there is deliberately **no `ANTHROPIC_API_KEY`** (this machine
//! authenticates through an existing `claude` CLI session under `~/.claude/`, and a placeholder key
//! would make the SDK prefer it and break everything); the workspace is a fresh `tempfile::TempDir`,
//! never this repo; and `poll_until` is how every stage waits, because every trigger in this
//! codebase is fire-and-forget with no "wait for the session" call anywhere.
//!
//! What the scenario proves that no deterministic test can: that the three halves agree about a
//! stream **nobody wrote by hand** — the sidecar tagged a real Task-spawned subagent's entries with
//! the spawning `tool_use` id (T1), the store kept them as first-class rows rather than dropping
//! them the way the beads blueprint did (T2), and the read nested them back under that `tool_use`
//! (T3). That is exactly the gap this epic exists to close, and a synthetic fixture can only ever
//! prove that the three agree with the fixture.
//!
//! ## What this smoke does NOT assert: `thinking` (nxf `w4wa`, closed)
//!
//! This test used to end on `assert!(kinds.contains(&"thinking"))` and was softened when a probe on
//! 2026-07-29 found `thinking_delta` carrying nothing over this repo's auth path (ambient `claude`
//! CLI session under `~/.claude/`, `apiKeySource: "none"`, deliberately no `ANTHROPIC_API_KEY` —
//! see above). That probe's CONCLUSION was wrong and is corrected here: it read the frames as never
//! delivered and `Options` as having no knob for it, and recorded "re-probe on an SDK bump" as the
//! way out. Re-probed on 2026-08-11 on the SAME pinned `@anthropic-ai/claude-agent-sdk` 0.3.215 —
//! no bump — one reasoning-heavy prompt, everything else exactly as the sidecar ships:
//!
//!   (as shipped then: no `thinking` option)                     ->  thinking=0 chars, text=3666
//!   thinking: { type: 'adaptive', display: 'summarized' }        ->  thinking=829 chars, text=4086
//!   thinking: { type: 'enabled', budgetTokens: 4096, display }   ->  thinking=1078 chars, text=3156
//!
//! The frames were always delivered — they were EMPTY, the blocks redacted to zero characters,
//! which reads identically to "never emitted" if you only count characters. And the knob is on
//! `Options` after all: not as `thinkingDisplay`, but as `display` inside `ThinkingConfig`
//! (`sdk.d.ts` `ThinkingAdaptive`/`ThinkingEnabled`). The `setMaxThinkingTokens(N, display)` control
//! request the old note pointed at could not have been the answer anyway — control requests are
//! "only available in streaming input mode" and the sidecar passes a plain string prompt, and the
//! method is deprecated in favour of exactly this option.
//!
//! `main.mjs` now ships `thinking: { type: 'adaptive', display: 'summarized' }`, and a live role
//! session records real `thinking` entries (verified 2026-08-11: one entry, 921 bytes, alongside
//! assistant/tool_use/tool_result/result). The capture path (`transcript.mjs`, `capThinking`) needed
//! no change — it was correct all along, just never fed.
//!
//! It stays a non-fatal observation here for a DIFFERENT reason than before, and a weaker one: with
//! `adaptive`, whether a turn thinks at all is the model's call, and this smoke's question ("which
//! deployment target does this workspace document?") is a lookup, not a puzzle. A hard assert would
//! red an otherwise-perfect run over a judgement the model is entitled to make. Absence is now a
//! statement about the RUN, not about the platform — and whenever thinking does appear, the
//! well-formedness assertions below are real.

use nxs_test_support::Command;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ---- environment plumbing (smoke_v3.rs's, verbatim in shape) -------------------------------------

/// The workspace root, fixed at compile time from THIS crate's manifest dir (`<root>/crates/chat`),
/// so it doesn't depend on the test's cwd.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/chat sits at <root>/crates/chat")
        .to_path_buf()
}

/// Build a `Command` for `bin` (`"nxc"` or `"nxs"`) wired for a LIVE role-runtime run — see the
/// module doc for why each piece is set the way it is. Goes through `nxs_test_support::cargo_bin`
/// (not a bare `assert_cmd::Command::cargo_bin`) so the stale-multicall-binary gate still runs.
fn cmd(bin: &str, ws: &Path) -> Command {
    let path = format!(
        "{}:{}",
        repo_root().join("target/debug").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut c = nxs_test_support::cargo_bin(bin);
    c.current_dir(ws)
        .env("PATH", path)
        .env("NXC_WORKER", "sidecar")
        .env(
            "NXC_SIDECAR",
            repo_root().join("agent-sidecar/src/main.mjs"),
        );
    c
}

fn nxc(ws: &Path) -> Command {
    cmd("nxc", ws)
}

fn nxs(ws: &Path) -> Command {
    cmd("nxs", ws)
}

fn json_of(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or_else(|e| {
        panic!(
            "expected JSON, got {:?}: {e}",
            String::from_utf8_lossy(bytes)
        )
    })
}

/// `nxs init --json --module flow --module chat` (no `--module` flags at all defaults to `flow`
/// ONLY — always name both explicitly).
fn nxs_init(ws: &Path) {
    let out = nxs(ws)
        .args(["init", "--json", "--module", "flow", "--module", "chat"])
        .output()
        .expect("run nxs init");
    assert!(
        out.status.success(),
        "nxs init failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Poll `try_once` every `interval` until it returns `Some`, or give up (returning `None`) once
/// `bound` has elapsed. A summon is fire-and-forget — there is no synchronous "wait for the
/// real session to finish" call anywhere — so this is what the one stage below waits on.
fn poll_until<T>(
    bound: Duration,
    interval: Duration,
    mut try_once: impl FnMut() -> Option<T>,
) -> Option<T> {
    let start = Instant::now();
    loop {
        if let Some(v) = try_once() {
            return Some(v);
        }
        if start.elapsed() >= bound {
            return None;
        }
        std::thread::sleep(interval);
    }
}

/// A diagnostic dump of every session this run spawned (role, declared tools, resume target, and
/// its sidecar log) — read directly off `.nxs/agent-logs/`. Used ONLY inside a `panic!` on timeout,
/// so a failing smoke shows its actual history rather than a bare "timed out".
fn dump_agent_logs(ws: &Path) -> String {
    let dir = ws.join(".nxs/agent-logs");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return format!("(no {} dir)", dir.display());
    };
    let mut sessions: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".spec.json")
                .map(str::to_string)
        })
        .collect();
    sessions.sort();
    let mut out = String::new();
    for session in sessions {
        let spec_raw =
            std::fs::read_to_string(dir.join(format!("{session}.spec.json"))).unwrap_or_default();
        let spec: Value = serde_json::from_str(&spec_raw).unwrap_or_default();
        let log = std::fs::read_to_string(dir.join(format!("{session}.log"))).unwrap_or_default();
        out.push_str(&format!(
            "  session {session}: role={:?} tools={:?} resume={:?}\n    log: {}\n",
            spec.get("role"),
            spec.get("tools"),
            spec.get("resume"),
            log.lines().collect::<Vec<_>>().join(" | ")
        ));
    }
    out
}

// ---- the scenario --------------------------------------------------------------------------------

/// The role: one turn, whose whole job is to delegate to a Task-spawned subagent and report back.
/// `Task` is what makes a subagent exist at all; `Read`/`Glob` give the subagent something real to
/// do inside the scratch workspace (a tool call of its own is what makes its sub-timeline more than
/// one message long). `Write` earns its place by forcing a SECOND main-conversation `tool_use` after
/// the Task returns — so the transcript proves the nesting boundary rather than merely the nesting:
/// a main-conversation call that follows a subagent's own calls must stay at top level. Its
/// `answer.txt` is also reported in the timeout diagnostic below (the one place anything reads it).
///
/// The prompt names the delegation EXPLICITLY (and `general-purpose`, the built-in subagent type —
/// the sidecar runs with `settingSources: []`, so no project-declared `.claude/agents` exist to
/// name instead). Real model behavior is never guaranteed, but "use the Task tool" is about as
/// deterministic as a live prompt gets, and a run where the model refuses to delegate fails LOUDLY
/// below with the observed entry kinds rather than silently proving nothing.
const DELEGATOR_ROLE_YAML: &str = "\
handle: delegator
system_prompt: |
  You are a coordinator. You do NOT read files yourself.

  When you are given a question about this workspace, you MUST answer it by delegating: call the
  Task tool exactly once with subagent_type \"general-purpose\", handing the subagent the question
  and telling it to read whatever files it needs and report back in one sentence. Then write the
  subagent's answer, verbatim, into a new file `answer.txt` using the Write tool, and stop.
tools: [Task, Read, Glob, Write]
";

#[test]
#[ignore = "spawns a real Claude Agent SDK session (NXC_WORKER=sidecar) -- real cost/time, run manually"]
fn a_live_role_session_records_a_transcript_with_a_nested_subagent_sub_timeline() {
    let ws = TempDir::new().expect("scratch workspace");
    let root = ws.path();

    nxs_init(root);
    let roles = root.join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(roles.join("delegator.yaml"), DELEGATOR_ROLE_YAML).unwrap();
    // Something real for the subagent to go and read, inside the scratch workspace only.
    std::fs::write(
        root.join("NOTES.md"),
        "# Project notes\n\nThe deployment target is `staging-eu-3`.\n",
    )
    .unwrap();

    // The internal session id comes straight back from the trigger: `send --to <persona>` mints
    // it, registers it pending, and returns it as `session` — the SAME id the sidecar keys its
    // `nxc transcript append` calls by, and the id `transcript show` is read with.
    let out = nxc(root)
        .args([
            "--json",
            "send",
            "--to",
            "delegator",
            "Which deployment target does this workspace document?",
        ])
        .output()
        .expect("send --to delegator");
    assert!(
        out.status.success(),
        "send --to delegator failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let session = json_of(&out.stdout)["session"]
        .as_str()
        .expect("a persona summon returns the minted internal session id")
        .to_string();

    // Wait for the SESSION TO BIND, not merely for the turn's `result` entry — both, in one gate.
    // The two commit at different times and in this order: a `result` triggers the sidecar's
    // MID-RUN flush from inside `for await` (`main.mjs:166-170`), while `nxc session bind` runs
    // afterwards in teardown (`:193-197`). A poll that stopped at `result` would sometimes land in
    // the gap between them — SDK generator teardown plus one debug-build `nxc` process, a fraction
    // of a second to a second and a half — and then fail the `real_sdk_id` assertion below on an
    // otherwise perfect run. That is a harness race, and this smoke costs real API budget per run,
    // so it must not exist. (`role` is never racy: `create_pending_session` runs at `send` time.)
    //
    // Everything after this poll is a plain assertion, so a run that DID complete but produced the
    // wrong shape fails naming what it actually recorded, instead of timing out opaquely.
    let bound = Duration::from_secs(300);
    let view = poll_until(bound, Duration::from_secs(5), || {
        let out = nxc(root)
            .args(["--json", "transcript", "show", &session])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v = json_of(&out.stdout);
        let done = v["entries"]
            .as_array()?
            .iter()
            .any(|e| e["kind"] == "result");
        (done && v["real_sdk_id"].is_string()).then_some(v)
    });
    let Some(view) = view else {
        panic!(
            "session {session} never reached `result` + a bound real SDK id within {bound:?} — the \
             live turn never completed, nothing was ever appended, or `nxc session bind` failed.\n\
             answer.txt: {}\n\
             last transcript: {:?}\n\
             agent-logs:\n{}",
            // The role's own last instruction, and the cheapest signal for "did the top-level turn
            // get all the way through": present ⇒ the model finished and the failure is in the
            // capture/store/read path; absent ⇒ the turn itself never got that far.
            match std::fs::read_to_string(root.join("answer.txt")) {
                Ok(s) => format!("{:?}", s.trim()),
                Err(e) => format!("(absent: {e})"),
            },
            nxc(root)
                .args(["--json", "transcript", "show", &session])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string()),
            dump_agent_logs(root)
        );
    };

    // The session-map join: the read resolves the role it was minted under and the REAL SDK session
    // id the sidecar bound (which is what a `--resume` and `messages.refs.session_id` name). The
    // `real_sdk_id` half is the poll's own gate, restated here as the assertion it is.
    assert_eq!(view["session"], session.as_str());
    assert_eq!(view["role"], "delegator");
    assert!(
        view["real_sdk_id"].is_string(),
        "the sidecar's `nxc session bind` callback ran: {view:#?}"
    );

    // Every entry, flattened in render order, with the depth it was served at — the whole tree is
    // one level deep by contract, so this is a complete walk. (Non-empty already: the poll gate
    // above only fires on a transcript that CONTAINS a `result` entry.)
    let entries = view["entries"].as_array().expect("entries array");
    let mut flat: Vec<(&Value, usize)> = Vec::new();
    for e in entries {
        flat.push((e, 0));
        for sub in e["subagent"]
            .as_array()
            .expect("subagent is always an array")
        {
            flat.push((sub, 1));
            assert_eq!(
                sub["subagent"].as_array().map(Vec::len),
                Some(0),
                "nesting is one level deep by contract: {sub:#?}"
            );
        }
    }
    let kinds: Vec<&str> = flat
        .iter()
        .map(|(e, _)| e["kind"].as_str().unwrap_or("?"))
        .collect();

    // `seq` is the store's own monotonic counter, so the whole flattened tree — main conversation
    // and every nested sub-timeline — reads back strictly increasing, with no id reused or lost.
    let seqs: Vec<i64> = flat
        .iter()
        .map(|(e, _)| e["seq"].as_i64().expect("seq is an integer"))
        .collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        seqs, sorted,
        "entries render in strictly increasing, unique seq order: {seqs:?} ({kinds:?})"
    );

    // THE EPIC'S POINT, asserted FIRST — deliberately ahead of the `thinking` check below. Whether
    // the model produced extended reasoning at all is the least controllable requirement in this
    // file, and a run that skipped it must still report whether the epic's actual property held; if
    // this came second, such a run would abort before ever telling us.
    //
    // A real Task-spawned subagent's own entries came back nested under the `tool_use` that spawned
    // them: capture tagged them, the store kept them (beads dropped them here), the read reassembled
    // the tree.
    let with_subagent: Vec<&Value> = entries
        .iter()
        .filter(|e| !e["subagent"].as_array().map(Vec::is_empty).unwrap_or(true))
        .collect();
    assert!(
        !with_subagent.is_empty(),
        "no entry carries a nested sub-timeline — the role never delegated, or subagent tagging \
         regressed. Observed kinds: {kinds:?}\nagent-logs:\n{}",
        dump_agent_logs(root)
    );
    for parent in &with_subagent {
        assert_eq!(
            parent["kind"], "tool_use",
            "only a tool_use can parent a sub-timeline: {parent:#?}"
        );
        let sub = parent["subagent"].as_array().unwrap();
        assert!(
            sub.iter().any(|e| e["subagent_type"].is_string()),
            "the subagent's entries keep the type the SDK reported: {sub:#?}"
        );
    }

    // The artifact worth reading: the human timeline of the live run (run with `--nocapture`).
    // Printed BEFORE the last, model-dependent assertion so a run that produced no extended
    // reasoning still shows its timeline instead of only a panic message.
    //
    // Two different reads, so both are attributed rather than mixed: this `show` is FRESH (its own
    // header line carries its own totals, and it may include a teardown flush the polled snapshot
    // predates), while the counts named here are the snapshot every assertion above ran on.
    let human = nxc(root)
        .args(["transcript", "show", &session])
        .output()
        .expect("transcript show");
    println!(
        "\n---- live transcript, re-read now (asserted on a snapshot of {} entries, {} of them \
         nested) ----\n{}",
        flat.len(),
        flat.iter().filter(|(_, depth)| *depth == 1).count(),
        String::from_utf8_lossy(&human.stdout)
    );

    // Extended THINKING reaches the transcript only when the sidecar asks for BOTH
    // `includePartialMessages` and a thinking display mode (nxf `w4wa`, closed — it now ships
    // `thinking: { type: 'adaptive', display: 'summarized' }`). Its presence is still NOT asserted,
    // because `adaptive` leaves "did this turn think at all" to the model and this smoke asks a
    // lookup question — see this file's module doc for the probe numbers. LAST, so nothing else in
    // the file depends on it. Both flags THEMSELVES are pinned deterministically, without a live
    // run, by `agent-sidecar/test/main-teardown.test.mjs` (which asserts the exact `options`
    // main.mjs hands `query()`), so the wiring is covered regardless of what this reports.
    let thinking: Vec<&Value> = flat
        .iter()
        .map(|(e, _)| *e)
        .filter(|e| e["kind"] == "thinking")
        .collect();
    if thinking.is_empty() {
        // NOT a passing check — an unverified requirement, printed so `--nocapture` cannot hide it.
        println!(
            "\n\
             ################################################################################\n\
             ##  NOT VERIFIED THIS RUN: no `thinking` entry was recorded.                  ##\n\
             ##  This is NOT a pass. The assertion is suppressed, not satisfied.           ##\n\
             ##                                                                            ##\n\
             ##  This is no longer a platform gap: nxf `w4wa` is closed and the sidecar    ##\n\
             ##  ships thinking adaptive + display summarized, which does put real         ##\n\
             ##  summarized reasoning into the transcript (verified live).                 ##\n\
             ##  `adaptive` leaves it to the MODEL whether a turn thinks at all, and this  ##\n\
             ##  smoke asks a lookup question -- so an empty run is a statement about THIS ##\n\
             ##  run, not about the platform, and a hard assert would red perfect runs.    ##\n\
             ##                                                                            ##\n\
             ##  If this is empty run after run, suspect the WIRING, not the model: check  ##\n\
             ##  that `options.thinking` still reaches query() (pinned by                  ##\n\
             ##  agent-sidecar/test/main-teardown.test.mjs). Read the module doc first.    ##\n\
             ################################################################################\n\
             Observed kinds this run: {kinds:?}\n"
        );
    } else {
        // The moment the SDK does start emitting, this becomes a real test of thinking rather than
        // a mere presence check: every entry carries text, and it honours the ONE property the
        // producer guarantees about it — `capThinking` (`agent-sidecar/src/transcript.mjs:27,45-49`)
        // caps the COALESCED block at `MAX_THINKING_CHARS` and, only on overflow, appends exactly one
        // `\n…[truncated N chars]` marker. Split on the marker rather than measuring the whole
        // string: the cap is applied in JS (UTF-16 units) and read here in Rust `char`s (scalar
        // values), which disagree on astral input — the head bound plus the marker's exact shape is
        // the part that is genuinely invariant.
        const MAX_THINKING_CHARS: usize = 4000;
        const TRUNCATION_MARKER: &str = "\n…[truncated ";
        for e in &thinking {
            let text = e["data"]["text"].as_str().unwrap_or_else(|| {
                panic!("a thinking entry carries `data.text` as a string: {e:#?}")
            });
            assert!(
                !text.is_empty(),
                "a thinking entry's `data.text` is never empty — an empty one means the coalescer \
                 flushed a block it never filled: {e:#?}"
            );
            let (head, marker) = match text.split_once(TRUNCATION_MARKER) {
                Some((head, rest)) => (head, Some(rest)),
                None => (text, None),
            };
            assert!(
                head.chars().count() <= MAX_THINKING_CHARS,
                "thinking text exceeds MAX_THINKING_CHARS ({MAX_THINKING_CHARS}) before any \
                 truncation marker ({} chars) — `capThinking` did not run on the coalesced block: \
                 {e:#?}",
                head.chars().count()
            );
            if let Some(rest) = marker {
                let dropped = rest.strip_suffix(" chars]");
                assert!(
                    dropped.is_some_and(|n| n.parse::<u64>().is_ok()),
                    "a truncated thinking block ends with exactly one `{TRUNCATION_MARKER}N chars]` \
                     marker naming the dropped count: {e:#?}"
                );
            }
        }
        println!(
            "\n---- `thinking` OBSERVED LIVE ({} entries, well-formedness asserted) — the SDK gap \
             tracked as nxf `w4wa` may be closed; re-probe and consider restoring the hard assert \
             ----\n",
            thinking.len()
        );
    }
}

// ---- the model-selection smoke (nxf 6j6v.ep7j) ---------------------------------------------------

/// Two roles that differ in exactly one thing: the model they declare. Nothing else about them —
/// same prompt, same (empty) toolset, same one-word answer — so the only variable between the two
/// live sessions below is `model:`.
///
/// `tools: []` is deliberate and is the whole reason this smoke is cheap: an explicit zero-tools
/// declaration disables the SDK's entire base toolset (see `worker.rs`'s doc on `RoleSpec.tools`),
/// so each session is one model turn with no tool loop at all.
fn one_word_role_yaml(handle: &str, model: &str) -> String {
    format!(
        "handle: {handle}\n\
         system_prompt: |\n  \
         Answer with exactly one word: ok. Nothing else.\n\
         model: {model}\n\
         tools: []\n"
    )
}

/// Trigger `role` and return `(internal_session, the model the live SDK session reported)`.
fn live_session_model(ws: &Path, role: &str) -> (String, String) {
    let out = nxc(ws)
        .args(["--json", "send", "--to", role, "Say ok."])
        .output()
        .unwrap_or_else(|e| panic!("send --to {role}: {e}"));
    assert!(
        out.status.success(),
        "send --to {role} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let session = json_of(&out.stdout)["session"]
        .as_str()
        .expect("a persona summon returns the minted internal session id")
        .to_string();

    // The `session_init` entry is the SDK's own `system`/`init` message, normalized by
    // `agent-sidecar/src/transcript.mjs` — `data.model` is what the LIVE SESSION says it is running
    // on, reported by the SDK itself rather than echoed back from anything this repo wrote. That is
    // precisely the link the ticket says is unproved: the spec file and `resolveModelOption` are
    // both already covered, and neither of them can show that the SDK honoured the value.
    //
    // Gated on `result` (not on `session_init`) so the poll waits for a turn that actually
    // completed — a session that dies mid-init would otherwise pass with an init entry and nothing
    // behind it.
    let bound = Duration::from_secs(300);
    let view = poll_until(bound, Duration::from_secs(5), || {
        let out = nxc(ws)
            .args(["--json", "transcript", "show", &session])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let v = json_of(&out.stdout);
        let done = v["entries"]
            .as_array()?
            .iter()
            .any(|e| e["kind"] == "result");
        done.then_some(v)
    });
    let Some(view) = view else {
        panic!(
            "role {role}'s session {session} never reached `result` within {bound:?}.\n\
             agent-logs:\n{}",
            dump_agent_logs(ws)
        );
    };

    let init = view["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .find(|e| e["kind"] == "session_init")
        .unwrap_or_else(|| panic!("role {role} recorded no session_init entry: {view:#?}"));
    let reported = init["data"]["model"]
        .as_str()
        .unwrap_or_else(|| panic!("role {role}'s session_init carries no model: {init:#?}"))
        .to_string();
    (session, reported)
}

/// The model id `SidecarWorker` wrote into the spec file for `session` — the MIDDLE link of the
/// chain, read back off disk so the smoke pins spec → sidecar → SDK end to end rather than only its
/// last hop.
fn spec_model(ws: &Path, session: &str) -> String {
    let raw = std::fs::read_to_string(ws.join(format!(".nxs/agent-logs/{session}.spec.json")))
        .unwrap_or_else(|e| panic!("reading the spec for {session}: {e}"));
    let spec: Value = serde_json::from_str(&raw).expect("the spec file is JSON");
    spec["model"]
        .as_str()
        .unwrap_or_else(|| panic!("the spec for {session} declares no model: {spec:#?}"))
        .to_string()
}

#[test]
#[ignore = "spawns two real Claude Agent SDK sessions (NXC_WORKER=sidecar) -- real cost/time, run manually"]
fn a_declared_model_is_the_model_the_live_sdk_session_actually_runs_on() {
    // nxf 6j6v.ep7j — the last unproved link of the model-selection chain. E5c §5's third
    // acceptance bullet asks for the choice verified against the sidecar spec file AND a real run;
    // only the first half existed (`send_role_model_beats_the_roles_own_declaration_in_the_spec_json`
    // reads the spec JSON, `resolveModelOption` has a `node --test` unit test), so `options.model`
    // taking effect against the live SDK was asserted nowhere.
    //
    // **Two roles, two DIFFERENT declared models, in one run.** A single session asserting
    // "declared opus, reported opus" cannot rule out the possibility that opus is simply the
    // default — and this repo cannot know what the SDK's default is, nor should it hardcode a guess
    // that a future SDK release would silently invalidate. Two declarations producing two different
    // reported models is a coincidence-free proof that needs no such knowledge.
    let ws = TempDir::new().expect("scratch workspace");
    let root = ws.path();

    nxs_init(root);
    let roles = root.join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("asks_opus.yaml"),
        one_word_role_yaml("asks_opus", "opus"),
    )
    .unwrap();
    std::fs::write(
        roles.join("asks_sonnet.yaml"),
        one_word_role_yaml("asks_sonnet", "sonnet"),
    )
    .unwrap();

    let (opus_session, opus_reported) = live_session_model(root, "asks_opus");
    let (sonnet_session, sonnet_reported) = live_session_model(root, "asks_sonnet");
    println!("asks_opus -> {opus_reported}\nasks_sonnet -> {sonnet_reported}");

    // The middle link: the alias the role declared, resolved to an SDK id on the Rust side
    // (`Model::sdk_id`) and written into the spec the sidecar reads.
    assert_eq!(spec_model(root, &opus_session), "claude-opus-5");
    assert_eq!(spec_model(root, &sonnet_session), "claude-sonnet-5");

    // The last link: what the live session says it is running on. `starts_with`, not equality — the
    // SDK may report a dated snapshot id (`claude-opus-5-2026…`) for the same alias, and pinning the
    // exact string would make this smoke fail on a routine snapshot bump for no reason.
    assert!(
        opus_reported.starts_with("claude-opus-5"),
        "asks_opus declared `model: opus` but the live session ran on {opus_reported:?}"
    );
    assert!(
        sonnet_reported.starts_with("claude-sonnet-5"),
        "asks_sonnet declared `model: sonnet` but the live session ran on {sonnet_reported:?}"
    );
    assert_ne!(
        opus_reported, sonnet_reported,
        "two different declarations must produce two different live models — equal ones would mean \
         the declaration changed nothing and both sessions merely ran on the same default"
    );
}
