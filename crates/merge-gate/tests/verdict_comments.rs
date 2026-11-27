//! NEX-172: the verdict is found in the pull request's comments (RFC 0010, "Where the verdict comes
//! from"): the newest comment, by `created_at` and then by `id`, whose author is in
//! `verdict_authors` and whose body holds the mark; its fenced `json` block after the mark is the
//! verdict, and an older comment is never a fallback.

use nexus_flow_merge_gate::verdict_comments::{verdict_from_comments, MARK};
use serde_json::{json, Value};

fn authors() -> Vec<String> {
    vec!["nexusflow-ug".to_string()]
}

fn body(verdict: &str) -> String {
    format!("{MARK}\n```json\n{verdict}\n```\nFor people: the gate ignores this.\n")
}

fn comment(id: u64, login: &str, created_at: &str, body: &str) -> Value {
    json!({ "id": id, "user": { "login": login }, "created_at": created_at, "body": body })
}

fn find(comments: Value) -> Result<String, Vec<String>> {
    verdict_from_comments(comments.to_string().as_bytes(), &authors())
}

#[test]
fn takes_the_json_block_after_the_mark() {
    let comments = json!([comment(
        1,
        "nexusflow-ug",
        "2026-11-27T10:00:00Z",
        &body(r#"{"format": 1}"#)
    )]);

    assert_eq!(find(comments).unwrap(), "{\"format\": 1}\n");
}

#[test]
fn a_verdict_from_a_login_outside_verdict_authors_is_ignored() {
    let comments = json!([
        comment(
            1,
            "nexusflow-ug",
            "2026-11-27T10:00:00Z",
            &body(r#"{"by": "agent"}"#)
        ),
        comment(
            2,
            "someone-else",
            "2026-11-27T11:00:00Z",
            &body(r#"{"by": "stranger"}"#)
        ),
    ]);

    assert_eq!(find(comments).unwrap(), "{\"by\": \"agent\"}\n");
}

#[test]
fn only_a_stranger_s_verdict_is_no_verdict() {
    let comments = json!([comment(
        2,
        "someone-else",
        "2026-11-27T11:00:00Z",
        &body("{}")
    )]);

    assert!(find(comments).is_err());
}

#[test]
fn authors_are_compared_without_regard_to_ascii_case() {
    let comments = json!([comment(
        1,
        "NexusFlow-UG",
        "2026-11-27T10:00:00Z",
        &body("{}")
    )]);

    assert_eq!(find(comments).unwrap(), "{}\n");
}

#[test]
fn the_newest_by_created_at_wins_whatever_the_order_in_the_array() {
    let comments = json!([
        comment(
            9,
            "nexusflow-ug",
            "2026-11-27T12:00:00Z",
            &body(r#"{"n": "newest"}"#)
        ),
        comment(
            10,
            "nexusflow-ug",
            "2026-11-27T10:00:00Z",
            &body(r#"{"n": "older"}"#)
        ),
    ]);

    assert_eq!(find(comments).unwrap(), "{\"n\": \"newest\"}\n");
}

#[test]
fn at_the_same_time_the_higher_id_wins() {
    let comments = json!([
        comment(
            11,
            "nexusflow-ug",
            "2026-11-27T10:00:00Z",
            &body(r#"{"n": 11}"#)
        ),
        comment(
            10,
            "nexusflow-ug",
            "2026-11-27T10:00:00Z",
            &body(r#"{"n": 10}"#)
        ),
    ]);

    assert_eq!(find(comments).unwrap(), "{\"n\": 11}\n");
}

#[test]
fn comments_without_the_mark_are_not_verdicts() {
    let comments = json!([
        comment(
            1,
            "nexusflow-ug",
            "2026-11-27T10:00:00Z",
            &body(r#"{"n": 1}"#)
        ),
        comment(
            2,
            "nexusflow-ug",
            "2026-11-27T11:00:00Z",
            "Looks good.\n```json\n{\"n\": 2}\n```\n"
        ),
    ]);

    assert_eq!(find(comments).unwrap(), "{\"n\": 1}\n");
}

#[test]
fn the_mark_may_carry_surrounding_whitespace() {
    let text = format!("Verdict:\n\n  {MARK}  \n\n```json\n{{}}\n```\n");
    let comments = json!([comment(1, "nexusflow-ug", "2026-11-27T10:00:00Z", &text)]);

    assert_eq!(find(comments).unwrap(), "{}\n");
}

#[test]
fn the_newest_without_a_json_block_is_invalid_and_no_older_one_is_taken() {
    let comments = json!([
        comment(1, "nexusflow-ug", "2026-11-27T10:00:00Z", &body("{}")),
        comment(
            2,
            "nexusflow-ug",
            "2026-11-27T11:00:00Z",
            &format!("{MARK}\nNo block here.\n")
        ),
    ]);

    let problems = find(comments).unwrap_err();

    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("comment 2"), "{problems:?}");
}

#[test]
fn a_block_with_another_info_string_is_not_the_verdict() {
    let text = format!("{MARK}\n```text\n{{}}\n```\n");
    let comments = json!([comment(1, "nexusflow-ug", "2026-11-27T10:00:00Z", &text)]);

    assert!(find(comments).is_err());
}

#[test]
fn a_json_block_after_other_text_is_not_the_verdict() {
    let text = format!("{MARK}\nSee below.\n\n```json\n{{}}\n```\n");
    let comments = json!([comment(1, "nexusflow-ug", "2026-11-27T10:00:00Z", &text)]);

    assert!(find(comments).is_err());
}

#[test]
fn a_mark_inside_a_code_block_is_not_the_mark() {
    let text = format!("```\n{MARK}\n```\n```json\n{{}}\n```\n");
    let comments = json!([comment(1, "nexusflow-ug", "2026-11-27T10:00:00Z", &text)]);

    assert!(find(comments).is_err());
}

#[test]
fn no_comments_is_no_verdict() {
    assert!(find(json!([])).is_err());
}

#[test]
fn comments_that_are_not_an_array_of_comments_are_no_verdict() {
    assert!(verdict_from_comments(b"not json", &authors()).is_err());
    assert!(find(json!({ "id": 1 })).is_err());
    assert!(find(json!([{ "id": 1, "body": "x" }])).is_err());
    assert!(find(json!([comment(
        1,
        "nexusflow-ug",
        "yesterday",
        &body("{}")
    )]))
    .is_err());
}
