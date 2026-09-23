//! **A message body never has to pass through the shell** (nxf 6j6v.s46h).
//!
//! `nxc reply` and `nxc send` took the body only as an ARGUMENT, so every long text this system
//! produces — a review verdict with findings, a session report, an evidence trail of commands and
//! their output — reached the thread through `"<body>"`, where the shell evaluates backticks and
//! `$(…)` before `nxc` ever sees the bytes. At best the finding is silently mangled; at worst a
//! role that is QUOTING somebody else's text executes it.
//!
//! The notation is `nxf`'s, verbatim (`crates/foundation/src/text_input.rs` is the one
//! implementation both surfaces call): a body of `-` reads STDIN, `--body-file <path>` reads a
//! file, and a `--body-file` of `-` is the same sentinel. Nothing here is about escaping BETTER —
//! on both paths the text passes no quoting at all.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

const NOW: &str = "2026-09-11T10:00:00Z";

/// The text this whole item exists for: a backtick command substitution, a `$(…)` substitution,
/// double quotes, a single quote and a line break — every one of which either dies or detonates
/// inside `nxc reply --thread <id> "<this>"`.
const DANGEROUS: &str = "Finding: `rm -rf /` is quoted from the transcript, not run.\n\
                         The reviewer wrote \"$(whoami)\" and it must stay four words wide.\n\
                         Don't let the shell touch it.";

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    std::fs::create_dir_all(tmp.path().join(".nxs-personas")).unwrap();
    std::fs::write(
        tmp.path().join(".nxs-personas/pm.yaml"),
        "handle: pm\njob_title: Product manager\nsystem_prompt: You are the PM.\n",
    )
    .unwrap();
    tmp
}

/// The human at the keyboard, with the DRY worker: what is under test is the body's route from
/// STDIN into the thread, never a real spawn.
fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env_remove("NXC_SESSION")
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "carsten")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", NOW)
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", tmp.path().join("dry.log"));
    c
}

fn json_of(cmd: &mut Command) -> Value {
    let out = cmd.assert().success();
    serde_json::from_str(String::from_utf8_lossy(&out.get_output().stdout).trim())
        .expect("valid json")
}

/// Open a conversation with the declared `pm` and hand back its thread id.
fn open_thread(tmp: &TempDir) -> String {
    json_of(nxc(tmp).args([
        "--json",
        "send",
        "--to",
        "pm",
        "--no-ref",
        "please review the branch",
    ]))["thread_id"]
        .as_str()
        .expect("a thread id")
        .to_string()
}

/// Every body in `thread`, in order, as the thread's own board reports them.
fn bodies(tmp: &TempDir, thread: &str) -> Vec<String> {
    json_of(nxc(tmp).args(["--json", "threads", "show", thread]))["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|m| m["body"].as_str().expect("a body").to_string())
        .collect()
}

#[test]
fn a_reply_body_piped_on_stdin_reaches_the_thread_verbatim() {
    let tmp = workspace();
    let thread = open_thread(&tmp);
    nxc(&tmp)
        .args(["reply", "--thread", &thread, "-"])
        .write_stdin(DANGEROUS)
        .assert()
        .success();
    assert_eq!(
        bodies(&tmp, &thread).last().map(String::as_str),
        Some(DANGEROUS),
        "the piped text passes no quoting, so it arrives byte for byte"
    );
}

#[test]
fn a_reply_body_read_from_a_file_reaches_the_thread_verbatim() {
    let tmp = workspace();
    let thread = open_thread(&tmp);
    let path = tmp.path().join("verdict.md");
    std::fs::write(&path, DANGEROUS).unwrap();
    nxc(&tmp)
        .args([
            "reply",
            "--thread",
            &thread,
            "--body-file",
            path.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert_eq!(
        bodies(&tmp, &thread).last().map(String::as_str),
        Some(DANGEROUS)
    );
}

#[test]
fn a_send_body_piped_on_stdin_opens_the_thread_verbatim() {
    let tmp = workspace();
    let opened = json_of(
        nxc(&tmp)
            .args(["--json", "send", "--to", "pm", "--no-ref", "-"])
            .write_stdin(DANGEROUS),
    );
    let thread = opened["thread_id"].as_str().expect("a thread id");
    assert_eq!(bodies(&tmp, thread), vec![DANGEROUS.to_string()]);
}

#[test]
fn a_send_body_read_from_a_file_opens_the_thread_verbatim() {
    let tmp = workspace();
    let path = tmp.path().join("brief.md");
    std::fs::write(&path, DANGEROUS).unwrap();
    let opened = json_of(nxc(&tmp).args([
        "--json",
        "send",
        "--to",
        "pm",
        "--no-ref",
        "--body-file",
        path.to_str().unwrap(),
    ]));
    let thread = opened["thread_id"].as_str().expect("a thread id");
    assert_eq!(bodies(&tmp, thread), vec![DANGEROUS.to_string()]);
}

/// DoD 4: the short form is what a one-line answer should still cost, and nothing about it moves.
#[test]
fn the_argument_form_still_answers_in_one_token() {
    let tmp = workspace();
    let thread = open_thread(&tmp);
    nxc(&tmp)
        .args(["reply", "--thread", &thread, "lgtm"])
        .assert()
        .success();
    assert_eq!(
        bodies(&tmp, &thread).last().map(String::as_str),
        Some("lgtm")
    );
}

/// Two sources for one body is a caller that does not know what it is sending — refused BEFORE the
/// write, so the thread is untouched either way.
#[test]
fn a_body_given_twice_is_refused_and_nothing_is_posted() {
    let tmp = workspace();
    let thread = open_thread(&tmp);
    let path = tmp.path().join("verdict.md");
    std::fs::write(&path, DANGEROUS).unwrap();
    let out = nxc(&tmp)
        .args([
            "--json",
            "reply",
            "--thread",
            &thread,
            "lgtm",
            "--body-file",
            path.to_str().unwrap(),
        ])
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let err: Value = serde_json::from_str(stdout.trim()).expect("the structured error envelope");
    assert_eq!(err["error"]["kind"], "validation", "{stdout}");
    assert!(
        err["error"]["msg"]
            .as_str()
            .expect("a message")
            .contains("more than one source"),
        "the refusal says what collided: {stdout}"
    );
    assert_eq!(
        bodies(&tmp, &thread).len(),
        1,
        "only the opening message is there"
    );
}

/// A body given on neither source is still a usage error, and it names the file flag as the way
/// out — `send`'s and `reply`'s positional is no longer unconditionally required, so this is the
/// one refusal that could have gone quiet.
#[test]
fn a_missing_body_is_still_refused() {
    let tmp = workspace();
    let thread = open_thread(&tmp);
    let out = nxc(&tmp)
        .args(["reply", "--thread", &thread])
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).into_owned();
    assert!(
        err.contains("BODY") || err.contains("body"),
        "clap names the missing body: {err}"
    );
}

/// **The one trap the sentinel brings with it, closed** (nxf 6j6v.s46h).
///
/// Every text that names the STDIN form shows it with the heredoc that feeds it — but a line that
/// LOOKS runnable gets copied and run, and `nxc reply --thread <id> -` with nothing on STDIN would
/// otherwise post an empty answer, discharge the obligation and end the round with no result.
/// Refused, so the mis-copy costs a message on stderr instead of a lost round.
///
/// Only on the STDIN/file path: an empty ARGUMENT is typed on purpose and is not this change's to
/// judge.
#[test]
fn an_empty_body_from_stdin_is_refused_and_nothing_is_posted() {
    let tmp = workspace();
    let thread = open_thread(&tmp);
    let out = nxc(&tmp)
        .args(["--json", "reply", "--thread", &thread, "-"])
        .write_stdin("   \n")
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let err: Value = serde_json::from_str(stdout.trim()).expect("the structured error envelope");
    assert_eq!(err["error"]["kind"], "validation", "{stdout}");
    assert!(
        err["error"]["msg"]
            .as_str()
            .expect("a message")
            .contains("empty"),
        "the refusal says the body is empty: {stdout}"
    );
    assert_eq!(bodies(&tmp, &thread).len(), 1, "nothing was posted");
}

/// The same guard on the file half — an empty `--body-file` is the same lost round.
#[test]
fn an_empty_body_file_is_refused() {
    let tmp = workspace();
    let thread = open_thread(&tmp);
    let path = tmp.path().join("empty.md");
    std::fs::write(&path, "").unwrap();
    let out = nxc(&tmp)
        .args([
            "--json",
            "reply",
            "--thread",
            &thread,
            "--body-file",
            path.to_str().unwrap(),
        ])
        .assert()
        .failure();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    let err: Value = serde_json::from_str(stdout.trim()).expect("the structured error envelope");
    assert_eq!(err["error"]["kind"], "validation", "{stdout}");
    assert_eq!(bodies(&tmp, &thread).len(), 1, "nothing was posted");
}

/// **The OTHER half of the guard's declared scope, and it was unpinned** (PR #463 review, Test
/// Quality · Medium).
///
/// `cli.rs::resolve_body` refuses an empty read on the STDIN/file path and says, in the same
/// breath, that an empty ARGUMENT is left alone: it is typed on purpose, it behaved this way
/// before nxf 6j6v.s46h, and refusing it is a different decision than the one that item made.
/// Nothing asserted the second half, so a later tightening of the guard to cover arguments too —
/// one word, `reads_a_stream &&` deleted — would have passed every test in this file in silence.
///
/// Verified to be a real guard rather than a tautology: dropping `reads_a_stream &&` from the
/// condition reds this test and nothing else.
#[test]
fn an_empty_body_given_as_an_argument_is_still_posted() {
    let tmp = workspace();
    let thread = open_thread(&tmp);
    nxc(&tmp)
        .args(["reply", "--thread", &thread, ""])
        .assert()
        .success();
    assert_eq!(
        bodies(&tmp, &thread).last().map(String::as_str),
        Some(""),
        "an empty argument is the caller's own choice and reaches the thread as one"
    );
}
