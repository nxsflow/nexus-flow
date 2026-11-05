//! Contract goldens: one JSON file per contract under `tests/golden/`. Each file is compared byte
//! for byte after passing through `serde_json::Value`, so a timestamp format or a renamed field
//! cannot change unseen. Its keys sort because `Value` uses serde_json's default `BTreeMap`; a build
//! with `serde_json/preserve_order` turns these red, which is the point.

use std::collections::BTreeSet;

use nexus_run_contracts::status::{MEMBER_STATES, STEP_STATES, WAITING_FOR_FIXED};
use nexus_run_contracts::{
    Appended, Capabilities, Decision, Disposition, DrainedFact, Enqueued, Event, EventBody,
    FlowRef, Key, Liveness, LivenessPolicy, MessageId, Origin, ReasonCode, RunId, RunRow,
    RunStatus, Secs, SessionFact, Slot, StartOutcome, StartSpec, StoreError, StoreLimits, Timer,
    Timestamp, CONTRACTS_STATUS, CONTRACT_V, ENV_ALLOWLIST, LOCAL_TENANT,
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

/// `until` is when the run retries, clamped to the policy; `resets_at` is what the backend named. Only a
/// hold without one counts toward `max_holds`, and a writer from before the field reads as without one.
#[test]
fn a_hold_carries_the_reset_the_backend_named_apart_from_its_retry() {
    let held = events()
        .into_iter()
        .find_map(|e| match e.body {
            EventBody::Decision(Decision::Held {
                until, resets_at, ..
            }) => Some((until, resets_at)),
            _ => None,
        })
        .unwrap();
    assert_eq!(held.0.to_string(), "2026-11-04T10:10:00.500Z");
    assert_eq!(held.1.unwrap().to_string(), "2026-11-11T10:00:00.000Z");
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

/// Required from the first run, so no reader carries an "absent" branch; `local` is the slice-1 value.
#[test]
fn commissioned_and_the_run_row_carry_the_tenant_of_this_store() {
    let tenant = events()
        .into_iter()
        .find_map(|e| match e.body {
            EventBody::Commissioned { tenant, .. } => Some(tenant),
            _ => None,
        })
        .unwrap();
    assert_eq!(tenant, LOCAL_TENANT);
    assert_eq!(LOCAL_TENANT, "local");
    let storage: StorageGolden = assert_byte_exact("storage.json");
    assert_eq!(storage.run_row.tenant, LOCAL_TENANT);
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
fn a_timestamp_moves_forward_by_a_span() {
    let at = Timestamp::parse("2026-11-04T10:00:00.250Z").unwrap();
    assert_eq!(at.plus(Secs(90)).to_string(), "2026-11-04T10:01:30.250Z");
}

#[test]
fn a_huge_span_stops_at_the_last_instant_the_wire_can_write() {
    let last = "9999-12-31T23:59:59.999Z";
    let at = Timestamp::parse("2026-11-04T10:00:00.000Z").unwrap();
    for span in [u64::MAX, i64::MAX as u64, u64::MAX / 1000, 400_000_000_000] {
        assert_eq!(at.plus(Secs(span)).to_string(), last, "span {span}");
    }
    let at_the_end = Timestamp::parse(last).unwrap();
    assert_eq!(at_the_end.plus(Secs(1)).to_string(), last);
}

const FIRST_WRITABLE: &str = "0000-01-01T00:00:00.000Z";
const LAST_WRITABLE: &str = "9999-12-31T23:59:59.999Z";

#[test]
fn an_offset_that_pushes_the_instant_past_the_wire_form_is_refused() {
    for text in [
        "9999-12-31T23:59:59.999-01:00",
        "0000-01-01T00:00:00+01:00",
        "0000-01-01T00:59:59.999+01:00",
    ] {
        assert_eq!(Timestamp::parse(text), None, "{text}");
        let quoted = format!("\"{text}\"");
        assert!(
            serde_json::from_str::<Timestamp>(&quoted).is_err(),
            "{text}"
        );
    }
}

#[test]
fn the_first_and_the_last_writable_instant_round_trip() {
    for (read, written) in [
        (FIRST_WRITABLE, FIRST_WRITABLE),
        ("0000-01-01T01:00:00+01:00", FIRST_WRITABLE),
        (LAST_WRITABLE, LAST_WRITABLE),
        ("9999-12-31T22:59:59.999-01:00", "9999-12-31T23:59:59.999Z"),
    ] {
        let at = Timestamp::parse(read).unwrap_or_else(|| panic!("{read} was refused"));
        assert_eq!(at.to_string(), written, "from {read}");
        assert_eq!(Timestamp::parse(&at.to_string()), Some(at), "{read}");
    }
}

#[test]
fn millis_outside_the_wire_form_stop_at_its_ends() {
    let first = Timestamp::parse(FIRST_WRITABLE).unwrap();
    let last = Timestamp::parse(LAST_WRITABLE).unwrap();
    for (millis, end) in [
        (i64::MIN, first),
        (first.unix_millis() - 1, first),
        (last.unix_millis() + 1, last),
        (i64::MAX, last),
    ] {
        let at = Timestamp::from_unix_millis(millis);
        assert_eq!(at, end, "{millis}");
        assert_eq!(
            serde_json::to_value(at).unwrap(),
            end.to_string(),
            "{millis}"
        );
    }
    let inside = last.unix_millis() - 1;
    assert_eq!(Timestamp::from_unix_millis(inside).unix_millis(), inside);
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
        StoreError::DamagedRow {
            key: "input/r/1".into(),
            seq: None,
        },
    ];
    let names: Vec<_> = every_error.iter().map(StoreError::kind).collect();
    assert_eq!(golden.store_errors, names);
    assert_eq!(golden.store_error_precedence, StoreError::PRECEDENCE);
}

#[test]
fn the_precedence_names_the_kinds_of_its_variants() {
    let first_wins = [
        StoreError::Fenced,
        StoreError::SeqMismatch,
        StoreError::DuplicateKey { seq: 1 },
        StoreError::CasFailed { key: "k".into() },
    ];
    assert_eq!(
        first_wins.each_ref().map(StoreError::kind),
        StoreError::PRECEDENCE
    );
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
    let local = status.local.unwrap();
    assert_eq!(local.tree.in_flight.unwrap().step, "fetch");
    assert_eq!(
        local.owner.last_seen.unwrap().to_string(),
        "2026-11-04T10:00:00.000Z"
    );
    assert_eq!(local.observed_at.to_string(), "2026-11-04T10:00:12.000Z");
    assert!(WAITING_FOR_FIXED.contains(&status.waiting_for.unwrap().as_str()));
}

/// What a client answers with is the `question` an `answer` takes: the key of the event that asked, and
/// the two are not the same type by accident. `message` is the text a person reads before answering.
#[test]
fn needs_carries_the_key_an_answer_takes_and_the_message_apart() {
    let status: RunStatus = assert_byte_exact("run-status.json");
    let needs = status.needs.unwrap();
    let answer = events()
        .into_iter()
        .find_map(|e| match e.body {
            EventBody::Answer { question, .. } => Some(question),
            _ => None,
        })
        .unwrap();
    let question: Option<Key> = needs.question;
    assert_eq!(question, Some(answer));
    let message: Option<MessageId> = needs.message;
    assert!(message.unwrap().starts_with("m-"));
}

#[test]
fn every_state_word_in_the_golden_is_a_published_one() {
    let status: RunStatus = assert_byte_exact("run-status.json");
    for step in &status.steps {
        assert!(STEP_STATES.contains(&step.state.as_str()), "{}", step.state);
        for member in &step.members {
            assert!(
                MEMBER_STATES.contains(&member.state.as_str()),
                "{}",
                member.state
            );
        }
    }
}

/// Every key in a JSON value, however deep.
fn keys_in(value: &serde_json::Value, keys: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, inner) in map {
                keys.insert(key.clone());
                keys_in(inner, keys);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|item| keys_in(item, keys)),
        _ => {}
    }
}

/// The part that folds from the log and may leave the machine: commit and branch are evidence, opt-in
/// per stream, and a path carries the account's name (SY5, dqem.19wp, PV1).
#[test]
fn a_status_without_local_is_byte_exact_and_holds_nothing_read_from_the_machine() {
    let status: RunStatus = assert_byte_exact("run-status-portable.json");
    assert!(status.local.is_none());
    let wire = serde_json::to_value(&status).unwrap();
    let mut keys = BTreeSet::new();
    keys_in(&wire, &mut keys);
    for local_only in [
        "local",
        "tree",
        "commit",
        "branch",
        "path",
        "dirty",
        "owner",
        "pending_inputs",
        "observed_at",
        "quiet_for_s",
    ] {
        assert!(!keys.contains(local_only), "{local_only} is in {keys:?}");
    }
}

#[test]
fn the_two_status_goldens_differ_by_local_alone() {
    let read = |name: &str| serde_json::from_str::<serde_json::Value>(&golden(name)).unwrap();
    let mut whole = read("run-status.json");
    whole.as_object_mut().unwrap().remove("local");
    assert_eq!(whole, read("run-status-portable.json"));
}

#[test]
fn the_contracts_are_frozen_as_v1() {
    assert_eq!(CONTRACTS_STATUS, "frozen-v1");
    assert_eq!(CONTRACT_V, 1);
}
