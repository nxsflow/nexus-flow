//! Additive evolution: a reader of this version keeps what it does not know, and tells an unknown
//! input (which folds to nothing) from an unknown decision (which it must refuse to fold past).
//! Adding a decision kind changes the variant list AND the contract version.

use nexus_run_contracts::status::{NeedsKind, RunState, Verdict, MEMBER_STATES, STEP_STATES};
use nexus_run_contracts::{
    Decision, Disposition, EndCause, Event, EventBody, GitReason, HeartbeatKind, Origin, Outcome,
    ParkOutcome, ReasonCode, RunStatus, SequenceKind, SessionFact, StepOutcome, StopKind,
    TimerPurpose, TreeRefusal, UnknownBody, WithdrawnBy, CONTRACT_V,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

fn golden_events() -> Vec<Value> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/run-events.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn round_trip(raw: &Value) -> (Event, Value) {
    let event: Event = serde_json::from_value(raw.clone()).expect("the reader accepts it");
    let back = serde_json::to_value(&event).unwrap();
    (event, back)
}

fn envelope(origin: &str, body: Value) -> Value {
    json!({
        "v": 2,
        "run": "r-01JAAAAAAAAAAAAAAAAAAAAAAA",
        "seq": 9,
        "at": "2026-11-04T10:00:00.000Z",
        "key": "k/9",
        "origin": origin,
        "body": body,
    })
}

#[test]
fn an_unknown_envelope_field_on_any_golden_event_survives_the_round_trip() {
    for mut raw in golden_events() {
        raw["from_the_future"] = json!({ "nested": [1, 2] });
        let (event, back) = round_trip(&raw);
        assert_eq!(back, raw);
        assert_eq!(event.extra["from_the_future"], json!({ "nested": [1, 2] }));
    }
}

#[test]
fn an_unknown_body_field_on_any_golden_event_survives_the_round_trip() {
    for mut raw in golden_events() {
        raw["body"]["from_the_future"] = json!("kept");
        let (event, back) = round_trip(&raw);
        assert_eq!(back, raw);
        assert!(
            !matches!(event.body, EventBody::Unknown(_)),
            "a new field must not turn a known kind into an unknown one: {:?}",
            event.body
        );
    }
}

#[test]
fn an_unknown_field_deep_in_a_known_body_survives_the_round_trip() {
    let mut raw = golden_events()
        .into_iter()
        .find(|e| e["body"]["kind"] == "commissioned")
        .unwrap();
    raw["body"]["policy"]["a_later_duration"] = json!(5);
    let (event, back) = round_trip(&raw);
    assert_eq!(back, raw);
    assert!(matches!(event.body, EventBody::Commissioned { .. }));
}

#[test]
fn the_extra_map_is_emitted_sorted_not_in_insertion_order() {
    let mut raw = golden_events().remove(0);
    raw["zz_late"] = json!(1);
    raw["aa_early"] = json!(2);
    let (event, _) = round_trip(&raw);
    let text = serde_json::to_string(&event).unwrap();
    assert!(text.find("aa_early").unwrap() < text.find("zz_late").unwrap());
}

#[test]
fn an_unknown_input_kind_survives_and_reads_as_an_input() {
    let raw = envelope("cli", json!({ "kind": "follow_up", "message": "m-1" }));
    let (event, back) = round_trip(&raw);
    assert_eq!(back, raw);
    assert!(event.is_unknown_input());
    assert!(!event.is_unknown_decision());
}

#[test]
fn an_unknown_decision_kind_survives_and_reads_as_a_decision() {
    let raw = envelope(
        "reducer",
        json!({ "kind": "step_skipped", "step": "verify" }),
    );
    let (event, back) = round_trip(&raw);
    assert_eq!(back, raw);
    assert!(event.is_unknown_decision());
    assert!(!event.is_unknown_input());
}

#[test]
fn a_known_kind_is_neither_unknown_input_nor_unknown_decision() {
    for raw in golden_events() {
        let (event, _) = round_trip(&raw);
        assert!(!event.is_unknown_input() && !event.is_unknown_decision());
    }
}

#[test]
fn a_body_with_no_kind_is_refused_rather_than_kept() {
    let raw = envelope("cli", json!({ "message": "m-1" }));
    assert!(serde_json::from_value::<Event>(raw).is_err());
}

/// The fields a body may leave out: `Option`s, which read an absent field as `None`.
const OPTIONAL_FIELDS: &[(&str, &str)] = &[
    ("tree_prepared", "found"),
    ("resume", "slot"),
    ("withdrawn", "message"),
    ("run_queued", "behind"),
    ("parked", "slot"),
    ("finishing", "reason"),
    ("finished", "reason"),
];

/// A value no field of any body can read: a struct takes an array too, but not this first element.
fn mistyped() -> Value {
    json!([[]])
}

fn body_fields(raw: &Value) -> Vec<String> {
    raw["body"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|field| *field != "kind")
        .cloned()
        .collect()
}

/// The golden holds exactly every kind, which `golden.rs` proves, so this walks every kind there is.
#[test]
fn a_known_kind_with_a_missing_required_field_is_refused_naming_the_field() {
    for raw in golden_events() {
        let kind = raw["body"]["kind"].as_str().unwrap().to_owned();
        for field in body_fields(&raw) {
            if OPTIONAL_FIELDS.contains(&(kind.as_str(), field.as_str())) {
                continue;
            }
            let mut damaged = raw.clone();
            damaged["body"].as_object_mut().unwrap().remove(&field);
            let refusal = serde_json::from_value::<Event>(damaged)
                .map(|event| format!("{:?}", event.body))
                .expect_err(&format!("{kind} without {field} was accepted"));
            assert!(
                refusal.to_string().contains(&format!("`{field}`")),
                "{kind} without {field}: {refusal}"
            );
        }
    }
}

#[test]
fn a_known_kind_with_a_mistyped_field_is_refused() {
    for raw in golden_events() {
        let kind = raw["body"]["kind"].as_str().unwrap().to_owned();
        for field in body_fields(&raw) {
            let mut damaged = raw.clone();
            damaged["body"][&field] = mistyped();
            let read = serde_json::from_value::<Event>(damaged);
            assert!(read.is_err(), "{kind} with a bad {field}: {read:?}");
        }
    }
}

/// The kinds the two walks above cannot damage, because they carry no field at all.
#[test]
fn only_the_kinds_without_a_field_escape_the_damage_walks() {
    let empty: Vec<String> = golden_events()
        .iter()
        .filter(|raw| body_fields(raw).is_empty())
        .map(|raw| raw["body"]["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        empty,
        [
            "root_requested",
            "claim_requested",
            "claim_taken",
            "resume_run",
            "withdraw_forced",
            "claim_released"
        ]
    );
}

#[test]
fn a_damaged_answer_is_refused_rather_than_folded_to_nothing() {
    let raw = envelope("cli", json!({ "kind": "answer", "question": "q" }));
    let refusal = serde_json::from_value::<Event>(raw).unwrap_err();
    assert!(refusal.to_string().contains("disposition"), "{refusal}");
}

#[test]
fn every_fact_kind_is_in_the_damage_walk() {
    let walked: Vec<Value> = every_fact()
        .iter()
        .map(|fact| fact["kind"].clone())
        .collect();
    assert_eq!(walked, SessionFact::KINDS);
}

/// One whole fact of every kind.
fn every_fact() -> Vec<Value> {
    vec![
        json!({ "kind": "started", "backend_session_id": "b", "pid": 7 }),
        json!({ "kind": "heartbeat", "beat": "model_output" }),
        json!({
            "kind": "usage",
            "model": "m",
            "input_tokens": 1,
            "output_tokens": 2,
            "cache_read_tokens": 3,
            "cache_write_tokens": 4,
            "turns": 5,
        }),
        json!({ "kind": "permission_refused", "rule": "r", "program": "p" }),
        json!({ "kind": "group_lingering", "pids": [7] }),
        json!({ "kind": "ended", "cause": { "code": "lost" } }),
    ]
}

fn fact_event(fact: Value) -> Value {
    envelope(
        "adapter",
        json!({ "kind": "session_fact", "session_key": "r/build/v1/coder/a1", "fact": fact }),
    )
}

#[test]
fn a_known_fact_kind_with_a_damaged_field_is_refused() {
    for fact in every_fact() {
        let kind = fact["kind"].as_str().unwrap().to_owned();
        let (event, _) = round_trip(&fact_event(fact.clone()));
        assert!(
            matches!(&event.body, EventBody::SessionFact { fact, .. } if !matches!(fact, SessionFact::Unknown(_))),
            "{kind} did not read as its kind"
        );
        for (field, _) in fact
            .as_object()
            .unwrap()
            .iter()
            .filter(|(f, _)| *f != "kind")
        {
            let mut bad = fact.clone();
            bad[field] = mistyped();
            let read = serde_json::from_value::<Event>(fact_event(bad));
            assert!(read.is_err(), "{kind} with a bad {field}: {read:?}");
            if (kind.as_str(), field.as_str()) == ("started", "pid") {
                continue;
            }
            let mut without = fact.clone();
            without.as_object_mut().unwrap().remove(field);
            let refusal = serde_json::from_value::<Event>(fact_event(without))
                .map(|event| format!("{:?}", event.body))
                .expect_err(&format!("{kind} without {field} was accepted"));
            assert!(
                refusal.to_string().contains(&format!("`{field}`")),
                "{kind} without {field}: {refusal}"
            );
        }
    }
}

#[test]
fn an_unknown_session_fact_kind_survives_inside_a_known_event() {
    let raw = envelope(
        "adapter",
        json!({
            "kind": "session_fact",
            "session_key": "r/build/v1/coder/a1",
            "fact": { "kind": "a_later_fact", "n": 1 },
        }),
    );
    let (event, back) = round_trip(&raw);
    assert_eq!(back, raw);
    assert!(matches!(event.body, EventBody::SessionFact { .. }));
}

#[test]
fn a_code_from_a_newer_version_reads_as_unknown_and_is_in_no_list() {
    let code: ReasonCode = serde_json::from_str("\"a_later_reason\"").unwrap();
    assert_eq!(code, ReasonCode::Unknown("a_later_reason".into()));
    assert_eq!(serde_json::to_string(&code).unwrap(), "\"a_later_reason\"");
    assert!(!ReasonCode::ALL.contains(&"a_later_reason"));
    let git: GitReason = serde_json::from_str("\"a_later_git\"").unwrap();
    assert_eq!(git, GitReason::Unknown("a_later_git".into()));
    assert!(!GitReason::ALL.contains(&"a_later_git"));
}

/// What each contract version shipped of the things a reader cannot fold past: the decision kinds, and
/// the values of the enums only decisions carry.
#[derive(Deserialize)]
struct History {
    versions: Vec<Pin>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct Pin {
    v: u32,
    decision_kinds: Vec<String>,
    decision_values: DecisionValues,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct DecisionValues {
    outcome: Vec<String>,
    step_outcome: Vec<String>,
    timer_purpose: Vec<String>,
}

fn history() -> Vec<Pin> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/decision-kinds.json"
    );
    let history: History = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    history.versions
}

/// The rule: the code's lists are the lists pinned for the code's version, and a version's lists differ
/// from the ones before it, so a new kind or value has to come with a new version.
fn check_pins(code: &Pin, history: &[Pin]) -> Result<(), String> {
    let v = code.v;
    let pin = history
        .iter()
        .find(|pin| pin.v == v)
        .ok_or_else(|| format!("CONTRACT_V {v} has no entry in decision-kinds.json"))?;
    if pin != code {
        return Err(format!(
            "the decision kinds or values differ from the lists pinned for v{v}: raise CONTRACT_V \
             and add a version to decision-kinds.json"
        ));
    }
    for pair in history.windows(2) {
        let same = pair[1].decision_kinds == pair[0].decision_kinds
            && pair[1].decision_values == pair[0].decision_values;
        if pair[1].v != pair[0].v + 1 || same {
            return Err(format!(
                "v{} does not change the lists of v{}",
                pair[1].v, pair[0].v
            ));
        }
    }
    Ok(())
}

fn owned(list: &[&str]) -> Vec<String> {
    list.iter().map(|value| (*value).to_owned()).collect()
}

/// The lists of this code, as what they would be pinned as.
fn current() -> Pin {
    Pin {
        v: CONTRACT_V,
        decision_kinds: owned(Decision::KINDS),
        decision_values: DecisionValues {
            outcome: owned(Outcome::ALL),
            step_outcome: owned(StepOutcome::ALL),
            timer_purpose: owned(TimerPurpose::ALL),
        },
    }
}

fn with_new_kind() -> Pin {
    let mut pin = current();
    pin.decision_kinds.push("step_skipped".into());
    pin
}

#[test]
fn the_decision_kinds_and_values_are_the_lists_pinned_for_contract_v() {
    check_pins(&current(), &history()).unwrap();
}

#[test]
fn a_new_decision_kind_under_the_same_version_is_refused() {
    let refusal = check_pins(&with_new_kind(), &history()).unwrap_err();
    assert!(refusal.contains("raise CONTRACT_V"), "{refusal}");
}

#[test]
fn a_new_value_of_a_decision_enum_under_the_same_version_is_refused() {
    for change in [
        |values: &mut DecisionValues| values.outcome.push(LATER.into()),
        |values: &mut DecisionValues| values.step_outcome.push(LATER.into()),
        |values: &mut DecisionValues| values.timer_purpose.push(LATER.into()),
    ] {
        let mut pin = current();
        change(&mut pin.decision_values);
        let refusal = check_pins(&pin, &history()).unwrap_err();
        assert!(refusal.contains("raise CONTRACT_V"), "{refusal}");
    }
}

#[test]
fn a_new_decision_kind_with_a_raised_version_and_no_pin_is_refused() {
    let mut pin = with_new_kind();
    pin.v = CONTRACT_V + 1;
    let refusal = check_pins(&pin, &history()).unwrap_err();
    assert!(refusal.contains("no entry"), "{refusal}");
}

#[test]
fn a_raised_version_that_keeps_the_lists_is_refused() {
    let mut pin = current();
    pin.v = CONTRACT_V + 1;
    let mut pins = history();
    pins.push(pin.clone());
    assert!(check_pins(&pin, &pins).is_err());
}

#[test]
fn a_new_decision_kind_with_a_raised_version_and_its_pin_is_accepted() {
    let mut pin = with_new_kind();
    pin.v = CONTRACT_V + 1;
    let mut pins = history();
    pins.push(pin.clone());
    check_pins(&pin, &pins).unwrap();
}

/// A value no version of this contract has written.
const LATER: &str = "a_later_value";

fn later_code() -> Value {
    json!(LATER)
}

fn later_tagged(tag: &str) -> Value {
    json!({ tag: LATER, "from_the_future": 1 })
}

/// Every place the log carries a closed enum, as (the body's kind, where in the event, an unknown value
/// of it).
fn log_positions() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("commissioned", "/origin", later_code()),
        ("stop_signalled", "/body/signal", later_code()),
        ("answer", "/body/disposition", later_code()),
        ("withdrawn", "/body/by", later_code()),
        ("step_finished", "/body/outcome", later_code()),
        ("finishing", "/body/outcome", later_code()),
        ("finished", "/body/outcome", later_code()),
        ("timer_set", "/body/purpose", later_code()),
        ("tree_prepared", "/body/found", later_tagged("kind")),
        ("tree_parked", "/body/outcome", later_tagged("kind")),
        ("tree_refused", "/body/reason", later_tagged("code")),
        (
            "tree_refused",
            "/body/reason",
            json!({ "code": "mid_sequence", "what": LATER }),
        ),
        (
            "tree_refused",
            "/body/reason",
            json!({ "code": "git", "reason": LATER }),
        ),
        ("session_fact", "/body/fact/beat", later_code()),
        ("session_fact", "/body/fact/cause", later_tagged("code")),
    ]
}

/// The golden event of this kind that has `pointer`, with the value there replaced.
fn golden_with(kind: &str, pointer: &str, value: &Value) -> Value {
    let mut raw = golden_events()
        .into_iter()
        .find(|e| e["body"]["kind"] == kind && e.pointer(pointer).is_some())
        .unwrap_or_else(|| panic!("the golden holds no {kind} with {pointer}"));
    *raw.pointer_mut(pointer).unwrap() = value.clone();
    raw
}

#[test]
fn an_unknown_value_of_any_enum_in_the_log_is_read_and_kept_on_a_round_trip() {
    for (kind, pointer, value) in log_positions() {
        let raw = golden_with(kind, pointer, &value);
        let event: Result<Event, _> = serde_json::from_value(raw.clone());
        let event = event.unwrap_or_else(|e| panic!("{kind} {pointer} {value}: {e}"));
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            raw,
            "{kind} {pointer}"
        );
    }
}

/// An unknown value is a string, or an object with its tag: anything else is damage, not the future.
#[test]
fn a_mistyped_enum_value_stays_refused() {
    for (kind, pointer, value) in log_positions() {
        let damaged = if value.is_string() {
            json!(7)
        } else {
            json!({ "no_tag_here": 1 })
        };
        let raw = golden_with(kind, pointer, &damaged);
        let read = serde_json::from_value::<Event>(raw);
        assert!(read.is_err(), "{kind} {pointer}: {read:?}");
    }
}

fn golden_status() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/run-status.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every place status carries a closed enum, beyond the ones the log already walks.
fn status_positions() -> Vec<(&'static str, Value)> {
    vec![
        ("/state", later_tagged("name")),
        ("/needs/kind", later_code()),
        ("/outcome", later_code()),
        ("/steps/0/outcome", later_code()),
        ("/steps/0/members/0/liveness/verdict", later_code()),
    ]
}

#[test]
fn an_unknown_value_of_any_enum_in_status_is_read_and_kept_on_a_round_trip() {
    for (pointer, value) in status_positions() {
        let mut raw = golden_status();
        *raw.pointer_mut(pointer)
            .unwrap_or_else(|| panic!("the golden has no {pointer}")) = value;
        let status: Result<RunStatus, _> = serde_json::from_value(raw.clone());
        let status = status.unwrap_or_else(|e| panic!("{pointer}: {e}"));
        assert_eq!(serde_json::to_value(&status).unwrap(), raw, "{pointer}");
    }
}

#[test]
fn a_status_reader_never_refuses_on_a_higher_v() {
    let mut raw = golden_status();
    raw["v"] = json!(CONTRACT_V + 7);
    raw["a_field_of_that_version"] = json!({ "n": 1 });
    let status: RunStatus = serde_json::from_value(raw).expect("a reader renders what it knows");
    assert_eq!(status.v, CONTRACT_V + 7);
}

/// A list of words: its wire form, what it does not know, and that a non-word is still damage.
fn assert_a_word_list<T>(name: &str, all: &[&str], known: fn(&T) -> bool, unknown: T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    for word in all {
        let read: T = serde_json::from_value(json!(word)).unwrap();
        assert!(known(&read), "{name}: {word} read as {read:?}");
        assert_eq!(serde_json::to_value(&read).unwrap(), json!(word), "{name}");
    }
    let read: T = serde_json::from_value(later_code()).unwrap();
    assert_eq!(read, unknown, "{name}");
    assert_eq!(serde_json::to_value(&read).unwrap(), later_code(), "{name}");
    assert!(serde_json::from_value::<T>(json!(7)).is_err(), "{name}");
}

macro_rules! word_lists {
    ($($name:ident),+ $(,)?) => {
        $(assert_a_word_list::<$name>(
            stringify!($name),
            $name::ALL,
            |read| !matches!(read, $name::Unknown(_)),
            $name::Unknown(LATER.to_owned()),
        );)+
    };
}

#[test]
fn every_word_list_keeps_its_wire_form_and_reads_what_it_does_not_know() {
    word_lists![
        Origin,
        Disposition,
        WithdrawnBy,
        StopKind,
        StepOutcome,
        Outcome,
        TimerPurpose,
        SequenceKind,
        HeartbeatKind,
        Verdict,
        NeedsKind,
        ReasonCode,
        GitReason,
    ];
}

fn assert_a_tagged_list<T>(
    name: &str,
    tag: &str,
    samples: Vec<T>,
    all: &[&str],
    unknown: fn(UnknownBody) -> T,
) where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let written: Vec<Value> = samples
        .iter()
        .map(|sample| serde_json::to_value(sample).unwrap()[tag].clone())
        .collect();
    assert_eq!(written, all, "{name}: one sample per value, in order");
    for sample in samples {
        let wire = serde_json::to_value(&sample).unwrap();
        assert_eq!(serde_json::from_value::<T>(wire).unwrap(), sample, "{name}");
    }
    let raw = later_tagged(tag);
    let read: T = serde_json::from_value(raw.clone()).unwrap();
    let kept = UnknownBody {
        kind: LATER.to_owned(),
        raw: raw.clone(),
    };
    assert_eq!(read, unknown(kept), "{name}");
    assert_eq!(serde_json::to_value(&read).unwrap(), raw, "{name}");
    assert!(
        serde_json::from_value::<T>(json!({ "no_tag_here": 1 })).is_err(),
        "{name}"
    );
    assert!(
        serde_json::from_value::<T>(json!({ tag: 7 })).is_err(),
        "{name}"
    );
}

#[test]
fn every_tagged_list_keeps_its_wire_form_and_reads_what_it_does_not_know() {
    let branch = || "b".to_owned();
    let commit = || "c".to_owned();
    let at = nexus_run_contracts::Timestamp::parse("2026-11-05T10:00:00.000Z").unwrap();
    assert_a_tagged_list(
        "ParkOutcome",
        "kind",
        vec![
            ParkOutcome::Committed {
                branch: branch(),
                commit: commit(),
            },
            ParkOutcome::OnOwnBranch {
                branch: branch(),
                commit: commit(),
            },
            ParkOutcome::Clean { branch: None },
            ParkOutcome::Missing,
            ParkOutcome::NotHeld,
        ],
        ParkOutcome::ALL,
        ParkOutcome::Unknown,
    );
    assert_a_tagged_list(
        "TreeRefusal",
        "code",
        vec![
            TreeRefusal::NotARepository,
            TreeRefusal::ForeignDirectory,
            TreeRefusal::MidSequence {
                what: SequenceKind::Merge,
            },
            TreeRefusal::Git {
                reason: GitReason::LockHeld,
            },
        ],
        TreeRefusal::ALL,
        TreeRefusal::Unknown,
    );
    assert_a_tagged_list(
        "EndCause",
        "code",
        vec![
            EndCause::TurnComplete,
            EndCause::Exit { status: Some(1) },
            EndCause::UsageLimit { resets_at: None },
            EndCause::ProviderUnavailable,
            EndCause::PermissionRefused {
                rule: "r".into(),
                program: "p".into(),
            },
            EndCause::StoppedByHost,
            EndCause::Lost,
        ],
        EndCause::ALL,
        EndCause::Unknown,
    );
    assert_a_tagged_list(
        "RunState",
        "name",
        vec![
            RunState::Created,
            RunState::Queued { behind: None },
            RunState::Running,
            RunState::NeedsDecision,
            RunState::Held { until: at },
            RunState::Parked,
            RunState::Finishing,
            RunState::Finished,
        ],
        RunState::ALL,
        RunState::Unknown,
    );
}

/// In-process return values of one binary, never persisted, so nothing written can hold a later value.
const EXEMPT_IN_PROCESS: &[&str] = &[
    "Capability",
    "TimerChangeKind",
    "Enqueued",
    "StartOutcome",
    "Liveness",
];

/// A closed `pub enum` that reads from the wire is a list of the two walks above or one of the exempt:
/// a new one fails here until somebody has decided which. The macros declare the tolerant ones, so what
/// this finds spelled out is a derive that would refuse a later value.
#[test]
fn every_closed_enum_on_the_wire_is_exempted_by_name() {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut closed = Vec::new();
    for entry in std::fs::read_dir(src).unwrap() {
        let text = std::fs::read_to_string(entry.unwrap().path()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let Some(rest) = line.trim().strip_prefix("pub enum ") else {
                continue;
            };
            let name = rest.split_whitespace().next().unwrap();
            if name.starts_with('$') {
                continue; // The macros' own declaration: what they declare is tolerant.
            }
            let derives_a_reader = lines[..index]
                .iter()
                .rev()
                .take_while(|above| {
                    above.trim_start().starts_with('#') || above.trim_start().starts_with("///")
                })
                .any(|above| above.contains("Deserialize"));
            if derives_a_reader {
                closed.push(name.to_owned());
            }
        }
    }
    closed.sort();
    let mut exempt = owned(EXEMPT_IN_PROCESS);
    exempt.sort();
    assert_eq!(closed, exempt);
}

#[test]
fn the_golden_pins_every_list_a_writer_may_use() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/enum-values.json");
    let golden: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let lists: [(&str, &[&str]); 14] = [
        ("step_states", STEP_STATES),
        ("member_states", MEMBER_STATES),
        ("disposition", Disposition::ALL),
        ("end_cause", EndCause::ALL),
        ("heartbeat_kind", HeartbeatKind::ALL),
        ("needs_kind", NeedsKind::ALL),
        ("origin", Origin::ALL),
        ("park_outcome", ParkOutcome::ALL),
        ("run_state", RunState::ALL),
        ("sequence_kind", SequenceKind::ALL),
        ("stop_kind", StopKind::ALL),
        ("tree_refusal", TreeRefusal::ALL),
        ("verdict", Verdict::ALL),
        ("withdrawn_by", WithdrawnBy::ALL),
    ];
    assert_eq!(golden.as_object().unwrap().len(), lists.len());
    for (name, list) in lists {
        assert_eq!(golden[name], json!(list), "{name}");
    }
}

/// (the decision, the field holding an enum only decisions carry).
const DECISION_POSITIONS: &[(&str, &str)] = &[
    ("step_finished", "/body/outcome"),
    ("finishing", "/body/outcome"),
    ("finished", "/body/outcome"),
    ("timer_set", "/body/purpose"),
];

#[test]
fn a_known_decision_holding_an_unknown_value_counts_as_an_unknown_decision() {
    for (kind, pointer) in DECISION_POSITIONS {
        let raw = golden_with(kind, pointer, &later_code());
        let (event, back) = round_trip(&raw);
        assert_eq!(back, raw);
        assert!(event.is_unknown_decision(), "{kind}");
        assert!(!event.is_unknown_input(), "{kind}");
        assert_eq!(event.body.kind(), *kind);
    }
}

/// Whoever appended it: the host appends `parked { poison_input }` itself, so the rule reads the body.
#[test]
fn a_decision_holding_an_unknown_value_counts_as_one_whatever_its_origin_says() {
    let mut raw = golden_with("finished", "/body/outcome", &later_code());
    raw["origin"] = json!("host");
    assert!(round_trip(&raw).0.is_unknown_decision());
}

/// An unknown value in an INPUT folds by its enum's rule, so it never refuses ownership: that is what
/// keeps a phone's newer `disposition` from taking a run away from its host.
#[test]
fn an_input_holding_an_unknown_value_is_no_unknown_decision() {
    for (kind, pointer, value) in log_positions() {
        if DECISION_POSITIONS.contains(&(kind, pointer)) || pointer == "/origin" {
            continue;
        }
        let (event, _) = round_trip(&golden_with(kind, pointer, &value));
        assert!(!event.is_unknown_decision(), "{kind} {pointer}");
        assert!(!event.is_unknown_input(), "{kind} {pointer}");
    }
}

/// `ReasonCode` explains and folds the same whatever it says, and a withdrawal by an unknown party writes
/// its raw value there: it is the one enum a decision holds that stays open.
#[test]
fn a_decision_holding_an_unknown_reason_code_folds_as_usual() {
    for kind in ["held", "parked", "liveness", "ignored"] {
        let raw = golden_with(kind, "/body/reason", &later_code());
        assert!(!round_trip(&raw).0.is_unknown_decision(), "{kind}");
    }
}

/// An unknown origin is never `reducer`, so an unknown kind under one is an input and folds to nothing.
#[test]
fn an_unknown_kind_under_an_unknown_origin_reads_as_an_input() {
    let raw = envelope(LATER, json!({ "kind": "a_later_kind" }));
    let (event, back) = round_trip(&raw);
    assert_eq!(back, raw);
    assert_eq!(event.origin, Origin::Unknown(LATER.to_owned()));
    assert!(event.is_unknown_input());
    assert!(!event.is_unknown_decision());
}
