//! Integration tests for `nxf guide [topic]` (E7-S3): embedded, offline, agent-native
//! narrative docs. The command needs no workspace and must be pipe-safe (identical output
//! whether a TTY is attached or not), so these tests assert on captured (non-TTY) output.

use assert_cmd::Command;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

/// The fixed topic order is the agent contract; keep it in sync with `TOPICS` in the CLI.
const TOPICS: &[&str] = &[
    "getting-started",
    "mcp",
    "core-concepts",
    "commands",
    "plugins",
    "migration",
    "deferring-and-waiting",
    "running-a-relay",
];

#[test]
fn no_topic_lists_every_topic() {
    let out = nxf().arg("guide").output().expect("run");
    assert!(out.status.success(), "guide listing succeeds");
    let text = String::from_utf8(out.stdout).unwrap();
    for topic in TOPICS {
        assert!(text.contains(topic), "listing mentions '{topic}': {text}");
    }
}

#[test]
fn no_topic_json_is_deterministic_and_ordered() {
    // Run twice; the bytes must be identical (determinism is the agent contract).
    let a = nxf().args(["guide", "--json"]).output().expect("run a");
    let b = nxf().args(["guide", "--json"]).output().expect("run b");
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout, "--json output is byte-for-byte stable");

    let v: serde_json::Value = serde_json::from_slice(&a.stdout).expect("valid json array");
    let arr = v.as_array().expect("an array");
    let order: Vec<&str> = arr
        .iter()
        .map(|e| e["topic"].as_str().expect("topic string"))
        .collect();
    assert_eq!(order, TOPICS, "topics appear in the fixed order");
    for e in arr {
        assert!(
            e["summary"].as_str().is_some_and(|s| !s.is_empty()),
            "each entry has a non-empty summary: {e}"
        );
    }
}

#[test]
fn topic_prints_its_embedded_markdown() {
    let out = nxf()
        .args(["guide", "getting-started"])
        .output()
        .expect("run");
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    // The heading from the embedded file must surface (rendered markdown). The renderer
    // upcases h1, so compare case-insensitively rather than pin a specific casing.
    assert!(
        text.to_lowercase().contains("getting started"),
        "prints the topic heading: {text}"
    );
    // Pipe-safe: rendered output carries no fence markers or ANSI escapes.
    assert!(
        !text.contains("```"),
        "code fences are rendered away: {text}"
    );
    assert!(!text.contains('\u{1b}'), "no ANSI escapes: {text}");
}

#[test]
fn topic_output_is_pipe_safe_and_stable() {
    // Same invocation twice yields identical bytes; no TTY gating means piped == not piped.
    let a = nxf().args(["guide", "commands"]).output().expect("run a");
    let b = nxf().args(["guide", "commands"]).output().expect("run b");
    assert!(a.status.success() && b.status.success());
    assert_eq!(a.stdout, b.stdout, "rendered output is stable");
}

#[test]
fn topic_json_carries_raw_markdown() {
    let out = nxf()
        .args(["guide", "core-concepts", "--json"])
        .output()
        .expect("run");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid json");
    assert_eq!(v["topic"], "core-concepts");
    let content = v["content"].as_str().expect("content string");
    assert!(
        content.contains("# Core Concepts"),
        "raw markdown heading present: {content}"
    );
}

#[test]
fn unknown_topic_is_a_validation_error_listing_valid_topics() {
    let out = nxf()
        .args(["guide", "nope", "--json"])
        .output()
        .expect("run");
    assert!(!out.status.success(), "unknown topic fails");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("error envelope json");
    assert_eq!(v["error"]["kind"], "validation");
    let msg = v["error"]["msg"].as_str().unwrap();
    for topic in TOPICS {
        assert!(msg.contains(topic), "error lists '{topic}': {msg}");
    }
}
