//! The raw SDK outcome to the contract's cause, by fixture (§9). Each raw value here is what the sidecar writes for a
//! stream recorded on tv58's ticket, or for the transcript of 6j6v.cn4p.

use nexus_run_contracts::{EndCause, Timestamp};
use nexus_run_session_claude::causes::map_raw;
use serde_json::json;

const CANARY: &str = "SECRETCANARY";

#[test]
fn a_turn_that_succeeded_is_turn_complete() {
    let raw =
        json!({ "kind": "result", "subtype": "success", "is_error": false, "message": "done" });
    assert_eq!(map_raw(&raw), EndCause::TurnComplete);
}

/// A retryable failure that then went through: the stream carries the error, and the turn ended well.
#[test]
fn an_error_the_runtime_got_past_is_still_turn_complete() {
    let raw =
        json!({ "kind": "result", "subtype": "success", "is_error": false, "error": "rate_limit" });
    assert_eq!(map_raw(&raw), EndCause::TurnComplete);
}

/// 6j6v.cn4p's transcript: `error {message: rate_limit}`, `result isError 'You've hit your session limit · resets 7:30pm'`.
#[test]
fn the_session_limit_of_cn4ps_transcript_is_a_usage_limit() {
    let raw = json!({
        "kind": "result", "subtype": "success", "is_error": true, "error": "rate_limit",
        "message": "You've hit your session limit · resets 7:30pm"
    });
    assert_eq!(map_raw(&raw), EndCause::UsageLimit { resets_at: None });
}

#[test]
fn a_rejected_rate_limit_names_its_instant() {
    let raw = json!({
        "kind": "result", "is_error": true,
        "rate_limit": { "status": "rejected", "resetsAt": 1_800_000_000, "rateLimitType": "five_hour" }
    });
    assert_eq!(
        map_raw(&raw),
        EndCause::UsageLimit {
            resets_at: Some(Timestamp::from_unix_millis(1_800_000_000_000))
        }
    );
}

#[test]
fn an_overloaded_provider_is_unavailable() {
    let raw = json!({ "kind": "result", "subtype": "error_during_execution", "is_error": true, "error": "overloaded" });
    assert_eq!(map_raw(&raw), EndCause::ProviderUnavailable);
}

#[test]
fn a_stop_is_stopped_by_host_whatever_the_stream_said() {
    let raw = json!({ "kind": "stopped", "subtype": "error_during_execution", "is_error": true, "error": "overloaded" });
    assert_eq!(map_raw(&raw), EndCause::StoppedByHost);
}

#[test]
fn an_end_right_after_a_refusal_is_permission_refused() {
    let raw = json!({
        "kind": "result", "subtype": "success", "is_error": false,
        "refused": { "rule": "Bash", "program": "curl" }
    });
    assert_eq!(
        map_raw(&raw),
        EndCause::PermissionRefused {
            rule: "Bash".to_owned(),
            program: "curl".to_owned()
        }
    );
}

#[test]
fn what_nothing_maps_is_exit() {
    for raw in [
        json!({ "kind": "threw", "is_error": true, "message": "boom" }),
        json!({ "kind": "stream_ended", "is_error": false }),
        json!({ "kind": "result", "subtype": "error_max_turns", "is_error": true }),
        json!({ "kind": "something new" }),
        json!("not even an object"),
    ] {
        assert_eq!(map_raw(&raw), EndCause::Exit { status: None }, "{raw}");
    }
}

/// The contract's cause holds no text of the raw outcome, so nothing of it can reach the log.
#[test]
fn the_raw_message_reaches_no_cause() {
    for raw in [
        json!({ "kind": "result", "is_error": true, "error": "rate_limit", "message": CANARY }),
        json!({ "kind": "threw", "is_error": true, "message": CANARY }),
        json!({ "kind": "result", "is_error": false, "message": CANARY }),
    ] {
        let cause = serde_json::to_string(&map_raw(&raw)).unwrap();
        assert!(!cause.contains(CANARY), "{cause}");
    }
}
