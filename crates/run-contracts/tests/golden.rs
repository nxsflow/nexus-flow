//! Contract goldens: one JSON file per contract under `tests/golden/`. Each file is compared byte
//! for byte after passing through `serde_json::Value`, so a timestamp format or a renamed field
//! cannot change unseen. Its keys sort because `Value` uses serde_json's default `BTreeMap`; a build
//! with `serde_json/preserve_order` turns these red, which is the point.

use std::collections::BTreeSet;

use nexus_run_contracts::status::WAITING_FOR_FIXED;
use nexus_run_contracts::{
    Appended, Capabilities, Decision, Disposition, DrainedFact, Enqueued, Event, EventBody,
    FlowRef, Liveness, LivenessPolicy, Origin, ReasonCode, RunId, RunRow, RunStatus, Secs,
    SessionFact, Slot, StartOutcome, StartSpec, StoreError, StoreLimits, Timer, Timestamp,
    CONTRACTS_STATUS, ENV_ALLOWLIST,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[test]
fn run_events_golden_round_trips() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/run-events.json"
    ))
    .unwrap();
    let events: Vec<Event> = serde_json::from_str(&text).expect("golden parses");
    let back = serde_json::to_string_pretty(&events).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        serde_json::from_str::<serde_json::Value>(&back).unwrap()
    );
    assert!(events
        .iter()
        .any(|e| matches!(e.body, EventBody::TreeRefused { reason: _ })));
    assert_eq!(
        serde_json::to_string(&ReasonCode::TreeRefused).unwrap(),
        "\"tree_refused\""
    );
}

fn golden(name: &str) -> String {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Parses a golden into `T` and writes it back: the bytes must be the file's, end to end.
fn assert_byte_exact<T: Serialize + DeserializeOwned>(name: &str) -> T {
    let text = golden(name);
    let value: T = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let canonical = serde_json::to_value(&value).unwrap();
    let back = serde_json::to_string_pretty(&canonical).unwrap() + "\n";
    assert_eq!(back, text, "{name} changed on a round trip");
    value
}

fn events() -> Vec<Event> {
    assert_byte_exact("run-events.json")
}

/// The kind each golden event's body says it is, read from the file itself and not from the types.
fn golden_kinds() -> BTreeSet<String> {
    let text = golden("run-events.json");
    let raw: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap();
    raw.iter()
        .map(|e| e["body"]["kind"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn run_events_golden_is_byte_exact() {
    events();
}

#[test]
fn run_events_golden_holds_exactly_every_input_and_decision_kind() {
    let declared: BTreeSet<String> = EventBody::KINDS
        .iter()
        .chain(Decision::KINDS)
        .map(|k| (*k).to_owned())
        .collect();
    assert_eq!(golden_kinds(), declared);
}

#[test]
fn no_golden_event_falls_through_to_unknown() {
    for event in events() {
        assert!(
            !matches!(event.body, EventBody::Unknown(_)),
            "seq {} did not parse as its kind: {:?}",
            event.seq,
            event.body
        );
        if let EventBody::SessionFact { fact, .. } = &event.body {
            assert!(
                !matches!(fact, SessionFact::Unknown(_)),
                "seq {} holds a fact that did not parse: {fact:?}",
                event.seq
            );
        }
    }
}

#[test]
fn a_golden_decision_parses_as_a_decision_and_an_input_does_not() {
    let events = events();
    for event in &events {
        let is_decision = matches!(event.body, EventBody::Decision(_));
        assert_eq!(
            is_decision,
            event.origin == Origin::Reducer,
            "seq {}: only the reducer appends decisions",
            event.seq
        );
    }
}

#[test]
fn answer_carries_its_question_disposition_and_message() {
    let answers: Vec<_> = events()
        .into_iter()
        .filter_map(|e| match e.body {
            EventBody::Answer {
                question,
                disposition,
                message,
            } => Some((question, disposition, message)),
            _ => None,
        })
        .collect();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[0].1, Disposition::Continue);
    assert_eq!(answers[1].1, Disposition::Accept);
    assert_eq!(answers[0].0, "decision/39/1");
    assert!(answers[0].2.starts_with("m-"));
}

#[test]
fn root_opened_carries_the_thread_and_the_root_message() {
    let opened = events()
        .into_iter()
        .find_map(|e| match e.body {
            EventBody::RootOpened { thread, message } => Some((thread, message)),
            _ => None,
        })
        .expect("the golden holds root_opened");
    assert!(opened.0.starts_with("m-"));
    assert!(opened.1.starts_with("m-"));
    assert_ne!(opened.0, opened.1);
}

#[test]
fn the_policy_on_commissioned_is_the_section_6_5_table() {
    let policy = events()
        .into_iter()
        .find_map(|e| match e.body {
            EventBody::Commissioned { policy, .. } => Some(policy),
            _ => None,
        })
        .expect("the golden holds commissioned");
    assert_eq!(policy, LivenessPolicy::default());
    // Spelled out so a changed default cannot move the golden and the type together unseen.
    assert_eq!(policy.root_deadline, Secs(60));
    assert_eq!(policy.start_deadline, Secs(180));
    assert_eq!(policy.quiet_after, Secs(600));
    assert_eq!(policy.confirm_within, Secs(300));
    assert_eq!(policy.call_bound, Secs(10_800));
    assert_eq!(policy.subprocess_idle, Secs(1_800));
    assert_eq!(policy.stop_grace, Secs(300));
    assert_eq!(policy.tree_window, Secs(600));
    assert_eq!(policy.wake_jump, Secs(120));
    assert_eq!(policy.hold_retry, Secs(1_800));
    assert_eq!(policy.hold_retry_min, Secs(60));
    assert_eq!(policy.hold_retry_max, Secs(86_400));
    assert_eq!(policy.max_attempts, 3);
    assert_eq!(policy.max_holds, 5);
}

#[test]
fn a_timestamp_is_written_with_exactly_three_fractional_digits() {
    for (read, written) in [
        ("2026-11-04T10:00:00Z", "2026-11-04T10:00:00.000Z"),
        ("2026-11-04T10:00:00.5Z", "2026-11-04T10:00:00.500Z"),
        ("2026-11-04T10:00:00.123456789Z", "2026-11-04T10:00:00.123Z"),
        ("2026-11-04T11:00:00.250+01:00", "2026-11-04T10:00:00.250Z"),
    ] {
        let at = Timestamp::parse(read).unwrap();
        assert_eq!(at.to_string(), written, "from {read}");
        let again = Timestamp::parse(&at.to_string()).unwrap();
        assert_eq!(again, at, "{read} does not equal its own round trip");
    }
    assert_eq!(Timestamp::parse("yesterday"), None);
}

#[test]
fn a_timestamp_before_1970_keeps_its_millisecond() {
    let at = Timestamp::parse("1969-12-31T23:59:59.999Z").unwrap();
    assert_eq!(at.unix_millis(), -1);
    assert_eq!(at.to_string(), "1969-12-31T23:59:59.999Z");
}

#[test]
fn a_run_id_is_r_and_a_ulid() {
    assert!(RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").is_some());
    assert!(RunId::parse("01JAAAAAAAAAAAAAAAAAAAAAAA").is_none());
    assert!(RunId::parse("r-not-a-ulid").is_none());
    assert!(serde_json::from_str::<RunId>("\"r-nope\"").is_err());
}

#[test]
fn a_slot_names_its_session_key_in_the_specified_form() {
    let run = RunId::parse("r-01JAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
    let slot = Slot {
        step: "build".into(),
        visit: 2,
        member: "coder".into(),
        attempt: 3,
    };
    assert_eq!(
        slot.session_key(&run),
        "r-01JAAAAAAAAAAAAAAAAAAAAAAA/build/v2/coder/a3"
    );
}

#[test]
fn a_flow_ref_is_the_sha256_of_the_canonical_bytes() {
    assert_eq!(
        FlowRef::of_canonical_json(b"abc").hash,
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

/// Contract (b)'s wire shapes, and the names and order of its errors.
#[derive(Serialize, Deserialize)]
struct StorageGolden {
    appended: Appended,
    due_timer: Timer,
    enqueued: Vec<Enqueued>,
    limits_declared: StoreLimits,
    limits_sqlite: StoreLimits,
    run_row: RunRow,
    store_errors: Vec<String>,
    store_error_precedence: Vec<String>,
}

#[test]
fn storage_golden_is_byte_exact_and_pins_the_errors_and_their_order() {
    let golden: StorageGolden = assert_byte_exact("storage.json");
    assert_eq!(golden.limits_sqlite, StoreLimits::default());
    let every_error = [
        StoreError::Fenced,
        StoreError::SeqMismatch,
        StoreError::DuplicateKey { seq: 1 },
        StoreError::CasFailed { key: "k".into() },
        StoreError::VersionTooOld { max_v: 2 },
        StoreError::Busy,
        StoreError::Protected {
            key: "owner/r".into(),
        },
        StoreError::UnknownRun,
        StoreError::InvalidEvent { what: "w".into() },
        StoreError::Backend { what: "w".into() },
    ];
    let names: Vec<_> = every_error.iter().map(StoreError::kind).collect();
    assert_eq!(golden.store_errors, names);
    assert_eq!(golden.store_error_precedence, StoreError::PRECEDENCE);
}

#[derive(Serialize, Deserialize)]
struct SessionGolden {
    start_spec: StartSpec,
    env_allowlist: Vec<String>,
    capabilities: Capabilities,
    liveness: Vec<Liveness>,
    start_outcomes: Vec<StartOutcome>,
    drained_fact: DrainedFact,
}

#[test]
fn session_golden_is_byte_exact_and_pins_the_environment_allowlist() {
    let golden: SessionGolden = assert_byte_exact("session.json");
    assert_eq!(golden.env_allowlist, ENV_ALLOWLIST);
    assert_eq!(golden.start_spec.cwd, "/work/nexus-flow-v2");
    assert_eq!(golden.capabilities.capabilities.len(), 6);
}

#[test]
fn run_status_golden_is_byte_exact() {
    let status: RunStatus = assert_byte_exact("run-status.json");
    assert_eq!(status.tree.in_flight.unwrap().step, "fetch");
    assert_eq!(
        status.owner.last_seen.unwrap().to_string(),
        "2026-11-04T10:00:00.000Z"
    );
    assert!(WAITING_FOR_FIXED.contains(&status.waiting_for.unwrap().as_str()));
}

#[test]
fn the_contracts_are_still_drafts() {
    assert_eq!(CONTRACTS_STATUS, "draft");
}
