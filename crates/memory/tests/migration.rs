//! `nxm migrate` end to end (6j6v.9yaj) — the judging migration over the real binary.
//!
//! The unit tests next to the module cover the cutting, the reconciliation and the mark. This file
//! is the vertical proof: a workspace with unfiled memories and a hand-written `CLAUDE.md` goes
//! through `plan` → (a judge) → `apply` and comes out as one source of truth with the projection
//! carrying it — which is the acceptance the ticket is written against.

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-08-04T12:00:00Z";

fn nxm(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", NOW);
    c
}

fn stdout_of(dir: &Path, args: &[&str]) -> String {
    let out = nxm(dir).args(args).assert().success().get_output().clone();
    String::from_utf8(out.stdout).expect("utf8")
}

/// The document this migration exists to move: an orienting paragraph, a rule, and a fenced block
/// whose `#` comment must not be mistaken for a section boundary.
const CLAUDE_MD: &str = "\
# Project Instructions

This project tracks its work in nexus-flow.

## Branching

Never commit to main; branch first.

```bash
# not a heading
git checkout -b feat/x
```

## Build

Run `cargo test` before pushing.
";

/// A workspace with two unfiled memories and a hand-written `CLAUDE.md`.
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    std::fs::write(tmp.path().join("CLAUDE.md"), CLAUDE_MD).unwrap();
    for (key, body) in [
        ("auth-jwt", "auth uses JWT, not sessions"),
        ("dolt-phantoms", "phantom DBs hide in three places"),
    ] {
        nxm(tmp.path())
            .args([
                "remember",
                body,
                "--key",
                key,
                "--introduction",
                "in one line",
            ])
            .assert()
            .success();
    }
    tmp
}

fn plan_of(dir: &Path) -> Value {
    serde_json::from_str(&stdout_of(dir, &["--json", "migrate", "plan"])).expect("a plan document")
}

/// Judge a plan the way a model would: file everything under `rules`, write the one line each entry
/// is replayed by (6j6v.xbnh), and drop the section that is not durable knowledge.
fn judged(mut plan: Value, skip_keys: &[&str]) -> Value {
    for entry in plan["entries"].as_array_mut().unwrap() {
        entry["category"] = Value::String("rules".into());
        let key = entry["key"].as_str().unwrap().to_string();
        entry["introduction"] = Value::String(format!("what `{key}` says, in one line"));
        if skip_keys.contains(&key.as_str()) {
            entry["action"] = Value::String("skip".into());
        }
    }
    plan
}

fn write_plan(dir: &Path, plan: &Value) -> String {
    let path = dir.join("migration.json");
    std::fs::write(&path, serde_json::to_string_pretty(plan).unwrap()).unwrap();
    path.display().to_string()
}

// ---- plan ---------------------------------------------------------------------------------

#[test]
fn plan_offers_the_unfiled_memories_and_the_document_sections_and_judges_nothing() {
    let tmp = workspace();
    let plan = plan_of(tmp.path());

    let entries = plan["entries"].as_array().unwrap();
    let keys: Vec<&str> = entries.iter().map(|e| e["key"].as_str().unwrap()).collect();
    assert_eq!(
        keys,
        [
            "auth-jwt",
            "dolt-phantoms",
            "project-instructions",
            "branching",
            "build"
        ],
        "the unfiled memories, then one entry per section — the fenced `#` comment is not one"
    );
    assert!(
        entries.iter().all(|e| e["category"] == "unsorted"),
        "nothing is guessed: a heuristic dressed as a proposal is what the ticket rules out"
    );
    assert!(
        entries[3]["body"]
            .as_str()
            .unwrap()
            .contains("# not a heading"),
        "the section keeps its own fenced content"
    );
    assert!(plan["instructions"].as_str().unwrap().contains("skip"));
    assert_eq!(plan["version"], 1);
}

#[test]
fn the_generated_document_is_never_offered_back_as_a_source() {
    // The rule hangs on CONTENT, and the everyday case for it is the file this feature itself
    // writes: importing a projection back into its own source is the loop the drift guard exists
    // against. `NEXUS_MEMORY.md` exists here because remembering wrote it.
    let tmp = workspace();
    assert!(tmp.path().join("NEXUS_MEMORY.md").is_file());

    let plan = plan_of(tmp.path());
    assert!(
        plan["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|e| e["source"].get("path"))
            .all(|p| p != "NEXUS_MEMORY.md"),
        "{plan:#}"
    );
    let skipped = plan["skipped_sources"].as_array().unwrap();
    assert!(
        skipped
            .iter()
            .any(|s| s["path"] == "NEXUS_MEMORY.md" && s["reason"] == "generated"),
        "and the skip is stated rather than silent: {skipped:?}"
    );
}

#[test]
fn the_human_rendering_names_every_entry_and_the_command_that_decides_them() {
    let tmp = workspace();
    let out = stdout_of(tmp.path(), &["migrate", "plan"]);
    assert!(
        out.contains("2 unfiled memories, 3 document sections"),
        "{out}"
    );
    assert!(out.contains("auth-jwt"), "{out}");
    assert!(out.contains("--with-judge"), "{out}");
}

// ---- the judge ----------------------------------------------------------------------------

#[test]
fn an_unknown_judge_is_refused_by_name_and_nothing_is_spawned() {
    let tmp = workspace();
    let out = nxm(tmp.path())
        .args(["migrate", "plan", "--with-judge", "gpt-9"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("unknown judge 'gpt-9'"), "{err}");
    assert!(
        err.contains("claude"),
        "and it names what it does know: {err}"
    );
}

/// A stub judge: reads the plan on stdin into `$1`, answers with `$2`. It stands in for
/// `claude -p` through the documented `NXM_JUDGE_CMD` test seam, so the subprocess path — the one
/// place a model is asked — is proven rather than asserted in prose.
#[cfg(unix)]
fn stub_judge(dir: &Path, reply: &Value) -> (String, std::path::PathBuf) {
    let script = dir.join("judge.sh");
    std::fs::write(&script, "#!/bin/sh\ncat > \"$1\"\ncat \"$2\"\n").unwrap();
    let captured = dir.join("captured.json");
    let reply_path = dir.join("reply.json");
    std::fs::write(&reply_path, serde_json::to_string(reply).unwrap()).unwrap();
    (
        format!(
            "sh {} {} {}",
            script.display(),
            captured.display(),
            reply_path.display()
        ),
        captured,
    )
}

#[cfg(unix)]
#[test]
fn the_judge_is_handed_the_plan_and_its_decisions_come_back() {
    let tmp = workspace();
    let reply = judged(plan_of(tmp.path()), &["project-instructions"]);
    let (cmd, captured) = stub_judge(tmp.path(), &reply);

    let out = nxm(tmp.path())
        .env("NXM_JUDGE_CMD", &cmd)
        .args(["--json", "migrate", "plan", "--with-judge"])
        .assert()
        .success()
        .get_output()
        .clone();
    let plan: Value = serde_json::from_slice(&out.stdout).unwrap();

    // The judge really received the request: the task as prose FIRST — a prompt whose instructions
    // are buried in a JSON field is a worse prompt — and the whole document after it.
    let sent = std::fs::read_to_string(captured).unwrap();
    assert!(sent.starts_with("Decide, for every entry,"), "{sent:.80}");
    let request: Value =
        serde_json::from_str(&sent[sent.find('{').unwrap()..=sent.rfind('}').unwrap()]).unwrap();
    assert_eq!(request["entries"].as_array().unwrap().len(), 5);
    assert!(request["instructions"].as_str().unwrap().contains("skip"));
    // …and its decisions are what the plan now carries.
    let entries = plan["entries"].as_array().unwrap();
    assert!(entries.iter().all(|e| e["category"] == "rules"));
    assert_eq!(entries[2]["action"], "skip");
}

#[cfg(unix)]
#[test]
fn a_judge_that_dropped_an_entry_is_refused_rather_than_believed() {
    // A lost entry is lost knowledge, and this is the one place it would never be noticed.
    let tmp = workspace();
    let mut reply = judged(plan_of(tmp.path()), &[]);
    reply["entries"].as_array_mut().unwrap().remove(1);
    let (cmd, _) = stub_judge(tmp.path(), &reply);

    let out = nxm(tmp.path())
        .env("NXM_JUDGE_CMD", &cmd)
        .args(["migrate", "plan", "--with-judge"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("dolt-phantoms"), "{err}");
    assert!(err.contains("nothing was applied"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_judge_that_fails_or_answers_with_nonsense_is_reported_not_believed() {
    // PR #289 review, Test Quality #4: the failure paths were only exercised against `parse_reply`
    // directly, so nothing proved the real subprocess path surfaces them — and this is the one
    // place in nexus-flow that runs somebody else's program.
    let tmp = workspace();
    let script = tmp.path().join("bad.sh");

    // (a) a judge that exits non-zero: its status is the message, not a parse error.
    std::fs::write(
        &script,
        "#!/bin/sh\ncat > /dev/null\necho 'I refuse' >&2\nexit 3\n",
    )
    .unwrap();
    let out = nxm(tmp.path())
        .env("NXM_JUDGE_CMD", format!("sh {}", script.display()))
        .args(["migrate", "plan", "--with-judge"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("exited with"), "{err}");

    // (b) a judge that succeeds but answers with prose: refused, and the plan is not half-taken.
    std::fs::write(
        &script,
        "#!/bin/sh\ncat > /dev/null\necho 'I could not do that'\n",
    )
    .unwrap();
    let out = nxm(tmp.path())
        .env("NXM_JUDGE_CMD", format!("sh {}", script.display()))
        .args(["migrate", "plan", "--with-judge"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("no JSON document"), "{err}");

    // (c) a judge that answers with JSON that is not a plan.
    std::fs::write(
        &script,
        "#!/bin/sh\ncat > /dev/null\necho '{\"hello\":true}'\n",
    )
    .unwrap();
    let out = nxm(tmp.path())
        .env("NXM_JUDGE_CMD", format!("sh {}", script.display()))
        .args(["migrate", "plan", "--with-judge"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("not a migration plan"), "{err}");

    // Through all three, the workspace is exactly as it was — planning never writes.
    assert!(stdout_of(tmp.path(), &["migrate", "status"]).contains("2 unfiled memories"));
}

#[cfg(unix)]
#[test]
fn a_judge_that_never_drains_its_stdin_does_not_deadlock_the_command() {
    // PR #289 review, Integrity #3. The plan carries the documents it is migrating, so it is large
    // — this repository's own is 77 KB — and writing it to the child before reading a byte of its
    // answer blocks forever once it outgrows the ~64 KiB pipe buffer while the child blocks writing
    // back. The stub here is exactly that judge: it answers immediately and never reads its input.
    let tmp = workspace();
    // A section big enough that the serialized plan cannot fit in one pipe buffer.
    std::fs::write(
        tmp.path().join("GEMINI.md"),
        format!("## Big\n\n{}\n", "padding ".repeat(20_000)),
    )
    .unwrap();
    let plan = judged(plan_of(tmp.path()), &[]);
    assert!(
        serde_json::to_string(&plan).unwrap().len() > 128 * 1024,
        "the request has to exceed the pipe buffer for this to prove anything"
    );

    let script = tmp.path().join("deaf.sh");
    let reply = tmp.path().join("reply.json");
    std::fs::write(&reply, serde_json::to_string(&plan).unwrap()).unwrap();
    // No `cat` — stdin is never read, so an unbuffered writer on our side would wedge.
    std::fs::write(&script, format!("#!/bin/sh\ncat {}\n", reply.display())).unwrap();

    nxm(tmp.path())
        .env("NXM_JUDGE_CMD", format!("sh {}", script.display()))
        .args(["--json", "migrate", "plan", "--with-judge"])
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
fn the_wait_for_the_judge_is_announced_and_bounded_from_the_command_line() {
    // 6j6v.wbwe. The seam's own proof is `tests/judge.rs`; this one is about the two things only the
    // command can show — that the deadline is VISIBLE before the wait starts (a user watching a
    // judge think for minutes must know what they are waiting for), and that `--judge-timeout`
    // really reaches the call.
    let tmp = workspace();
    let script = tmp.path().join("hangs.sh");
    std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();

    let out = nxm(tmp.path())
        .env("NXM_JUDGE_CMD", format!("sh {}", script.display()))
        .args(["migrate", "plan", "--with-judge", "--judge-timeout", "1s"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(
        err.contains("asking the judge `claude`") && err.contains("waiting up to 1s"),
        "the deadline is announced before the wait, not only when it expires: {err}"
    );
    assert!(err.contains("did not answer within 1s"), "{err}");
    assert!(
        err.contains("loses nothing") && err.contains("--judge-timeout 0"),
        "the overrun says how to recover and how to opt out: {err}"
    );
    // Planning writes nothing, and a stopped judge is no exception.
    assert!(stdout_of(tmp.path(), &["migrate", "status"]).contains("2 unfiled memories"));

    // A malformed duration is refused by name, before anything is spawned or read.
    let out = nxm(tmp.path())
        .env("NXM_JUDGE_CMD", format!("sh {}", script.display()))
        .args(["migrate", "plan", "--with-judge", "--judge-timeout", "soon"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("--judge-timeout `soon`"), "{err}");
    assert!(
        !err.contains("asking the judge"),
        "nothing was spawned: {err}"
    );

    // …and the flag is meaningless without a judge, so it is refused rather than silently ignored.
    let out = nxm(tmp.path())
        .args(["migrate", "plan", "--judge-timeout", "30m"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("--with-judge"), "{err}");
}

// ---- apply --------------------------------------------------------------------------------

#[test]
fn apply_files_the_memories_moves_the_sections_and_hands_over_to_the_projection() {
    let tmp = workspace();
    let plan = judged(plan_of(tmp.path()), &["project-instructions"]);
    let path = write_plan(tmp.path(), &plan);

    let report: Value = serde_json::from_str(&stdout_of(
        tmp.path(),
        &["--json", "migrate", "apply", "--plan", &path],
    ))
    .unwrap();
    assert_eq!(
        report["classified"],
        serde_json::json!(["auth-jwt", "dolt-phantoms"])
    );
    assert_eq!(
        report["remembered"],
        serde_json::json!(["branching", "build"])
    );
    assert_eq!(
        report["skipped"],
        serde_json::json!(["project-instructions"])
    );
    assert_eq!(report["pruned"], serde_json::json!(["CLAUDE.md"]));

    // The sections are memories now, filed and in the order the plan put them in.
    let memories: Value =
        serde_json::from_str(&stdout_of(tmp.path(), &["--json", "memories", "--ordered"])).unwrap();
    let keys: Vec<&str> = memories
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["auth-jwt", "dolt-phantoms", "branching", "build"]);
    assert!(memories
        .as_array()
        .unwrap()
        .iter()
        .all(|m| m["category"] == "rules"));

    // The document is the ONE source now: the migrated sections left CLAUDE.md and the projection
    // carries them.
    let left = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
    assert!(!left.contains("Never commit to main"), "{left}");
    assert!(!left.contains("cargo test"), "{left}");
    assert!(
        left.contains("This project tracks its work in nexus-flow."),
        "the skipped section stays where it was: {left}"
    );
    let doc = std::fs::read_to_string(tmp.path().join("NEXUS_MEMORY.md")).unwrap();
    assert!(doc.contains("Never commit to main"), "{doc}");
    assert!(doc.contains("cargo test"), "{doc}");

    // …and the projection is in sync, which is what makes the hand-written document dispensable.
    nxm(tmp.path()).args(["doc", "--check"]).assert().success();
    assert!(stdout_of(tmp.path(), &["migrate", "status"]).contains("nothing left to file"));
}

#[test]
fn apply_refuses_a_plan_nobody_judged_and_writes_nothing() {
    let tmp = workspace();
    let path = write_plan(tmp.path(), &plan_of(tmp.path()));

    let out = nxm(tmp.path())
        .args(["migrate", "apply", "--plan", &path])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("unsorted"), "{err}");

    let plan = plan_of(tmp.path());
    assert_eq!(
        plan["entries"].as_array().unwrap().len(),
        5,
        "the workspace is exactly as it was"
    );
    assert!(std::fs::read_to_string(tmp.path().join("CLAUDE.md"))
        .unwrap()
        .contains("Never commit to main"));
}

#[test]
fn apply_reads_the_plan_from_stdin_too() {
    let tmp = workspace();
    let plan = judged(plan_of(tmp.path()), &[]);
    nxm(tmp.path())
        .args(["migrate", "apply", "--plan", "-"])
        .write_stdin(serde_json::to_string(&plan).unwrap())
        .assert()
        .success();
    assert!(stdout_of(tmp.path(), &["migrate", "status"]).contains("nothing left to file"));
}

// ---- once per stream, not once per machine ------------------------------------------------

#[test]
fn a_second_run_reports_the_earlier_one_and_stops_until_it_is_told_to_go_again() {
    // Two independent judgements of the same memories would converge by last-writer-wins into a
    // blend neither device decided. So the marked stream ASKS instead of starting over.
    let tmp = workspace();
    let path = write_plan(tmp.path(), &judged(plan_of(tmp.path()), &[]));
    nxm(tmp.path())
        .args(["migrate", "apply", "--plan", &path])
        .assert()
        .success();

    let out = nxm(tmp.path())
        .args(["migrate", "plan"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("already migrated by alice"), "{err}");
    assert!(
        err.contains("--again"),
        "and it names the way through: {err}"
    );

    nxm(tmp.path())
        .args(["migrate", "plan", "--again"])
        .assert()
        .success();

    // The mark is durable and is not a memory — it must never surface where memories are read.
    let status: Value =
        serde_json::from_str(&stdout_of(tmp.path(), &["--json", "migrate", "status"])).unwrap();
    assert_eq!(status["mark"]["actor"], "alice");
    assert_eq!(status["unsorted"], 0);
    let memories: Value =
        serde_json::from_str(&stdout_of(tmp.path(), &["--json", "memories"])).unwrap();
    assert!(
        memories
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["key"] != "nxm:migration"),
        "{memories:#}"
    );
    assert!(!std::fs::read_to_string(tmp.path().join("NEXUS_MEMORY.md"))
        .unwrap()
        .contains("nxm:migration"));
}

#[test]
fn a_dry_run_says_what_it_would_do_and_leaves_everything_where_it_is() {
    let tmp = workspace();
    let path = write_plan(tmp.path(), &judged(plan_of(tmp.path()), &[]));
    let before = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();

    let report: Value = serde_json::from_str(&stdout_of(
        tmp.path(),
        &["--json", "migrate", "apply", "--plan", &path, "--dry-run"],
    ))
    .unwrap();
    assert_eq!(report["dry_run"], true);
    assert_eq!(
        report["remembered"],
        serde_json::json!(["project-instructions", "branching", "build"])
    );
    assert!(report["mark"].is_null());

    assert_eq!(
        std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap(),
        before
    );
    assert!(stdout_of(tmp.path(), &["migrate", "status"]).contains("2 unfiled memories"));
}
