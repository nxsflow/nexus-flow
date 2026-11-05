use super::*;

fn decl(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("the fixture parses")
}

fn real() -> (ChannelDecl, ChannelDecl) {
    (
        decl(include_str!("../../tests/fixtures/coding.yaml")),
        decl(include_str!("../../tests/fixtures/review.yaml")),
    )
}

fn step<'a>(def: &'a FlowDef, id: &str) -> &'a StepDef {
    def.steps.iter().find(|s| s.id == id).unwrap()
}

/// `coding` with one line of its text replaced, so a refusal is tested on the real shape.
fn coding_with(from: &str, to: &str) -> ChannelDecl {
    let text = include_str!("../../tests/fixtures/coding.yaml");
    assert!(text.contains(from), "the fixture no longer says {from:?}");
    decl(&text.replacen(from, to, 1))
}

fn review_with(from: &str, to: &str) -> ChannelDecl {
    let text = include_str!("../../tests/fixtures/review.yaml");
    assert!(text.contains(from), "the fixture no longer says {from:?}");
    decl(&text.replacen(from, to, 1))
}

#[test]
fn the_real_coding_channel_compiles_to_the_four_steps_of_the_specification() {
    let (coding, review) = real();
    let def = compile(&coding, &review).unwrap();

    assert_eq!(def.name, "coding");
    assert_eq!(def.working_tree, WorkingTree::Exclusive);
    let ids: Vec<&str> = def.steps.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["build", "verify", "judge", "finish"]);

    let build = step(&def, "build");
    assert_eq!(
        build.kind,
        StepKind::Single {
            target: "coder".into()
        }
    );
    assert!(build.resume);
    assert_eq!(build.next.as_deref(), Some("verify"));
    assert_eq!(build.on_needs_rework, None);
    assert_eq!(build.max_passes, 1);
    assert_eq!(build.input, None);

    let verify = step(&def, "verify");
    assert_eq!(
        verify.kind,
        StepKind::Single {
            target: "verifier".into()
        }
    );
    assert_eq!(verify.input, Some(vec![]));
    assert_eq!(verify.max_passes, 3);
    assert_eq!(verify.on_needs_rework.as_deref(), Some("build"));
    assert!(!verify.resume);

    let judge = step(&def, "judge");
    let StepKind::FanOut {
        members,
        expects,
        synthesis,
    } = &judge.kind
    else {
        panic!("judge is a fan-out: {:?}", judge.kind);
    };
    assert_eq!(members, &["code-quality", "test-quality", "integrity"]);
    assert_eq!(*expects, FanOutExpects::All);
    let synthesis = synthesis.as_ref().expect("review declares summarize");
    assert_eq!(synthesis.model.as_deref(), Some("opus"));
    assert!(synthesis.prompt.starts_with("You are the consolidator"));
    assert_eq!(judge.max_passes, 3);
    assert_eq!(judge.on_needs_rework.as_deref(), Some("build"));
    assert!(judge
        .task
        .as_deref()
        .unwrap()
        .contains("must-fix (design review)"));
    assert_eq!(judge.next.as_deref(), Some("finish"));

    let finish = step(&def, "finish");
    assert_eq!(
        finish.kind,
        StepKind::Single {
            target: "finisher".into()
        }
    );
    assert_eq!(
        finish.input,
        Some(vec!["verify".to_owned(), "judge".to_owned()])
    );
    assert_eq!(finish.next, None);
    assert_eq!(finish.on_needs_rework.as_deref(), Some("build"));
    assert_eq!(finish.max_passes, 3);
}

#[test]
fn a_persona_step_opens_in_the_flow_channel_and_a_fan_out_in_its_own_both_as_the_supervisor() {
    let (coding, review) = real();
    let def = compile(&coding, &review).unwrap();
    assert_eq!(step(&def, "build").channel, "coding");
    assert_eq!(step(&def, "judge").channel, "review");
    for s in &def.steps {
        assert_eq!(s.opener, SUPERVISOR);
    }
}

#[test]
fn a_timeout_is_reported_as_ignored_and_never_dropped() {
    let (coding, review) = real();
    let def = compile(&coding, &review).unwrap();
    assert_eq!(
        def.ignored_timeouts,
        vec![
            IgnoredTimeout {
                scope: "coding".into(),
                value: "6h".into()
            },
            IgnoredTimeout {
                scope: "review".into(),
                value: "1h".into()
            },
        ]
    );

    let text = include_str!("../../tests/fixtures/coding.yaml").replacen(
        "  - id: verify\n",
        "  - id: verify\n    timeout: 20m\n",
        1,
    );
    let def = compile(&decl(&text), &review).unwrap();
    assert!(def.ignored_timeouts.contains(&IgnoredTimeout {
        scope: "verify".into(),
        value: "20m".into()
    }));
}

#[test]
fn a_shared_flow_is_refused() {
    let (_, review) = real();
    let coding = coding_with("working_tree: exclusive", "working_tree: shared");
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::SharedTree {
            channel: "coding".into()
        })
    );
}

#[test]
fn a_flow_that_declares_no_working_tree_is_shared_as_the_old_default_and_refused() {
    let (_, review) = real();
    let coding = coding_with("working_tree: exclusive\n", "");
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::SharedTree {
            channel: "coding".into()
        })
    );
}

#[test]
fn expects_subset_is_refused() {
    let (coding, _) = real();
    let review = review_with("expects: all", "expects: [code-quality, integrity]");
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::ExpectsSubset {
            channel: "review".into()
        })
    );
}

#[test]
fn pass_through_with_a_fan_out_is_refused() {
    let (coding, _) = real();
    let review = review_with("on_complete: summarize", "on_complete: pass_through");
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::PassThroughFanOut {
            channel: "review".into()
        })
    );
}

#[test]
fn a_fan_out_that_declares_no_on_complete_passes_through_as_the_old_default_and_is_refused() {
    let (coding, _) = real();
    let review = review_with("on_complete: summarize\n", "");
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::PassThroughFanOut {
            channel: "review".into()
        })
    );
}

#[test]
fn summarize_with_no_prompt_is_refused() {
    let (coding, _) = real();
    let mut review = decl(include_str!("../../tests/fixtures/review.yaml"));
    review.summary_prompt = None;
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::SummaryWithoutPrompt {
            channel: "review".into()
        })
    );
}

#[test]
fn ids_outside_the_alphabet_are_refused_because_they_become_paths_and_keys() {
    let (_, review) = real();
    for bad in ["bu/ild", "bu ild", "bü", "..", ""] {
        let coding = coding_with("  - id: build\n", &format!("  - id: \"{bad}\"\n"));
        assert_eq!(
            compile(&coding, &review),
            Err(CompileRefusal::BadId { id: bad.into() }),
            "{bad:?}"
        );
    }
    let coding = decl(include_str!("../../tests/fixtures/coding.yaml"));
    let review_bad = review_with("code-quality", "code/quality");
    assert_eq!(
        compile(&coding, &review_bad),
        Err(CompileRefusal::BadId {
            id: "code/quality".into()
        })
    );
    let persona_bad = coding_with("target: coder", "target: \"co/der\"");
    assert_eq!(
        compile(&persona_bad, &review),
        Err(CompileRefusal::BadId {
            id: "co/der".into()
        })
    );
}

#[test]
fn a_reserved_identity_cannot_be_declared_as_a_step_target() {
    let (_, review) = real();
    let coding = coding_with("target: coder", "target: __synth__");
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::BadId {
            id: "__synth__".into()
        })
    );
}

#[test]
fn a_reference_to_a_step_that_does_not_exist_is_refused() {
    let (_, review) = real();
    for (from, to, name) in [
        ("next: verify", "next: nowhere", "nowhere"),
        (
            "on_needs_rework: build",
            "on_needs_rework: nowhere",
            "nowhere",
        ),
        (
            "input: [verify, judge]",
            "input: [verify, nowhere]",
            "nowhere",
        ),
    ] {
        let coding = coding_with(from, to);
        assert_eq!(
            compile(&coding, &review),
            Err(CompileRefusal::UnknownStep { name: name.into() }),
            "{from}"
        );
    }
}

#[test]
fn a_channel_with_no_steps_is_refused() {
    let (_, review) = real();
    let coding = decl("name: bare\nmembers: []\nworking_tree: exclusive\n");
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::NoSteps {
            channel: "bare".into()
        })
    );
}

#[test]
fn a_fan_out_to_a_channel_with_no_members_is_refused_because_no_slot_would_ever_answer() {
    let (coding, _) = real();
    let members = "members: [code-quality, test-quality, integrity]\n";
    for review in [
        review_with(members, "members: []\n"),
        // A misspelled key parses as no members at all.
        review_with(members, "member: [code-quality, test-quality, integrity]\n"),
    ] {
        assert_eq!(
            compile(&coding, &review),
            Err(CompileRefusal::NoMembers {
                channel: "review".into()
            })
        );
    }
}

#[test]
fn two_steps_under_one_id_are_refused_because_edges_would_address_the_wrong_one() {
    let (mut coding, review) = real();
    let again = coding.steps[1].clone();
    coding.steps.push(again);
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::DuplicateStep {
            id: "verify".into()
        })
    );
}

#[test]
fn a_member_named_twice_is_refused_because_its_two_slots_would_share_one_key() {
    let (coding, _) = real();
    let review = review_with(
        "members: [code-quality, test-quality, integrity]",
        "members: [code-quality, test-quality, code-quality]",
    );
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::DuplicateMember {
            channel: "review".into(),
            handle: "code-quality".into()
        })
    );
}

#[test]
fn one_persona_in_two_steps_is_not_a_duplicate() {
    let (_, review) = real();
    let coding = coding_with("target: verifier", "target: coder");
    let def = compile(&coding, &review).unwrap();
    assert_eq!(step(&def, "build").members(), step(&def, "verify").members());
}

#[test]
fn a_ceiling_of_zero_is_refused_because_its_back_edge_could_never_be_taken() {
    let (_, review) = real();
    let coding = coding_with("max_passes: 3", "max_passes: 0");
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::ZeroMaxPasses {
            step: "verify".into()
        })
    );
}

#[test]
fn a_next_chain_that_closes_on_itself_is_refused_because_no_ceiling_bounds_it() {
    let (mut coding, review) = real();
    coding.steps[3].next = Some("build".into());
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::NextCycle {
            step: "build".into()
        })
    );
}

#[test]
fn a_next_cycle_the_first_step_never_reaches_is_refused_too() {
    let (mut coding, review) = real();
    coding.steps.push(StepDecl {
        id: "stray".into(),
        target: "coder".into(),
        next: Some("stray".into()),
        ..StepDecl::default()
    });
    assert_eq!(
        compile(&coding, &review),
        Err(CompileRefusal::NextCycle {
            step: "stray".into()
        })
    );
}

#[test]
fn max_passes_defaults_to_one_on_a_declared_back_edge() {
    let (_, review) = real();
    let text = include_str!("../../tests/fixtures/coding.yaml").replacen(
        "    on_needs_rework: build\n    max_passes: 3\n    # NOTHING",
        "    on_needs_rework: build\n    # NOTHING",
        1,
    );
    assert_ne!(text, include_str!("../../tests/fixtures/coding.yaml"));
    let def = compile(&decl(&text), &review).unwrap();
    assert_eq!(step(&def, "verify").max_passes, 1);
}

#[test]
fn the_flow_reference_is_the_hash_of_the_canonical_json_and_survives_a_store_round_trip() {
    let (coding, review) = real();
    let def = compile(&coding, &review).unwrap();
    let bytes = def.canonical_json();
    let back: FlowDef = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(back, def);
    assert_eq!(back.flow_ref(), FlowRef::of_canonical_json(&bytes));
    assert_eq!(
        def.flow_ref(),
        compile(&coding, &review).unwrap().flow_ref()
    );

    let edited = coding_with("max_passes: 3", "max_passes: 4");
    assert_ne!(
        compile(&edited, &review).unwrap().flow_ref(),
        def.flow_ref()
    );
}
