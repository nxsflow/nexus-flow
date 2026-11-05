//! Additive evolution: a reader of this version keeps what it does not know, and tells an unknown
//! input (which folds to nothing) from an unknown decision (which it must refuse to fold past).
//! Adding a decision kind changes the variant list AND the contract version.

use nexus_run_contracts::{
    Decision, Event, EventBody, GitReason, ReasonCode, SessionFact, CONTRACT_V,
};
use serde::Deserialize;
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

/// The decision-kind history: which list each contract version shipped.
#[derive(Deserialize)]
struct History {
    versions: Vec<Pin>,
}

#[derive(Clone, Deserialize)]
struct Pin {
    v: u32,
    decision_kinds: Vec<String>,
}

fn history() -> Vec<Pin> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/golden/decision-kinds.json"
    );
    let history: History = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    history.versions
}

/// The rule: the code's list is the list pinned for the code's version, and a version's list is
/// different from the one before it, so a new kind has to come with a new version.
fn check_pins(kinds: &[String], v: u32, history: &[Pin]) -> Result<(), String> {
    let pin = history
        .iter()
        .find(|pin| pin.v == v)
        .ok_or_else(|| format!("CONTRACT_V {v} has no entry in decision-kinds.json"))?;
    if pin.decision_kinds != kinds {
        return Err(format!(
            "the decision kinds differ from the list pinned for v{v}: raise CONTRACT_V and add a \
             version to decision-kinds.json"
        ));
    }
    for pair in history.windows(2) {
        if pair[1].v != pair[0].v + 1 || pair[1].decision_kinds == pair[0].decision_kinds {
            return Err(format!(
                "v{} does not change the list of v{}",
                pair[1].v, pair[0].v
            ));
        }
    }
    Ok(())
}

fn current_kinds() -> Vec<String> {
    Decision::KINDS.iter().map(|k| (*k).to_owned()).collect()
}

#[test]
fn the_decision_kinds_are_the_list_pinned_for_contract_v() {
    check_pins(&current_kinds(), CONTRACT_V, &history()).unwrap();
}

#[test]
fn a_new_decision_kind_under_the_same_version_is_refused() {
    let mut kinds = current_kinds();
    kinds.push("step_skipped".into());
    let refusal = check_pins(&kinds, CONTRACT_V, &history()).unwrap_err();
    assert!(refusal.contains("raise CONTRACT_V"), "{refusal}");
}

#[test]
fn a_new_decision_kind_with_a_raised_version_and_no_pin_is_refused() {
    let mut kinds = current_kinds();
    kinds.push("step_skipped".into());
    let refusal = check_pins(&kinds, CONTRACT_V + 1, &history()).unwrap_err();
    assert!(refusal.contains("no entry"), "{refusal}");
}

#[test]
fn a_raised_version_that_keeps_the_list_is_refused() {
    let mut pins = history();
    pins.push(Pin {
        v: CONTRACT_V + 1,
        decision_kinds: current_kinds(),
    });
    assert!(check_pins(&current_kinds(), CONTRACT_V + 1, &pins).is_err());
}

#[test]
fn a_new_decision_kind_with_a_raised_version_and_its_pin_is_accepted() {
    let mut kinds = current_kinds();
    kinds.push("step_skipped".into());
    let mut pins = history();
    pins.push(Pin {
        v: CONTRACT_V + 1,
        decision_kinds: kinds.clone(),
    });
    check_pins(&kinds, CONTRACT_V + 1, &pins).unwrap();
}
