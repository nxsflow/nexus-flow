//! Black-box CLI tests for `nxc transcript append` (nxf epic 6wt2, ticket f8c9): the sidecar's
//! callback contract, driven exactly as `agent-sidecar/src/main.mjs` drives it — JSON-lines on
//! STDIN, `--session <internal>` naming the transcript. Mirrors `tests/workflow_session.rs`'s
//! harness: spawn the real `nxc` binary against a seeded `.nxs/` workspace, then re-open the SAME db
//! to inspect what actually landed.
//!
//! Why black-box on top of `src/transcript.rs`'s unit tests: the stdin framing, the line-numbered
//! parse error, and the `--json` record ARE the contract T1 already ships against. T1's call site
//! tolerates exactly clap's "unrecognized subcommand" shape and propagates everything else, so any
//! error this surface returns is fatal to the sidecar — the exit code and the message have to be
//! right, not just the store write.

use assert_cmd::Command;
use nexus_chat::store::ChatStore;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use serde_json::{json, Value};
use tempfile::TempDir;

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "alice")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", "2026-07-26T00:00:00Z");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

/// Open the SAME db the `nxc` subprocess wrote to.
fn open_store(tmp: &TempDir) -> ChatStore {
    Workspace::resolve(None, tmp.path())
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store")
}

/// The sidecar's framing verbatim (`main.mjs`): one `JSON.stringify(entry)` per line, trailing
/// newline.
fn lines(entries: &[Value]) -> String {
    let mut s = entries
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    s.push('\n');
    s
}

#[test]
fn append_stores_a_json_lines_batch_from_stdin_and_reports_the_count() {
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["--json", "transcript", "append", "--session", "s-1"])
        .write_stdin(lines(&[
            json!({"kind":"session_init","at":"2026-07-26T07:42:39.123Z",
                   "data":{"sdkSessionId":"real-abc","model":"claude-opus-5","tools":12}}),
            json!({"kind":"assistant","at":"2026-07-26T07:42:40.000Z",
                   "data":{"text":"On it."}}),
        ]))
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout),
        json!({ "session": "s-1", "appended": 2 })
    );

    let rows = open_store(&tmp).transcript_rows("s-1").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].seq, 0);
    assert_eq!(rows[0].entry.kind, "session_init");
    assert_eq!(rows[0].entry.data["sdkSessionId"], "real-abc");
    assert_eq!(rows[1].seq, 1);
    assert_eq!(rows[1].entry.data["text"], "On it.");
}

#[test]
fn a_second_append_continues_the_transcript_across_processes() {
    // The resume case end-to-end: a SECOND `nxc` process (a second sidecar) appends to the same
    // internal session, and `seq` continues rather than restarting — the whole reason `seq` is
    // assigned store-side instead of carried on the wire.
    let tmp = workspace();
    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-1"])
        .write_stdin(lines(&[
            json!({"kind":"assistant","at":"2026-07-26T07:42:40.000Z","data":{"text":"turn one"}}),
            json!({"kind":"result","at":"2026-07-26T07:42:41.000Z",
                   "data":{"subtype":"success","isError":false,"numTurns":1,"resultText":"done"}}),
        ]))
        .assert()
        .success()
        .stdout("appended 2 entries to s-1\n");
    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-1"])
        .write_stdin(lines(&[
            json!({"kind":"assistant","at":"2026-07-26T07:43:00.000Z","data":{"text":"turn two"}}),
        ]))
        .assert()
        .success()
        .stdout("appended 1 entries to s-1\n");

    let rows = open_store(&tmp).transcript_rows("s-1").unwrap();
    assert_eq!(rows.iter().map(|r| r.seq).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(rows[2].entry.data["text"], "turn two");
    // `data` is opaque and round-trips as given, so a success result keeps its `resultText` (an
    // error result, where T1 omits the key entirely, keeps it omitted for the same reason).
    assert_eq!(rows[1].entry.data["resultText"], "done");
}

#[test]
fn nested_subagent_entries_round_trip_through_the_cli() {
    // The gap beads left open — a Task-spawned subagent's sub-timeline must survive the persist
    // path. Driven over the wire, camelCase tags and all, so the `#[serde(rename_all)]` mapping is
    // exercised by the real contract rather than only by a Rust-side literal.
    let tmp = workspace();
    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-1"])
        .write_stdin(lines(&[
            json!({"kind":"tool_use","at":"2026-07-26T07:42:40.000Z","toolUseId":"toolu_task_1",
                   "data":{"name":"Task","input":{"subagent_type":"code-reviewer"}}}),
            json!({"kind":"thinking","at":"2026-07-26T07:42:41.000Z",
                   "parentToolUseId":"toolu_task_1","subagentType":"code-reviewer",
                   "data":{"text":"the diff touches the reducer"}}),
            json!({"kind":"tool_result","at":"2026-07-26T07:42:42.000Z","toolUseId":"toolu_read_9",
                   "parentToolUseId":"toolu_task_1","subagentType":"code-reviewer",
                   "data":{"content":"fn main() {}","isError":false}}),
            json!({"kind":"assistant","at":"2026-07-26T07:42:43.000Z",
                   "data":{"text":"reviewer says it is fine"}}),
        ]))
        .assert()
        .success();

    let rows = open_store(&tmp).transcript_rows("s-1").unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].entry.tool_use_id.as_deref(), Some("toolu_task_1"));
    assert_eq!(rows[0].entry.parent_tool_use_id, None);
    for r in &rows[1..3] {
        assert_eq!(r.entry.parent_tool_use_id.as_deref(), Some("toolu_task_1"));
        assert_eq!(r.entry.subagent_type.as_deref(), Some("code-reviewer"));
    }
    assert_eq!(rows[2].entry.tool_use_id.as_deref(), Some("toolu_read_9"));
    // The trailing main-conversation entry is untagged — the nesting boundary is real, not just a
    // property of position in the stream.
    assert_eq!(rows[3].entry.parent_tool_use_id, None);
    assert_eq!(rows[3].entry.subagent_type, None);
}

#[test]
fn payloads_survive_stdin_intact_including_a_batch_larger_than_the_pipe_buffer() {
    // Two things at once, because they share the one fixture that makes either worth running:
    //
    // 1. T1 applies both caps producer-side (MAX_THINKING_CHARS = 4000, MAX_TOOL_RESULT_CHARS =
    //    8000) and appends its own truncation marker. The store re-caps nothing: what the sidecar
    //    sent is what comes back, marker included.
    // 2. The batch is deliberately pushed past 64 KB — the OS pipe buffer. `main.mjs:94-96` reasons
    //    explicitly about a batch that exceeds it (a child that exits without draining stdin raises
    //    EPIPE on the writer), and stdin is the ONLY way entries reach this command, so a reader
    //    that stopped short of EOF would silently drop the tail of every large flush. A single
    //    unbounded `tool_use` input — the very case the no-cap decision accepts — is what gets it
    //    over the line, so this also proves that decision end-to-end rather than only in-process.
    let tmp = workspace();
    let thinking = format!("{}\n…[truncated 91 chars]", "t".repeat(4000));
    let tool_result = format!("{}\n…[truncated 12345 chars]", "r".repeat(8000));
    let written_file = "fn main() { /* ".repeat(10_000); // ~150 KB, comfortably over one pipe buffer
    let stdin = lines(&[
        json!({"kind":"thinking","at":"2026-07-26T07:42:40.000Z","data":{"text":thinking}}),
        json!({"kind":"tool_result","at":"2026-07-26T07:42:41.000Z","toolUseId":"toolu_1",
               "data":{"content":tool_result,"isError":false}}),
        json!({"kind":"tool_use","at":"2026-07-26T07:42:42.000Z","toolUseId":"toolu_2",
               "data":{"name":"Write","input":{"file_path":"/big.rs","content":written_file}}}),
    ]);
    assert!(
        stdin.len() > 64 * 1024,
        "fixture must exceed the pipe buffer, got {} bytes",
        stdin.len()
    );
    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-1"])
        .write_stdin(stdin)
        .assert()
        .success()
        .stdout("appended 3 entries to s-1\n");

    let rows = open_store(&tmp).transcript_rows("s-1").unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].entry.data["text"], json!(thinking));
    assert_eq!(rows[1].entry.data["content"], json!(tool_result));
    // The last entry is the one on the far side of the pipe buffer: it must be there AND whole.
    assert_eq!(rows[2].entry.data["input"]["content"], json!(written_file));
}

#[test]
fn blank_lines_are_skipped_but_a_malformed_line_is_a_loud_error_naming_its_line_number() {
    // The sidecar joins entries with "\n" and adds a trailing newline, so a trailing blank line is
    // NORMAL framing and must not be an error. A line that is genuinely not a transcript entry is
    // the opposite: fatal, with the line number, because T1 propagates everything that is not
    // clap's "unrecognized subcommand" — a silent skip here would lose transcript with no trace.
    let tmp = workspace();
    nxc(&tmp)
        .args(["--json", "transcript", "append", "--session", "s-1"])
        .write_stdin("\n{\"kind\":\"assistant\",\"data\":{\"text\":\"ok\"}}\n\n")
        .assert()
        .success();
    assert_eq!(open_store(&tmp).transcript_rows("s-1").unwrap().len(), 1);

    let out = nxc(&tmp)
        .args(["--json", "transcript", "append", "--session", "s-1"])
        .write_stdin("{\"kind\":\"assistant\",\"data\":{\"text\":\"ok\"}}\nnot json at all\n")
        .assert()
        .failure();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["error"]["kind"], "validation");
    let msg = v["error"]["msg"].as_str().unwrap();
    assert!(msg.contains("line 2"), "names the line number: {msg}");

    // ...and the good line that preceded it did not land: one bad line rejects the whole batch. Not
    // so a retry is safe (the sidecar does not retry — it clears its buffer before the write), but
    // so the store never keeps a prefix of a failed flush that reads back like a complete one.
    assert_eq!(open_store(&tmp).transcript_rows("s-1").unwrap().len(), 1);
}

#[test]
fn an_entry_with_an_empty_kind_is_rejected_with_its_line_number() {
    // `kind` is what makes `data` interpretable at all; an empty one is a malformed line, not a
    // storable row. Reported with the LINE number (not the batch index) — the sidecar's operator is
    // looking at a stream of lines.
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["--json", "transcript", "append", "--session", "s-1"])
        .write_stdin("{\"kind\":\"assistant\",\"data\":{}}\n{\"kind\":\"\",\"data\":{}}\n")
        .assert()
        .failure();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["error"]["kind"], "validation");
    let msg = v["error"]["msg"].as_str().unwrap();
    assert!(msg.contains("line 2"), "names the line number: {msg}");
    assert!(open_store(&tmp).transcript_rows("s-1").unwrap().is_empty());
}

#[test]
fn an_entirely_empty_stdin_is_a_clean_no_op() {
    // The sidecar guards `transcript.length === 0` itself, but a flush that races an empty buffer
    // must not become a fatal error in the middle of a real session.
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["--json", "transcript", "append", "--session", "s-1"])
        .write_stdin("")
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout),
        json!({ "session": "s-1", "appended": 0 })
    );
}

#[test]
fn an_append_survives_a_concurrent_writer_holding_the_write_lock() {
    // Review round 1, IMPORTANT-1 — the regression guard for `BEGIN IMMEDIATE`.
    //
    // `agent_transcript` lives in the SAME `db.sqlite` as the ops log, so the role's own in-session
    // `nxc send` is a genuine concurrent writer. Under a DEFERRED transaction (what
    // `unchecked_transaction()` gives) `append_transcript` takes its read snapshot at the
    // `MAX(seq)` SELECT and only asks for the write lock at the first INSERT; if anything committed
    // in between, SQLite cannot upgrade the stale snapshot and returns SQLITE_BUSY_SNAPSHOT — for
    // which the busy handler is NOT invoked, so `busy_timeout=5000` does nothing and the call fails
    // instantly. That would abort the entire role turn (the sidecar propagates everything but
    // clap's "unrecognized subcommand") over a telemetry write.
    //
    // Measured on this exact fixture: DEFERRED → Err("database is locked") after 1.5ms;
    // IMMEDIATE → Ok after 286ms, having waited for the lock like a well-behaved writer.
    //
    // The sleeps only ever make this test MORE likely to pass spuriously (if B finishes before A
    // starts, both behaviors succeed) rather than to fail spuriously; the one way a loaded machine
    // could red it is B's 300ms hold stretching past A's `busy_timeout=5000`, which is a wide margin.
    let tmp = workspace();
    let path = tmp
        .path()
        .join(".nxs/db.sqlite")
        .to_str()
        .unwrap()
        .to_string();
    let mut store = open_store(&tmp);

    // B takes the write lock, writes, and holds it while A tries to append.
    let p = path.clone();
    let competing_writer = std::thread::spawn(move || {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("PRAGMA busy_timeout=5000;").unwrap();
        c.execute_batch("BEGIN IMMEDIATE").unwrap();
        // A real committed change to the same file is what makes the deferred reader's snapshot
        // STALE. It is NOT what makes the guard discriminate, though — measured across the full
        // 2x2, DEFERRED fails instantly under BOTH commit and rollback: SQLite does not invoke the
        // busy handler for ANY read→write upgrade against a concurrent write-lock holder (deadlock
        // avoidance), and committing merely layers SQLITE_BUSY_SNAPSHOT on top of that. COMMIT is
        // written here for FIDELITY to the real scenario — the role's own in-session `nxc send`
        // commits — not because a rollback would let a deferred append through.
        c.execute("INSERT INTO membership_removes(tag) VALUES ('noise')", [])
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        c.execute_batch("COMMIT").unwrap();
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let entry: nexus_chat::transcript::TranscriptEntry =
        serde_json::from_value(json!({"kind":"assistant","at":"2026-07-26T07:42:40.000Z",
                                      "data":{"text":"written while the lock was held"}}))
        .unwrap();
    let appended = store.append_transcript("s-1", std::slice::from_ref(&entry));
    competing_writer.join().unwrap();

    assert!(
        appended.is_ok(),
        "a concurrent writer must not fail the append: {:?}",
        appended.err().map(|e| e.msg)
    );
    assert_eq!(store.transcript_rows("s-1").unwrap().len(), 1);
}

#[test]
fn an_empty_batch_returns_zero_without_ever_taking_the_write_lock() {
    // `append_transcript`'s empty-batch early return became load-bearing when the transaction went
    // `BEGIN IMMEDIATE` (review round 1): without it, every empty flush would take the WORKSPACE
    // WRITE lock — not merely open a harmless read transaction — and the sidecar flushes on a timer
    // against the SAME db.sqlite the role's own in-session `nxc` writes to. The existing unit test
    // proves "returns 0"; only this one distinguishes "returns 0 WITHOUT locking".
    //
    // The holder keeps the write lock for the whole call, so a lock-taking version would block on
    // its own `busy_timeout` (5s) and then fail — this reds by unwrap-panic, and the elapsed bound
    // is only a second, faster signal.
    let tmp = workspace();
    let path = tmp.path().join(".nxs/db.sqlite");
    let mut store = open_store(&tmp);

    let holder = rusqlite::Connection::open(&path).unwrap();
    holder
        .execute_batch("PRAGMA busy_timeout=5000; BEGIN IMMEDIATE;")
        .unwrap();
    holder
        .execute("INSERT INTO membership_removes(tag) VALUES ('noise')", [])
        .unwrap();

    let started = std::time::Instant::now();
    let appended = store.append_transcript("s-1", &[]);
    let elapsed = started.elapsed();
    holder.execute_batch("ROLLBACK").unwrap();

    assert_eq!(
        appended.unwrap(),
        0,
        "an empty batch must not even try for the write lock"
    );
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "an empty batch returned only after waiting {elapsed:?} — it took the write lock"
    );
}

#[test]
fn a_transcript_for_an_unmapped_session_is_still_stored() {
    // Deliberately no foreign key to `session_map`: `create_pending_session` and the sidecar's first
    // flush race, and a transcript for a session that was never bound is exactly the evidence of
    // what went wrong. Storing it must not depend on the mapping existing.
    let tmp = workspace();
    nxc(&tmp)
        .args(["transcript", "append", "--session", "never-minted"])
        .write_stdin(lines(&[
            json!({"kind":"error","at":"2026-07-26T07:42:40.000Z",
                   "data":{"kind":"auth","message":"authentication_failed"}}),
        ]))
        .assert()
        .success();
    let rows = open_store(&tmp).transcript_rows("never-minted").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entry.data["kind"], "auth");
}

// ---- `nxc transcript show` (nxf epic 6wt2, ticket 5bym) ---------------------

/// The batch every `show` test reads back: a main-conversation Task whose subagent contributed
/// three entries, plus a trailing main-conversation answer. Written through the real CLI append
/// path, so `show` is genuinely reading what the sidecar's own wire produced.
fn seed_a_subagent_transcript(tmp: &TempDir) {
    nxc(tmp)
        .args(["transcript", "append", "--session", "s-1"])
        .write_stdin(lines(&[
            json!({"kind":"session_init","at":"2026-07-26T07:42:39.123Z",
                   "data":{"sdkSessionId":"real-abc"}}),
            json!({"kind":"tool_use","at":"2026-07-26T07:42:40.000Z","toolUseId":"toolu_task_1",
                   "data":{"name":"Task","input":{"subagent_type":"code-reviewer"}}}),
            json!({"kind":"thinking","at":"2026-07-26T07:42:41.000Z",
                   "parentToolUseId":"toolu_task_1","subagentType":"code-reviewer",
                   "data":{"text":"the diff touches the reducer"}}),
            json!({"kind":"tool_use","at":"2026-07-26T07:42:42.000Z","toolUseId":"toolu_read_9",
                   "parentToolUseId":"toolu_task_1","subagentType":"code-reviewer",
                   "data":{"name":"Read","input":{"file_path":"/x.rs"}}}),
            json!({"kind":"tool_result","at":"2026-07-26T07:42:43.000Z","toolUseId":"toolu_read_9",
                   "parentToolUseId":"toolu_task_1","subagentType":"code-reviewer",
                   "data":{"content":"fn main() {}","isError":false}}),
            json!({"kind":"assistant","at":"2026-07-26T07:42:44.000Z",
                   "data":{"text":"the reviewer found nothing"}}),
        ]))
        .assert()
        .success();
}

#[test]
fn show_json_is_the_nested_view_and_is_byte_identical_to_the_facade() {
    // The seam invariant, at the one place it can be checked cheaply: `--json` prints
    // `TranscriptView::to_value()` verbatim, so the `nxc` bytes and the in-process facade value
    // (what `Engine::transcript_page` hands an embedding app) are the SAME rendering of the same
    // read.
    let tmp = workspace();
    seed_a_subagent_transcript(&tmp);

    let out = nxc(&tmp)
        .args(["--json", "transcript", "show", "s-1"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(
        v,
        nexus_chat::facade::transcript(&open_store(&tmp), "s-1")
            .unwrap()
            .to_value()
    );

    // …and that value is the nested shape, not the flat rows.
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(
        entries.iter().map(|e| e["seq"].clone()).collect::<Vec<_>>(),
        [json!(0), json!(1), json!(5)]
    );
    let task = &entries[1];
    assert_eq!(task["tool_use_id"], "toolu_task_1");
    assert_eq!(
        task["subagent"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["seq"].clone())
            .collect::<Vec<_>>(),
        [json!(2), json!(3), json!(4)]
    );
    assert_eq!(task["subagent"][1]["subagent_type"], "code-reviewer");
    assert_eq!(task["subagent"][1]["data"]["input"]["file_path"], "/x.rs");
    // The subagent's own tool_use nests no further (one level, by contract).
    assert_eq!(task["subagent"][1]["subagent"], json!([]));
}

#[test]
fn show_json_carries_the_session_map_facts_when_the_session_was_minted() {
    let tmp = workspace();
    {
        let mut s = open_store(&tmp);
        s.create_pending_session("s-1", "coder").unwrap();
        s.bind_session("s-1", "real-abc").unwrap();
    }
    seed_a_subagent_transcript(&tmp);
    let out = nxc(&tmp)
        .args(["--json", "transcript", "show", "s-1"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["role"], "coder");
    assert_eq!(v["real_sdk_id"], "real-abc");
}

#[test]
fn show_of_an_unknown_session_is_an_empty_transcript_not_an_error() {
    // A session whose sidecar never flushed is indistinguishable from one that had nothing to say,
    // and an operator reading a transcript that isn't there deserves an empty timeline, not a
    // failure exit code.
    let tmp = workspace();
    let out = nxc(&tmp)
        .args(["--json", "transcript", "show", "never-existed"])
        .assert()
        .success();
    assert_eq!(
        json_of(&out.get_output().stdout),
        json!({ "session": "never-existed", "entries": [] })
    );
    nxc(&tmp)
        .args(["transcript", "show", "never-existed"])
        .assert()
        .success()
        .stdout("transcript never-existed  (0 entries)\n");
}

#[test]
fn show_human_output_indents_the_subagent_timeline_under_its_tool_use() {
    let tmp = workspace();
    seed_a_subagent_transcript(&tmp);
    let out = nxc(&tmp)
        .args(["transcript", "show", "s-1"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "transcript s-1  (6 entries)");
    assert_eq!(
        lines[2], "#1 2026-07-26T07:42:40.000Z tool_use {\"input\":{\"subagent_type\":\"code-reviewer\"},\"name\":\"Task\"}",
        "a main-conversation entry sits at column 0, with a gist of its opaque data"
    );
    // The three subagent entries are indented under it and keep their own seq numbers.
    for (i, seq) in [2, 3, 4].iter().enumerate() {
        let line = lines[3 + i];
        assert!(
            line.starts_with(&format!("  #{seq} ")),
            "subagent entry indented under its tool_use: {line:?}"
        );
        assert!(
            line.contains("[code-reviewer]"),
            "subagent entry names its type: {line:?}"
        );
    }
    // …and the main conversation resumes at column 0 afterwards.
    assert!(lines[6].starts_with("#5 "), "{:?}", lines[6]);
}

#[test]
fn show_truncates_a_huge_payload_in_the_human_gist_but_never_in_json() {
    // The no-cap decision (`append_transcript`'s doc comment) says the STORE keeps every byte; the
    // human timeline is a rendering, and one `Write` of a large file must not scroll a whole
    // session off the screen. `--json` stays whole — it is the machine surface.
    let tmp = workspace();
    let big = "x".repeat(5_000);
    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-1"])
        .write_stdin(lines(&[
            json!({"kind":"tool_use","at":"2026-07-26T07:42:40.000Z","toolUseId":"toolu_1",
                   "data":{"name":"Write","input":{"content":big}}}),
        ]))
        .assert()
        .success();

    let out = nxc(&tmp)
        .args(["transcript", "show", "s-1"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    let entry_line = stdout.lines().nth(1).unwrap();
    assert!(
        entry_line.chars().count() < 200,
        "gist is bounded: {}",
        entry_line.chars().count()
    );
    assert!(
        entry_line.ends_with('…'),
        "and marked as truncated: {entry_line:?}"
    );

    let out = nxc(&tmp)
        .args(["--json", "transcript", "show", "s-1"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["entries"][0]["data"]["input"]["content"], json!(big));
}

#[test]
fn an_unrecognized_kind_round_trips_through_append_store_and_show_unchanged() {
    // PR #258 review, Test Quality #1 / Integrity & Robustness #3 (one gap, found twice). The store
    // accepts ANY non-empty `kind` on purpose — `transcript.rs`'s validate only rejects an empty
    // one — so that a NEWER sidecar emitting a kind this build has never heard of records it
    // instead of failing the batch and losing the whole turn's transcript. That forward-compat
    // promise is what makes a sidecar/binary version skew survivable, and nothing pinned it: every
    // other test uses one of the seven kinds the producer emits today.
    //
    // The unknown kind must survive all three hops unchanged — accepted by the CLI, stored as
    // itself, and rendered by `show` (nested under its parent when it carries subagent provenance,
    // since the read keys nesting on `parent_tool_use_id`, never on a kind allow-list).
    let tmp = workspace();
    nxc(&tmp)
        .args(["--json", "transcript", "append", "--session", "s-1"])
        .write_stdin(lines(&[
            json!({"kind":"tool_use","at":"2026-07-26T07:42:40.000Z","toolUseId":"toolu_task",
                   "data":{"name":"Task","input":{"prompt":"delegate"}}}),
            json!({"kind":"web_search_result","at":"2026-07-26T07:42:41.000Z",
                   "parentToolUseId":"toolu_task","subagentType":"general-purpose",
                   "data":{"query":"rust sqlite wal","hits":3}}),
            json!({"kind":"some_future_kind","at":"2026-07-26T07:42:42.000Z",
                   "data":{"anything":{"nested":[1,2,3]}}}),
        ]))
        .assert()
        .success();

    let rows = open_store(&tmp).transcript_rows("s-1").unwrap();
    assert_eq!(rows.len(), 3, "no unknown-kind row was rejected or dropped");
    assert_eq!(rows[1].entry.kind, "web_search_result");
    assert_eq!(rows[2].entry.kind, "some_future_kind");

    let out = nxc(&tmp)
        .args(["--json", "transcript", "show", "s-1"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    // The unknown SUBAGENT-tagged kind nests under its parent `tool_use`, payload intact.
    assert_eq!(v["entries"][0]["subagent"][0]["kind"], "web_search_result");
    assert_eq!(v["entries"][0]["subagent"][0]["data"]["hits"], 3);
    assert_eq!(
        v["entries"][0]["subagent"][0]["subagent_type"],
        "general-purpose"
    );
    // The unknown MAIN-conversation kind stays top level with its arbitrary payload whole.
    assert_eq!(v["entries"][1]["kind"], "some_future_kind");
    assert_eq!(
        v["entries"][1]["data"]["anything"]["nested"],
        json!([1, 2, 3])
    );
}

#[test]
fn an_oversized_batch_is_a_loud_validation_error_not_an_unbounded_read() {
    // PR #258 review, Integrity & Robustness #1. The store trusts the sidecar's producer-side caps
    // across a process boundary; without a backstop one pathological line is an unbounded
    // `read_to_string` that OOM-kills `nxc`, and since the sidecar's mid-run flush PROPAGATES, that
    // aborts the whole role turn with nothing diagnosable. Feed one byte past the limit and require
    // a structured `validation` error.
    //
    // The payload is deliberately NOT valid JSON: the size check has to fire before any parsing, so
    // an over-limit batch reports as over-limit rather than as a parse error on line 1. It is also
    // pure ASCII with no newline, i.e. one enormous "line" — the shape that would actually blow up.
    let tmp = workspace();
    let oversized = "a".repeat(64 * 1024 * 1024 + 1);
    let out = nxc(&tmp)
        .args(["--json", "transcript", "append", "--session", "s-1"])
        .write_stdin(oversized)
        .assert()
        .failure();
    // `--json` renders a failure as a structured record on STDOUT (the shape the sidecar and any
    // other machine caller parse), not as bare text on stderr.
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["error"]["kind"], "validation");
    let msg = v["error"]["msg"].as_str().unwrap();
    assert!(
        msg.contains("exceeds 67108864 bytes"),
        "names the limit it refused at: {msg}"
    );
    assert!(
        msg.contains("transcript.mjs"),
        "and points at where a real cap belongs: {msg}"
    );
    // Nothing was written — the batch is refused whole, never as a stored prefix.
    assert!(open_store(&tmp).transcript_rows("s-1").unwrap().is_empty());
}

// ---- `nxc transcript show --from-seq/--limit` (nxf 6j6v.t7pa) ---------------

#[test]
fn show_windows_the_read_from_a_cursor_and_stops_at_the_limit() {
    // The paging lever `facade::transcript`'s doc comment named and nothing implemented: a consumer
    // walks a long session forward instead of materializing every `tool_use` payload at once.
    let tmp = workspace();
    seed_a_subagent_transcript(&tmp); // seq 0..5

    let out = nxc(&tmp)
        .args([
            "--json",
            "transcript",
            "show",
            "s-1",
            "--from-seq",
            "1",
            "--limit",
            "2",
        ])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    let entries = v["entries"].as_array().unwrap();
    // seq 2 and 3 are both subagent entries whose parent (seq 1) is below the cursor, so they take
    // the orphan lane at top level rather than vanishing.
    assert_eq!(
        entries.iter().map(|e| e["seq"].clone()).collect::<Vec<_>>(),
        [json!(2), json!(3)]
    );
    assert_eq!(entries[0]["subagent_type"], "code-reviewer");
    assert_eq!(entries[0]["subagent"], json!([]));
    // …and it is the same value the seam hands an app, exactly as the un-windowed read is.
    assert_eq!(
        v,
        nexus_chat::facade::transcript_page(&open_store(&tmp), "s-1", 1, Some(2))
            .unwrap()
            .to_value()
    );
}

#[test]
fn show_without_the_window_flags_is_byte_identical_to_before() {
    // Additive, as the ticket's design required: the default read is the whole session, unchanged.
    let tmp = workspace();
    seed_a_subagent_transcript(&tmp);
    let windowed = nxc(&tmp)
        .args(["--json", "transcript", "show", "s-1", "--from-seq", "-1"])
        .assert()
        .success();
    let plain = nxc(&tmp)
        .args(["--json", "transcript", "show", "s-1"])
        .assert()
        .success();
    assert_eq!(
        json_of(&plain.get_output().stdout),
        json_of(&windowed.get_output().stdout)
    );
    assert_eq!(
        json_of(&plain.get_output().stdout)["entries"]
            .as_array()
            .unwrap()
            .len(),
        3,
        "three top-level entries, the Task's three children nested under one of them"
    );
}

// ---- `nxc transcript prune` (nxf 6j6v.t7pa) ---------------------------------

/// A one-entry transcript for `session`, stamped `at` — the shape retention decides on.
fn seed_dated(tmp: &TempDir, session: &str, at: &str) {
    nxc(tmp)
        .args(["transcript", "append", "--session", session])
        .write_stdin(lines(&[
            json!({"kind":"assistant","at":at,"data":{"text":"x"}}),
        ]))
        // Retention rides the first flush of a NEW session, and these seeds ARE first flushes — so
        // the seeding itself must not prune the sessions seeded before it.
        .env("NXC_TRANSCRIPT_KEEP_DAYS", "off")
        .assert()
        .success();
}

#[test]
fn prune_retires_the_stale_sessions_whole_and_reports_what_went() {
    let tmp = workspace();
    seed_dated(&tmp, "s-stale", "2026-05-01T00:00:00Z");
    seed_dated(&tmp, "s-fresh", "2026-07-25T00:00:00Z");

    let out = nxc(&tmp)
        .args(["--json", "transcript", "prune", "--keep-days", "30"])
        .assert()
        .success();
    // `NXC_NOW` is 2026-07-26, so the window reaches back to 2026-06-26.
    assert_eq!(
        json_of(&out.get_output().stdout),
        json!({ "sessions": ["s-stale"], "entries": 1, "undated": [], "dryRun": false })
    );
    let store = open_store(&tmp);
    assert!(store.transcript_rows("s-stale").unwrap().is_empty());
    assert_eq!(store.transcript_rows("s-fresh").unwrap().len(), 1);
}

#[test]
fn prune_dry_run_says_what_it_would_take_and_takes_nothing() {
    let tmp = workspace();
    seed_dated(&tmp, "s-stale", "2026-05-01T00:00:00Z");
    let out = nxc(&tmp)
        .args([
            "--json",
            "transcript",
            "prune",
            "--keep-days",
            "30",
            "--dry-run",
        ])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["sessions"], json!(["s-stale"]));
    assert_eq!(v["dryRun"], json!(true));
    assert_eq!(
        open_store(&tmp).transcript_rows("s-stale").unwrap().len(),
        1,
        "a dry run writes nothing"
    );
}

#[test]
fn prune_human_output_names_the_window_and_the_sessions_it_could_not_date() {
    let tmp = workspace();
    seed_dated(&tmp, "s-stale", "2026-05-01T00:00:00Z");
    // An entry with no `at` at all — legal on the wire, and of unknown age forever after.
    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-undated"])
        .write_stdin(lines(&[json!({"kind":"assistant","data":{"text":"x"}})]))
        .env("NXC_TRANSCRIPT_KEEP_DAYS", "off")
        .assert()
        .success();

    nxc(&tmp)
        .args(["transcript", "prune", "--keep-days", "30"])
        .assert()
        .success()
        .stdout(
            "pruned 1 transcript(s), 1 entries (keeping 30 days)\n  \
             s-stale\nkept 1 session(s) with no readable timestamp:\n  s-undated\n",
        );
}

#[test]
fn prune_with_retention_switched_off_and_no_flag_says_so_rather_than_doing_nothing() {
    // `off` is a legitimate answer for the automatic pass and a dead end for an operator who just
    // typed `prune`: silently pruning nothing would read as "nothing was stale".
    let tmp = workspace();
    seed_dated(&tmp, "s-stale", "2026-05-01T00:00:00Z");
    let out = nxc(&tmp)
        .args(["--json", "transcript", "prune"])
        .env("NXC_TRANSCRIPT_KEEP_DAYS", "off")
        .assert()
        .failure();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["error"]["kind"], "validation");
    assert!(v["error"]["msg"].as_str().unwrap().contains("--keep-days"));
    assert_eq!(
        open_store(&tmp).transcript_rows("s-stale").unwrap().len(),
        1
    );
}

#[test]
fn a_malformed_retention_setting_is_loud_on_the_command_and_silent_on_the_append() {
    // The one place the two paths deliberately differ. `prune` has an operator standing on it, so a
    // typo'd window is reported. `append` is the sidecar's mid-run flush, which propagates
    // everything and would abort the whole role turn — a housekeeping variable must not cost a
    // turn's evidence, so there the automatic prune is simply skipped.
    let tmp = workspace();
    seed_dated(&tmp, "s-stale", "2026-05-01T00:00:00Z");

    let out = nxc(&tmp)
        .args(["--json", "transcript", "prune"])
        .env("NXC_TRANSCRIPT_KEEP_DAYS", "thirty")
        .assert()
        .failure();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["error"]["kind"], "validation");
    assert!(v["error"]["msg"]
        .as_str()
        .unwrap()
        .contains("NXC_TRANSCRIPT_KEEP_DAYS"));

    // The same setting on the append path: the flush lands, and nothing is deleted.
    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-new"])
        .write_stdin(lines(&[
            json!({"kind":"assistant","at":"2026-07-26T00:00:00Z","data":{"text":"x"}}),
        ]))
        .env("NXC_TRANSCRIPT_KEEP_DAYS", "thirty")
        .assert()
        .success();
    let store = open_store(&tmp);
    assert_eq!(store.transcript_rows("s-new").unwrap().len(), 1, "flushed");
    assert_eq!(
        store.transcript_rows("s-stale").unwrap().len(),
        1,
        "and the prune was skipped rather than run on a guessed window"
    );
}

#[test]
fn a_new_sessions_first_flush_retires_the_stale_ones_with_no_operator_in_the_loop() {
    // The automatic half, end to end through the real sidecar callback: nobody ran `prune`, and the
    // table is bounded anyway. This is what makes the lever more than a command nobody remembers.
    let tmp = workspace();
    seed_dated(&tmp, "s-stale", "2026-05-01T00:00:00Z");
    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-new"])
        .write_stdin(lines(&[
            json!({"kind":"session_init","at":"2026-07-26T00:00:00Z","data":{}}),
        ]))
        .assert()
        .success();
    let store = open_store(&tmp);
    assert!(
        store.transcript_rows("s-stale").unwrap().is_empty(),
        "retired by the new session's first flush, at the default 30-day window"
    );
    assert_eq!(store.transcript_rows("s-new").unwrap().len(), 1);
}

#[test]
fn a_future_dated_stamp_on_the_wire_cannot_delete_the_workspaces_history() {
    // PR review, Integrity #1 — the regression test at the boundary the bad value actually crosses.
    // `at` is a string off a pipe, validated for RFC3339 shape and nothing else. When it was also
    // the reference instant for retention, one flush stamped in 2031 computed a cutoff in 2030 and
    // the automatic prune deleted every other session in the workspace. Reproduced exactly like this
    // on the shipped build before the fix; now the cutoff comes from the device's clock, so a stamp
    // can only make its OWN session look younger.
    let tmp = workspace();
    seed_dated(&tmp, "s-a", "2026-07-25T00:00:00Z");
    seed_dated(&tmp, "s-b", "2026-07-24T00:00:00Z");

    nxc(&tmp)
        .args(["transcript", "append", "--session", "s-skewed"])
        .write_stdin(lines(&[
            json!({"kind":"session_init","at":"2031-01-01T00:00:00Z","data":{}}),
        ]))
        .assert()
        .success();

    let store = open_store(&tmp);
    assert_eq!(store.transcript_rows("s-a").unwrap().len(), 1, "untouched");
    assert_eq!(store.transcript_rows("s-b").unwrap().len(), 1, "untouched");
    assert_eq!(store.transcript_rows("s-skewed").unwrap().len(), 1);
}

#[test]
fn a_corrupt_row_in_an_unrelated_session_does_not_abort_a_healthy_flush() {
    // PR review, Integrity #2 / Test Quality #1, end to end: the prune's read spans every session,
    // so an unreadable value in a long-dead one used to take down the turn of a healthy session that
    // merely happened to start next. The sidecar propagates this path's errors, so that abort was a
    // whole role turn lost to a chore.
    let tmp = workspace();
    {
        let store = open_store(&tmp);
        store
            .connection()
            .execute(
                "INSERT INTO agent_transcript(internal_session, seq, kind, at, data)
                 VALUES ('s-corrupt', 0, 'assistant', X'DEADBEEF', '{}')",
                [],
            )
            .unwrap();
    }
    nxc(&tmp)
        .args(["--json", "transcript", "append", "--session", "s-healthy"])
        .write_stdin(lines(&[
            json!({"kind":"session_init","at":"2026-07-26T00:00:00Z","data":{}}),
        ]))
        .assert()
        .success();
    assert_eq!(
        open_store(&tmp).transcript_rows("s-healthy").unwrap().len(),
        1
    );

    // …while the operator's own path reports it rather than hiding it.
    let out = nxc(&tmp)
        .args(["--json", "transcript", "prune", "--keep-days", "30"])
        .assert()
        .failure();
    assert!(!out.get_output().stdout.is_empty(), "says something");
}
