//! Tests for `channel::validate_channels` (nxf ticket 6j6v.t146): pure referential validation of
//! declared channels against declared roles. No I/O — every test builds `RoleDecl`/`ChannelDecl`
//! values directly and asserts on the returned `Vec<ValidationError>`.

use nexus_chat::channel::validate_channels;
use nexus_chat::channel::{ChannelDecl, ChannelKind, Expects, Flow, OnComplete, Visibility};
use nexus_chat::role::{BasePrompt, ClaudeMd, Model, RoleDecl, SessionPolicy};

fn role(handle: &str) -> RoleDecl {
    RoleDecl {
        handle: handle.to_string(),
        job_title: None,
        job_description: None,
        reports_to: None,
        system_prompt: "be helpful".to_string(),
        base_prompt: BasePrompt::ClaudeCode,
        claude_md: ClaudeMd::Inherit,
        tools: None,
        permissions: None,
        model: None,
        machine: None,
        session: SessionPolicy::Fresh,
        sub_agents: false,
        expected_output: None,
        prime: nexus_chat::role::PrimeDecl::All(true),
        stage: None,
        addressable: Default::default(),
        address_book: None,
        working_tree: Default::default(),
    }
}

fn channel(name: &str, members: &[&str]) -> ChannelDecl {
    ChannelDecl {
        name: name.to_string(),
        members: members.iter().map(|s| s.to_string()).collect(),
        kind: ChannelKind::Group,
        description: None,
        expects: Expects::All,
        timeout: None,
        on_complete: OnComplete::PassThrough,
        summary_prompt: None,
        summary_model: None,
        visibility: Visibility::RequesterOnly,
        working_tree: Default::default(),
        flow: Flow::Parallel,
        preconditions: Vec::new(),
        steps: Vec::new(),
        rework_notice: None,
    }
}

#[test]
fn clean_set_yields_no_errors() {
    let roles = vec![role("pm"), role("coder"), role("reviewer")];
    let channels = vec![
        channel("standup", &["pm", "coder"]),
        channel("retro", &["pm", "coder", "reviewer"]),
    ];
    assert_eq!(validate_channels(&roles, &channels), vec![]);
}

#[test]
fn unknown_member_handle_names_channel_and_handle() {
    let roles = vec![role("pm")];
    let channels = vec![channel("standup", &["pm", "ghost"])];

    let errors = validate_channels(&roles, &channels);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].file, "channels.yaml");
    assert!(
        errors[0].what.contains("standup"),
        "what: {}",
        errors[0].what
    );
    assert!(errors[0].what.contains("ghost"), "what: {}", errors[0].what);
}

#[test]
fn unknown_expects_handle_is_distinguishable_from_member_error() {
    let roles = vec![role("pm"), role("coder")];
    let mut ch = channel("standup", &["pm", "coder"]);
    ch.expects = Expects::Subset(vec!["pm".to_string(), "ghost".to_string()]);
    let channels = vec![ch];

    let errors = validate_channels(&roles, &channels);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].file, "channels.yaml");
    assert!(errors[0].what.contains("standup"));
    assert!(errors[0].what.contains("ghost"));

    // Build the equivalent "unknown member" error for the SAME handle to confirm the wording
    // differs (a human/agent must be able to tell "member" and "expects" problems apart).
    let member_channels = vec![channel("standup", &["pm", "ghost"])];
    let member_errors = validate_channels(&roles, &member_channels);
    assert_eq!(member_errors.len(), 1);
    assert_ne!(errors[0].what, member_errors[0].what);
}

#[test]
fn summarize_without_summary_prompt_errors_naming_channel() {
    let roles = vec![role("pm")];
    let mut ch = channel("standup", &["pm"]);
    ch.on_complete = OnComplete::Summarize;
    ch.summary_prompt = None;
    let channels = vec![ch];

    let errors = validate_channels(&roles, &channels);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].file, "channels.yaml");
    assert!(errors[0].what.contains("standup"));
}

#[test]
fn summarize_with_summary_prompt_is_not_a_false_positive() {
    let roles = vec![role("pm")];
    let mut ch = channel("standup", &["pm"]);
    ch.on_complete = OnComplete::Summarize;
    ch.summary_prompt = Some("Summarize the replies.".to_string());
    let channels = vec![ch];

    assert_eq!(validate_channels(&roles, &channels), vec![]);
}

#[test]
fn pass_through_default_with_no_summary_prompt_is_fine() {
    let roles = vec![role("pm")];
    let ch = channel("standup", &["pm"]); // on_complete defaults to PassThrough, no prompt
    let channels = vec![ch];

    assert_eq!(validate_channels(&roles, &channels), vec![]);
}

#[test]
fn two_channels_sharing_a_name_yield_exactly_one_duplicate_error() {
    let roles = vec![role("pm")];
    let channels = vec![channel("standup", &["pm"]), channel("standup", &["pm"])];

    let errors = validate_channels(&roles, &channels);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].file, "channels.yaml");
    assert!(errors[0].what.contains("standup"));
}

#[test]
fn three_channels_sharing_a_name_still_yield_exactly_one_error() {
    let roles = vec![role("pm")];
    let channels = vec![
        channel("standup", &["pm"]),
        channel("standup", &["pm"]),
        channel("standup", &["pm"]),
    ];

    let errors = validate_channels(&roles, &channels);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].what.contains("standup"));
}

#[test]
fn multi_error_channel_orders_by_check_then_handle() {
    // Two unknown members (in given order) AND a missing summary_prompt on the same channel.
    let roles = vec![role("pm")];
    let mut ch = channel("standup", &["ghost1", "ghost2"]);
    ch.on_complete = OnComplete::Summarize;
    ch.summary_prompt = None;
    let channels = vec![ch];

    let errors = validate_channels(&roles, &channels);
    assert_eq!(errors.len(), 3);
    // Check 1 (members), in handle order, then check 3 (summary_prompt).
    assert!(
        errors[0].what.contains("ghost1"),
        "errors[0]: {}",
        errors[0].what
    );
    assert!(
        errors[1].what.contains("ghost2"),
        "errors[1]: {}",
        errors[1].what
    );
    assert!(
        errors[2].what.contains("summary_prompt") || errors[2].what.contains("summarize"),
        "errors[2]: {}",
        errors[2].what
    );
    for e in &errors {
        assert_eq!(e.file, "channels.yaml");
    }
}

#[test]
fn expects_all_with_all_members_resolving_yields_no_expects_error() {
    let roles = vec![role("pm"), role("coder")];
    let ch = channel("standup", &["pm", "coder"]); // expects defaults to All
    let channels = vec![ch];

    assert_eq!(validate_channels(&roles, &channels), vec![]);
}

#[test]
fn full_deterministic_order_across_multiple_channels_then_dup_names() {
    // channel "a": unknown member "x" (check 1)
    // channel "b": unknown expects handle "y" (check 2)
    // channel "a" again (dup of "a", but distinct content doesn't matter for dup detection)
    let roles = vec![role("pm")];
    let mut ch_b = channel("b", &["pm"]);
    ch_b.expects = Expects::Subset(vec!["y".to_string()]);
    let channels = vec![
        channel("a", &["x"]),
        ch_b,
        channel("a", &["pm"]), // duplicate name "a"
    ];

    let errors = validate_channels(&roles, &channels);
    // Per-channel errors in declaration order first: channel "a" (index 0) member error, then
    // channel "b" (index 1) expects error, then channel "a" (index 2) has no per-channel errors
    // itself (member "pm" resolves). Then the duplicate-name pass: "a" is the only dup.
    assert_eq!(errors.len(), 3);
    assert!(errors[0].what.contains('a') && errors[0].what.contains('x'));
    assert!(errors[1].what.contains('b') && errors[1].what.contains('y'));
    assert_eq!(errors[2].file, "channels.yaml");
    assert!(
        errors[2].what.contains('a'),
        "dup error should name 'a': {}",
        errors[2].what
    );
}

// ---- the declared consolidator's fold model (nxf 6j6v.e9qj) ---------------------------------

#[test]
fn a_fold_model_on_a_channel_that_never_folds_is_flagged() {
    // Acceptance point 4, the consistent extension of check 3 (`summarize` ⇒ `summary_prompt`): a
    // declared fold MODEL on a channel whose consolidator passes through names a thing that will
    // never be reached. Same class of author mistake, same validator, same shape of message.
    let roles = vec![role("bob")];
    let mut ch = channel("review", &["bob"]); // on_complete defaults to PassThrough
    ch.summary_model = Some(Model::Opus);

    let errors = validate_channels(&roles, std::slice::from_ref(&ch));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].file, "channels.yaml");
    assert!(
        errors[0].what.contains("summary_model") && errors[0].what.contains("summarize"),
        "the error must name the field and the policy it needs: {:?}",
        errors[0]
    );
}

#[test]
fn a_fold_model_on_a_channel_that_folds_is_accepted() {
    let roles = vec![role("bob")];
    let mut ch = channel("review", &["bob"]);
    ch.on_complete = OnComplete::Summarize;
    ch.summary_prompt = Some("fold them".to_string());
    ch.summary_model = Some(Model::Opus);
    assert_eq!(validate_channels(&roles, std::slice::from_ref(&ch)), vec![]);
}

#[test]
fn a_fold_with_no_declared_model_is_not_an_error_because_the_fallback_is_declared() {
    // The rule is deliberately NOT "summarize ⇒ summary_model": acceptance point 3 gives an
    // undeclared fold model a DEFINED, documented fallback, so requiring one would refuse a
    // declaration that is complete.
    let roles = vec![role("bob")];
    let mut ch = channel("review", &["bob"]);
    ch.on_complete = OnComplete::Summarize;
    ch.summary_prompt = Some("fold them".to_string());
    assert_eq!(validate_channels(&roles, std::slice::from_ref(&ch)), vec![]);
}

// ---- the declared FLOW (nxf 6j6v.hq71, DoD point 1) ------------------------------------------

#[test]
fn a_sequential_flow_that_names_the_same_target_twice_is_refused() {
    // The incoherent flow declaration this item owes a rule for. A repeated step is a LOOP, and
    // hq71 §5 retired the only signal that could ever have routed one (a free branching token an
    // agent uttered). It is also what the engine's own pass derivation reads to decide which step a
    // slot thread serves, so a repeat cannot be left to mean something by accident.
    let roles = vec![role("coder"), role("reviewer")];
    let mut ch = channel("coding", &["coder", "reviewer", "coder"]);
    ch.flow = Flow::Sequential;

    let errors = validate_channels(&roles, std::slice::from_ref(&ch));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].file, "channels.yaml");
    assert!(
        errors[0].what.contains("coding") && errors[0].what.contains("coder"),
        "the channel and the repeated step are both named: {errors:?}"
    );

    // …and the SAME list under the default flow is not this rule's business: a parallel fan-out
    // that names a member twice asks it twice, which is a different (and pre-existing) question.
    ch.flow = Flow::Parallel;
    assert_eq!(validate_channels(&roles, std::slice::from_ref(&ch)), vec![]);
}

#[test]
fn a_flow_step_may_name_a_declared_channel_and_an_unknown_name_is_still_refused() {
    // "Kanal = Arbeitsablauf. Es gibt keinen zweiten Begriff dafuer": a step addresses a channel
    // exactly as `nxc send --to <name>` does, so check 1 resolves a member against roles AND
    // declared channels. A name that is neither is still the same loud error it always was.
    let roles = vec![role("reviewer")];
    let review = channel("review", &["reviewer"]);
    let mut coding = channel("coding", &["review"]);
    coding.flow = Flow::Sequential;
    assert_eq!(
        validate_channels(&roles, &[coding, review.clone()]),
        vec![],
        "a channel is a legitimate flow step"
    );

    let unknown = channel("coding", &["nobody"]);
    let errors = validate_channels(&roles, &[unknown, review]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].what.contains("nobody"),
        "the offending name is still named: {errors:?}"
    );
}

#[test]
fn a_flow_that_can_reach_itself_is_refused_because_it_would_never_end() {
    // The cost of letting a step name a channel: a flow can now close a loop, and a loop opens a
    // fresh session at every hop until the depth guard stops it. Refused at the declaration
    // instead, where it costs nothing.
    let roles = vec![role("reviewer")];
    let a = channel("a", &["b"]);
    let b = channel("b", &["a", "reviewer"]);

    let errors = validate_channels(&roles, &[a, b]);
    assert_eq!(
        errors.len(),
        2,
        "one per channel that lies on the cycle: {errors:?}"
    );
    assert!(
        errors.iter().all(|e| e.what.contains("reaches itself")),
        "{errors:?}"
    );
}

#[test]
fn an_ordered_channel_may_not_declare_an_expects_subset_in_either_of_its_two_harmful_shapes() {
    // Review round 1, Important 1. `fan_out_targets` returns an `Expects::Subset` VERBATIM and never
    // consults `members`, so a subset on an ordered channel is a second list deciding the flow —
    // and check 6, which scans `members`, cannot see into it. Two shapes, both refused:
    let roles = vec![role("a"), role("b")];

    // (1) REORDERING: the flow would run b→a while `members` says a→b, making this item's own
    // "the order of `members` IS the flow" false in exactly the case that matters.
    let mut reordered = channel("coding", &["a", "b"]);
    reordered.flow = Flow::Sequential;
    reordered.expects = Expects::Subset(vec!["b".to_string(), "a".to_string()]);
    let errors = validate_channels(&roles, std::slice::from_ref(&reordered));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].what.contains("coding") && errors[0].what.contains("expects"),
        "{errors:?}"
    );

    // (2) REPEATING: `[a, b, a]` names a step twice through the door check 6 does not guard. That
    // is the precondition `current_pass`'s boundary derivation depends on, and without this rule the
    // flow ping-pongs between two steps, one thread and one paid session per hop.
    let mut repeating = channel("coding", &["a", "b"]);
    repeating.flow = Flow::Sequential;
    repeating.expects = Expects::Subset(vec!["a".to_string(), "b".to_string(), "a".to_string()]);
    let errors = validate_channels(&roles, std::slice::from_ref(&repeating));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].what.contains("expects"), "{errors:?}");

    // …and the SAME subsets on a parallel channel are untouched: `expects` narrowing a fan-out is
    // what it has always been, and nothing about it decides an order.
    let mut parallel = reordered.clone();
    parallel.flow = Flow::Parallel;
    assert_eq!(
        validate_channels(&roles, std::slice::from_ref(&parallel)),
        vec![]
    );
}

#[test]
fn the_ordered_subset_rule_fails_closed_at_the_point_of_use_not_only_in_prime() {
    // `validate_channels` is advisory; `validate_channel_for_use` is what `Definitions::declared_channel`
    // runs before any verb acts on a declaration. A declaration that cannot mean one thing must not
    // run, so the rule lives there and is surfaced here rather than the other way round.
    let mut ch = channel("coding", &["a", "b"]);
    ch.flow = Flow::Sequential;
    ch.expects = Expects::Subset(vec!["b".to_string()]);
    let err = nexus_chat::channel::validate_channel_for_use(&ch)
        .expect("an ordered channel with a subset cannot be run");
    assert_eq!(err.file, "channels.yaml");
    assert!(
        err.what.contains("flow=sequential") && err.what.contains("expects"),
        "{err:?}"
    );
}

// ---- declared hurdles (nxf 6j6v.n92p, check 8) -----------------------------------------------

fn hurdle(name: &str, run: &str) -> nexus_chat::precondition::Precondition {
    nexus_chat::precondition::Precondition {
        name: name.to_string(),
        run: run.to_string(),
        expect: None,
    }
}

#[test]
fn well_formed_hurdles_yield_no_errors() {
    let roles = vec![role("a"), role("b")];
    let mut ch = channel("coding", &["a", "b"]);
    ch.preconditions = vec![
        hurdle("remote-not-ahead", "git rev-list --count HEAD..@{u}"),
        hurdle("tree-clean", "git status --porcelain"),
    ];
    assert_eq!(validate_channels(&roles, std::slice::from_ref(&ch)), vec![]);
}

#[test]
fn two_hurdles_under_one_name_are_refused_because_a_refusal_names_one() {
    // The name is what a refusal is reported under and therefore the one string a requester
    // branches on — two hurdles sharing it make a machine-readable refusal ambiguous exactly where
    // it is supposed to be exact.
    let roles = vec![role("a")];
    let mut ch = channel("coding", &["a"]);
    ch.preconditions = vec![
        hurdle("tree-clean", "git status --porcelain"),
        hurdle("tree-clean", "git diff --quiet"),
    ];
    let errors = validate_channels(&roles, std::slice::from_ref(&ch));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].what.contains("declared twice"), "{errors:?}");
    assert!(errors[0].what.contains("tree-clean"), "{errors:?}");
}

#[test]
fn a_hurdle_with_no_name_and_a_hurdle_with_no_command_are_both_refused() {
    // An empty `run` is the one that matters most: an empty shell line exits zero, so it would
    // declare a hurdle that always passes — fail-open wearing the costume of a rule.
    let roles = vec![role("a")];
    let mut ch = channel("coding", &["a"]);
    ch.preconditions = vec![hurdle("", "git status --porcelain"), hurdle("empty", "  ")];
    let errors = validate_channels(&roles, std::slice::from_ref(&ch));
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].what.contains("no `name`"), "{errors:?}");
    assert!(errors[1].what.contains("no `run` command"), "{errors:?}");
}

// ---- `steps:` — the referential half (nxf 6j6v.553s (d)) --------------------------------------
//
// This file had no `steps:` coverage at all when the field shipped (independent review of PR #400,
// Test Quality #2). What that left unguarded is not the error text but the LIST the check walks:
// a stepped channel commissions its STEP TARGETS, and a regression that silently fell back to
// reading `members` would pass every test in this file.

/// A stepped channel — `build` (a role) then `check` (whatever the caller names).
fn stepped(name: &str, members: &[&str], build: &str, check: &str) -> ChannelDecl {
    let yaml = format!(
        "name: {name}\nmembers: [{}]\nsteps:\n  - id: build\n    target: {build}\n    next: check\n  \
         - id: check\n    target: {check}\n",
        members.join(", ")
    );
    serde_yaml::from_str(&yaml).expect("the stepped channel parses")
}

#[test]
fn an_unknown_step_target_is_refused_and_named_as_a_step_target() {
    let roles = vec![role("coder")];
    let ch = stepped("coding", &["coder"], "coder", "ghost");
    let errors = validate_channels(&roles, std::slice::from_ref(&ch));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].what.contains("unknown step target"), "{errors:?}");
    assert!(errors[0].what.contains("ghost"), "{errors:?}");
    // …and it says STEP TARGET rather than "member handle": on a stepped channel `members` is not
    // the list that was checked, and blaming it would send the author to edit the wrong line.
    assert!(!errors[0].what.contains("member handle"), "{errors:?}");
}

#[test]
fn a_stepped_channel_is_checked_on_its_steps_and_not_on_its_members() {
    // **The regression this file could not have caught.** Both halves are asserted, because either
    // one alone is satisfied by a check that reads the wrong list:
    //
    //  - a member nothing commissions does NOT have to resolve (a channel may list an observer),
    //  - a step target DOES, even when every member resolves.
    //
    // Read `members` instead of `steps` and the first assertion fails; read both and the first
    // fails too; read neither and the second fails.
    let roles = vec![role("coder"), role("review")];

    let mut wide = stepped("coding", &["coder", "review"], "coder", "review");
    wide.members.push("observer-nobody-declared".to_string());
    assert!(
        validate_channels(&roles, std::slice::from_ref(&wide)).is_empty(),
        "a member no step commissions is not a referential error on a stepped channel: {:?}",
        validate_channels(&roles, std::slice::from_ref(&wide))
    );

    let narrow = stepped("coding", &["coder", "review"], "coder", "ghost");
    let errors = validate_channels(&roles, std::slice::from_ref(&narrow));
    assert_eq!(
        errors.len(),
        1,
        "every member resolves, so only the step target can be the finding: {errors:?}"
    );
    assert!(errors[0].what.contains("ghost"), "{errors:?}");
}

#[test]
fn a_step_may_commission_a_whole_channel_exactly_as_a_member_entry_may() {
    // The other direction of the same lookup: a step target resolves through `flow_step_target`, so
    // a declared CHANNEL is as good a target as a role — which is what makes a review quorum a step.
    let roles = vec![role("coder"), role("checker")];
    let quorum = channel("review", &["checker"]);
    let coding = stepped("coding", &["coder", "review"], "coder", "review");
    assert!(
        validate_channels(&roles, &[coding, quorum]).is_empty(),
        "a step naming a declared channel resolves"
    );
}

/// **A step whose target is a whole CHANNEL may not ask to continue a session** (nxf 6j6v.y1t9).
///
/// A channel step has no ONE session behind it — its members each have their own — so `resume: true`
/// there names something that cannot happen. Reported here rather than refused at the point of use
/// ([`nexus_chat::channel::validate_channel_for_use`]) for check 1's own reason and with check 1's
/// own consequence: telling a role target from a channel one needs the ROLE catalogue, and the
/// declaration DEGRADES rather than routing somewhere nobody declared — the step opener finds no
/// session and starts a fresh one, which is what a channel step does anyway.
#[test]
fn a_step_whose_target_is_a_whole_channel_may_not_ask_to_continue_a_session() {
    let channels: Vec<ChannelDecl> = serde_yaml::from_str(
        "- name: coding\n  members: [coder]\n  steps:\n    \
         - id: build\n      target: coder\n      next: check\n    \
         - id: check\n      target: panel\n      resume: true\n\
         - name: panel\n  members: [checkera]\n  on_complete: summarize\n  \
         summary_prompt: Fold it.\n",
    )
    .expect("the channels parse");
    let roles = vec![role("coder"), role("checkera")];
    let errors = validate_channels(&roles, &channels);
    assert_eq!(errors.len(), 1, "exactly the one rule fires: {errors:?}");
    let what = &errors[0].what;
    assert!(
        what.contains("\"check\"") && what.contains("\"panel\"") && what.contains("resume: true"),
        "it names the step, its target and the key the author wrote: {what}"
    );

    // The SAME declaration with a ROLE target is clean — the rule is about what a target IS, not
    // about the key existing.
    let ok: Vec<ChannelDecl> = serde_yaml::from_str(
        "- name: coding\n  members: [coder]\n  steps:\n    \
         - id: build\n      target: coder\n      next: check\n    \
         - id: check\n      target: checkera\n      resume: true\n",
    )
    .expect("the channels parse");
    assert_eq!(validate_channels(&roles, &ok), vec![]);
}

/// `resume: false` on a channel target is simply TRUE, so nothing is refused: the rule is about an
/// author who believes a session is carried across when none can be.
#[test]
fn a_channel_target_may_still_say_it_does_not_continue_a_session() {
    let channels: Vec<ChannelDecl> = serde_yaml::from_str(
        "- name: coding\n  members: [coder]\n  steps:\n    \
         - id: check\n      target: panel\n      resume: false\n\
         - name: panel\n  members: [checkera]\n  on_complete: summarize\n  \
         summary_prompt: Fold it.\n",
    )
    .expect("the channels parse");
    assert_eq!(
        validate_channels(&[role("coder"), role("checkera")], &channels),
        vec![]
    );
}
