//! What the contracts promise in words, because no type can say it (p71m). Each sentence below is an
//! accepted external finding: rewording one is fine, losing what it says is what this catches, and the
//! finding it came from is named beside it.

/// The words of the doc comments of one source file, with their line breaks gone.
fn docs(file: &str) -> String {
    let path = format!("{}/src/{file}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let lines = text.lines().filter_map(|line| {
        let line = line.trim_start();
        line.strip_prefix("//!")
            .or_else(|| line.strip_prefix("///"))
    });
    lines
        .flat_map(str::split_whitespace)
        .collect::<Vec<_>>()
        .join(" ")
}

#[track_caller]
fn assert_states(file: &str, finding: &str, sentences: &[&str]) {
    let docs = docs(file);
    for sentence in sentences {
        assert!(docs.contains(sentence), "{file} lost {finding}: {sentence}");
    }
}

#[test]
fn h4_a_later_tenant_is_never_a_stream_or_person_id_in_clear() {
    assert_states(
        "events.rs",
        "H4",
        &["A later real value is never a stream id or a person id in clear (PV1)."],
    );
}

#[test]
fn c1_a_local_run_stays_on_its_machine_and_no_call_reaches_another_tenant() {
    assert_states(
        "events.rs",
        "C1",
        &[
            "A run with tenant `local` does not leave its machine",
            "component 9 maps it at the boundary or refuses to sync it",
        ],
    );
    assert_states(
        "store.rs",
        "C1",
        &["No call on a store instance reaches another tenant's rows, a reconciler's `due_timers` and `open_runs` included."],
    );
}

#[test]
fn h5_the_owner_tombstone_never_identifies_a_person() {
    assert_states(
        "store.rs",
        "H5",
        &[
            "`owner/<run>` survives a purge holding only `owner_id`, `epoch` and `expires_at`",
            "`owner_id` is an opaque host id and never identifies a person",
        ],
    );
}

#[test]
fn m2_the_texts_for_people_are_never_parsed() {
    assert_states(
        "status.rs",
        "M2",
        &["`waiting_for` and `next_action` are for people and are never parsed."],
    );
}

#[test]
fn m3_a_human_gate_answer_is_a_new_input_kind() {
    assert_states(
        "events.rs",
        "M3",
        &["A human-gate answer arrives as a NEW input kind, never as fields on this one"],
    );
}

#[test]
fn m7_the_allowlist_is_the_baseline() {
    assert_states("session.rs", "M7", &["This list is the baseline"]);
}

#[test]
fn l1_the_sync_trait_is_deliberate() {
    assert_states("store.rs", "L1", &["The trait is synchronous on purpose"]);
}

#[test]
fn sy5_evidence_is_opt_in_and_never_in_the_portable_status() {
    assert_states(
        "status.rs",
        "SY5",
        &["Commit, branch, failure class, grade and log excerpts are evidence: opt-in per stream and never in the portable status."],
    );
}

#[test]
fn dqem_19wp_the_log_is_not_synced_as_it_stands() {
    assert_states(
        "events.rs",
        "dqem.19wp",
        &["It is not synced as it stands: sync carries an allowlist, so an unknown field never travels."],
    );
}

#[test]
fn r4_verdict_and_last_fact_make_no_claim_about_now() {
    assert_states(
        "status.rs",
        "R4",
        &["`verdict` and `last_fact` are as of `seq`, no claim about now"],
    );
}

#[test]
fn h1_a_status_reader_never_refuses_on_a_higher_v() {
    assert_states(
        "status.rs",
        "H1",
        &["A status reader never refuses on a higher `v`"],
    );
}

#[test]
fn the_objection_a_resume_is_an_explicit_action_never_a_root_reply() {
    let docs = docs("events.rs");
    for (kind, sentence) in [
        (
            "resume",
            "`Resume` comes from an explicit action, never a root reply",
        ),
        (
            "resume_run",
            "`ResumeRun` comes from an explicit action, never a root reply",
        ),
    ] {
        assert!(docs.contains(sentence), "the doc of {kind}: {sentence}");
    }
    assert!(
        !docs.contains("A root reply while"),
        "a doc still says a root reply resumes"
    );
}
