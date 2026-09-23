//! 6j6v.wbwe — the judge subprocess is **bounded**, proven at the library seam.
//!
//! `nxm migrate plan --with-judge` is the one place nexus-flow runs somebody else's program, and
//! until now it waited for that program without a deadline: a judge that neither answers nor exits
//! — `claude -p` sitting on a hanging login, say — left the command standing until somebody pressed
//! Ctrl-C. These tests call [`ask_judge`] **directly**, not through `nxm`, because that is the seam
//! an embedding host uses (the CLI is one caller of it, and a host that renders its own review UI is
//! the other); a subprocess test would prove the flag and leave the seam itself unproven.
//!
//! Its own test binary on purpose: the documented [`JUDGE_CMD_ENV`] seam is a process-global
//! environment variable, so an in-process test that sets it must not run beside another test that
//! reads it. **Exactly one test function in this file spawns a judge** — the sections inside it run
//! in sequence, each setting the variable for its own stub. Everything else here is pure. A second
//! spawning test would have to join that one rather than sit beside it (PR #293 review, Test
//! Quality #6).

#![cfg(unix)]

use nexus_memory::facade::{self, Classification};
use nexus_memory::migration::{
    ask_judge, spell_timeout, Judge, JudgeOptions, MigrationPlan, DEFAULT_JUDGE_TIMEOUT,
    DEFAULT_JUDGE_TIMEOUT_SPELLING, JUDGE_CMD_ENV,
};
use nexus_memory::store::MemoryStore;
use std::path::Path;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const NOW: &str = "2026-08-05T09:00:00Z";

/// A plan with one real entry — enough that a reply has something to be reconciled against.
fn plan(dir: &Path) -> MigrationPlan {
    let mut store = MemoryStore::open_in_memory(1);
    facade::remember(
        &mut store,
        NOW,
        "alice",
        Some("dolt-phantoms"),
        "the board is the single source of truth",
        &Classification {
            introduction: Some("the board is the single source of truth".into()),
            ..Classification::default()
        },
    )
    .unwrap();
    let plan = facade::migrate_plan(&store, dir).unwrap();
    assert_eq!(plan.entries.len(), 1, "one unfiled memory, no documents");
    plan
}

/// Write `body` as an executable-by-`sh` script and return the [`JUDGE_CMD_ENV`] value that runs it.
fn judge_script(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    format!("sh {}", path.display())
}

/// A reply that files the one entry — the shape a judge answers with, optionally padded so the
/// answer is far bigger than a pipe buffer.
fn reply(request: &MigrationPlan, pad: usize) -> String {
    let mut judged = request.clone();
    for entry in &mut judged.entries {
        entry.category = "rules".into();
    }
    // `instructions` is carried by the request, never taken from the reply — so padding it grows the
    // bytes that have to be READ without changing a single decision.
    judged.instructions = format!("{}{}", judged.instructions, "x".repeat(pad));
    serde_json::to_string(&judged).unwrap()
}

#[test]
fn the_judge_call_is_bounded_by_the_deadline_the_caller_set() {
    let tmp = TempDir::new().unwrap();
    let request = plan(tmp.path());

    // ---- (a) a judge that never answers is stopped, and the message says so --------------------
    // `sleep`, not a busy loop: it is exactly the shape of the real failure — a process that is
    // alive and healthy and simply never produces an answer.
    let hangs = judge_script(tmp.path(), "hangs.sh", "#!/bin/sh\nsleep 30\n");
    std::env::set_var(JUDGE_CMD_ENV, &hangs);
    let started = Instant::now();
    let err = ask_judge(
        Judge::Claude,
        &request,
        &JudgeOptions {
            timeout: Some(Duration::from_millis(300)),
        },
    )
    .expect_err("a judge that never answers must not be waited on forever");
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_secs(10),
        "the deadline has to actually end the call — waited {waited:?} on a 300ms limit"
    );
    assert!(
        waited >= Duration::from_millis(300),
        "…and it must not fire early either: {waited:?}"
    );
    // The overrun explains itself: how long it waited, that nothing was lost, and both ways out.
    assert!(
        err.msg.contains("did not answer within 300ms"),
        "{}",
        err.msg
    );
    assert!(err.msg.contains("--judge-timeout"), "{}", err.msg);
    assert!(err.msg.contains("nothing was written"), "{}", err.msg);

    // ---- (b) a judge that answers is believed — however large its answer ----------------------
    // The bounded wait polls for the child's exit, which means it cannot be the thing reading its
    // stdout; a reply that outgrows the ~64 KiB pipe buffer would wedge the child mid-write and the
    // deadline would then blame it for a deadlock on this side. This repository's own plan is 77 KB,
    // so that is the ordinary case, not a corner one.
    let answer = reply(&request, 200 * 1024);
    let answer_path = tmp.path().join("reply.json");
    std::fs::write(&answer_path, &answer).unwrap();
    assert!(
        answer.len() > 128 * 1024,
        "the reply has to exceed the pipe buffer for this to prove anything"
    );
    let answers = judge_script(
        tmp.path(),
        "answers.sh",
        &format!(
            "#!/bin/sh\ncat > /dev/null\ncat {}\n",
            answer_path.display()
        ),
    );
    std::env::set_var(JUDGE_CMD_ENV, &answers);
    let judged = ask_judge(
        Judge::Claude,
        &request,
        &JudgeOptions {
            timeout: Some(Duration::from_secs(60)),
        },
    )
    .expect("a judge that answers inside the limit is believed");
    assert_eq!(
        judged.entries[0].category, "rules",
        "its decision came back"
    );
    assert_eq!(
        judged.instructions, request.instructions,
        "and the request stayed the frame"
    );

    // ---- (c) no deadline at all is still a supported answer ------------------------------------
    // The reason the limit was deferred in the first place holds: a killed judge loses a whole
    // judgement. `--judge-timeout 0` gives back exactly the old contract, and it must not turn into
    // "wait zero seconds".
    let judged = ask_judge(Judge::Claude, &request, &JudgeOptions { timeout: None })
        .expect("no deadline waits for the answer rather than refusing to wait");
    assert_eq!(judged.entries[0].category, "rules");

    // …and a host on this seam can spell that same "off" as a zero duration. `--judge-timeout 0`
    // parses to `None`, so `Some(ZERO)` meaning "kill it immediately" would make the library
    // contradict the command line for the one value where the two meet (PR #293 review,
    // Integrity #1).
    let judged = ask_judge(
        Judge::Claude,
        &request,
        &JudgeOptions {
            timeout: Some(Duration::ZERO),
        },
    )
    .expect("a zero deadline is no deadline, not an instant kill");
    assert_eq!(judged.entries[0].category, "rules");

    std::env::remove_var(JUDGE_CMD_ENV);
}

#[test]
fn the_default_deadline_is_generous_and_spelled_the_same_way_twice() {
    // The `--help` text and the duration the seam defaults to are two facts that must agree; they
    // are separate constants because clap needs a string and the wait needs a `Duration`. Both
    // directions, so the pair cannot drift by either constant moving (PR #293 review, Test
    // Quality #2).
    assert_eq!(
        nexus_memory::migration::parse_judge_timeout(DEFAULT_JUDGE_TIMEOUT_SPELLING).unwrap(),
        Some(DEFAULT_JUDGE_TIMEOUT),
        "the spelling in --help is the duration the seam waits"
    );
    assert_eq!(
        spell_timeout(DEFAULT_JUDGE_TIMEOUT),
        DEFAULT_JUDGE_TIMEOUT_SPELLING,
        "…and the wait names itself the same way when it expires"
    );
    // A host that never touches the CLI gets that same default from the options struct.
    assert_eq!(
        JudgeOptions::default().timeout,
        Some(DEFAULT_JUDGE_TIMEOUT),
        "the seam's own default is the documented one"
    );
    // Generous is the requirement, not a detail: this repository's own run takes ~3 minutes, and a
    // limit that can kill real work would cost a whole judgement.
    assert!(
        DEFAULT_JUDGE_TIMEOUT >= Duration::from_secs(15 * 60),
        "a default that can interrupt a judge doing its job is worse than no default"
    );
}

#[test]
fn a_deadline_names_itself_in_the_grammar_it_was_written_in() {
    // The expiry message quotes the limit back, so every branch a real `--judge-timeout` can reach
    // has to render — the hour and minute ones were previously never exercised at all.
    for (secs, spelled) in [(45, "45s"), (90, "90s"), (1_800, "30m"), (7_200, "2h")] {
        assert_eq!(spell_timeout(Duration::from_secs(secs)), spelled);
    }
    // Sub-second deadlines exist only in tests, and must still read as something rather than "0s".
    assert_eq!(spell_timeout(Duration::from_millis(300)), "300ms");
}

#[test]
fn a_timeout_is_read_the_way_it_is_written() {
    use nexus_memory::migration::parse_judge_timeout;
    for (spelled, secs) in [("45s", 45), ("30m", 1_800), ("2h", 7_200)] {
        assert_eq!(
            parse_judge_timeout(spelled).unwrap(),
            Some(Duration::from_secs(secs)),
            "{spelled}"
        );
    }
    // Zero is not "give up immediately" — it is the escape hatch back to waiting as long as it takes.
    for off in ["0", "0s", "0m"] {
        assert_eq!(parse_judge_timeout(off).unwrap(), None, "{off}");
    }
    for bad in ["", "soon", "30", "-5m", "30 m", "5x", "m30"] {
        let err = parse_judge_timeout(bad).expect_err("{bad} is not a duration");
        assert!(err.msg.contains("--judge-timeout"), "{}", err.msg);
    }
    // A number whose SECONDS overflow is refused like any other malformed value — the guard exists
    // so an absurd `<n>` is a message, never a panic (PR #293 review, Test Quality #5).
    let err = parse_judge_timeout("9223372036854775807h").expect_err("seconds overflow u64");
    assert!(err.msg.contains("--judge-timeout"), "{}", err.msg);
}
