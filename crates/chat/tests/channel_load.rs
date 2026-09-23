use nexus_chat::channel::*;

const SINGLE_CHANNEL_YAML: &str = "- name: standup
  members: [pm, coder, reviewer]
  expects: [pm, coder]
  timeout: 20m
  on_complete: summarize
  summary_prompt: Summarize what each member reported.
  visibility: all_members
  kind: public
";

const MULTI_CHANNEL_YAML: &str = "- name: standup
  members: [pm, coder]
- name: retro
  members: [pm, coder, reviewer]
- name: incident
  members: [pm, reviewer]
";

#[test]
fn load_all_channels_over_a_missing_directory_is_empty_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("does-not-exist");
    assert_eq!(load_all_channels(&missing).unwrap(), vec![]);
}

#[test]
fn load_all_channels_over_a_missing_file_is_empty_not_an_error() {
    // The directory exists, but no channels.yaml inside it.
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(load_all_channels(tmp.path()).unwrap(), vec![]);
}

#[test]
fn load_all_channels_parses_a_single_well_formed_channel() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("channels.yaml"), SINGLE_CHANNEL_YAML).unwrap();

    let channels = load_all_channels(tmp.path()).unwrap();
    assert_eq!(
        channels,
        vec![ChannelDecl {
            name: "standup".to_string(),
            members: vec![
                "pm".to_string(),
                "coder".to_string(),
                "reviewer".to_string()
            ],
            kind: ChannelKind::Public,
            description: None,
            expects: Expects::Subset(vec!["pm".to_string(), "coder".to_string()]),
            timeout: Some("20m".to_string()),
            on_complete: OnComplete::Summarize,
            summary_prompt: Some("Summarize what each member reported.".to_string()),
            summary_model: None,
            visibility: Visibility::AllMembers,
            working_tree: Default::default(),
            flow: Flow::Parallel,
            preconditions: Vec::new(),
            steps: Vec::new(),
            rework_notice: None,
        }]
    );
}

/// A `channels.yaml` written before nxf 6j6v.fepb retired `member_session:` must still LOAD, and
/// the retired key must be ignored rather than rejected — that property is the whole reason the
/// removal was cheap, and it holds only for as long as [`ChannelDecl`] carries no
/// `deny_unknown_fields`. Asserted rather than assumed: the loaded declaration must equal the same
/// file with the key deleted, so "ignored" means ignored and not "parsed into something".
#[test]
fn a_channel_yaml_still_setting_a_retired_key_loads_and_ignores_it() {
    let retired = "- name: review\n  members: [alice, bob]\n  member_session: resume\n";
    let without = "- name: review\n  members: [alice, bob]\n";

    let load = |yaml: &str| {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("channels.yaml"), yaml).unwrap();
        // Not `.unwrap()` on a `Result` the assertion would swallow: a rejection here is the
        // regression this test exists for, and it must say so by name.
        load_all_channels(tmp.path())
            .unwrap_or_else(|e| panic!("a declaration setting a retired key must load, got: {e}"))
    };

    assert_eq!(
        load(retired),
        load(without),
        "`member_session:` must be ignored, leaving a declaration identical to one without it"
    );
}

#[test]
fn load_all_channels_parses_multiple_channels_in_file_order() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("channels.yaml"), MULTI_CHANNEL_YAML).unwrap();

    let channels = load_all_channels(tmp.path()).unwrap();
    let names: Vec<_> = channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["standup", "retro", "incident"]);
}

#[test]
fn expects_all_string_parses_to_expects_all() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("channels.yaml"),
        "- name: standup\n  members: [pm, coder]\n  expects: all\n",
    )
    .unwrap();

    let channels = load_all_channels(tmp.path()).unwrap();
    assert_eq!(channels[0].expects, Expects::All);
}

#[test]
fn expects_list_parses_to_expects_subset() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("channels.yaml"),
        "- name: standup\n  members: [pm, coder]\n  expects: [pm, coder]\n",
    )
    .unwrap();

    let channels = load_all_channels(tmp.path()).unwrap();
    assert_eq!(
        channels[0].expects,
        Expects::Subset(vec!["pm".to_string(), "coder".to_string()])
    );
}

#[test]
fn expects_omitted_defaults_to_expects_all() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("channels.yaml"),
        "- name: standup\n  members: [pm, coder]\n",
    )
    .unwrap();

    let channels = load_all_channels(tmp.path()).unwrap();
    assert_eq!(channels[0].expects, Expects::All);
}

#[test]
fn expects_malformed_value_is_a_validation_error_naming_the_path() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("channels.yaml");
    std::fs::write(
        &p,
        "- name: standup\n  members: [pm, coder]\n  expects: 42\n",
    )
    .unwrap();

    let err = load_all_channels(tmp.path()).unwrap_err();
    assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
    assert!(err.msg.contains("channels.yaml"));
}

#[test]
fn load_all_channels_reports_validation_error_on_malformed_yaml() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("channels.yaml"),
        "- name: [this is not valid\n",
    )
    .unwrap();

    let err = load_all_channels(tmp.path()).unwrap_err();
    assert_eq!(err.kind, nxs_foundation::error::ErrorKind::Validation);
    assert!(err.msg.contains("channels.yaml"));
}

#[test]
fn on_complete_summarize_without_summary_prompt_still_parses() {
    // Semantic validation ("summary_prompt required iff on_complete == summarize") is
    // `validate_channels`'s job (6j6v.t146, see channel_validate.rs), not the parser's — this must
    // succeed with summary_prompt == None.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("channels.yaml"),
        "- name: standup\n  members: [pm, coder]\n  on_complete: summarize\n",
    )
    .unwrap();

    let channels = load_all_channels(tmp.path()).unwrap();
    assert_eq!(channels[0].on_complete, OnComplete::Summarize);
    assert_eq!(channels[0].summary_prompt, None);
}

#[test]
fn channel_decl_defaults_when_only_required_fields_are_present() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("channels.yaml"),
        "- name: standup\n  members: [pm, coder]\n",
    )
    .unwrap();

    let channels = load_all_channels(tmp.path()).unwrap();
    assert_eq!(
        channels[0],
        ChannelDecl {
            name: "standup".to_string(),
            members: vec!["pm".to_string(), "coder".to_string()],
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
    );
}

// ---- declared hurdles (nxf 6j6v.n92p) ---------------------------------------------------------

/// The `preconditions:` block, in the shape the ticket sketches it, loaded off the real file.
///
/// The two forms are both here on purpose: a hurdle with an `expect:` (the command's output must be
/// exactly that) and one without (the exit status alone decides). `expect: ""` is a THIRD thing
/// again — "printed nothing" — and is only distinguishable from an omitted `expect:` because the
/// field is an `Option`.
#[test]
fn load_all_channels_parses_the_declared_preconditions() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("channels.yaml"),
        "- name: coding\n  members: [coder, finisher]\n  flow: sequential\n  \
         working_tree: exclusive\n  preconditions:\n    - name: remote-nicht-voraus\n      \
         run: git rev-list --count HEAD..@{u}\n      expect: \"0\"\n    - name: baum-sauber\n      \
         run: git status --porcelain\n      expect: \"\"\n    - name: kein-lauf\n      \
         run: test ! -f .nxs/build.lock\n",
    )
    .unwrap();

    let channels = load_all_channels(tmp.path()).unwrap();
    assert_eq!(
        channels[0].preconditions,
        vec![
            nexus_chat::precondition::Precondition {
                name: "remote-nicht-voraus".to_string(),
                run: "git rev-list --count HEAD..@{u}".to_string(),
                expect: Some("0".to_string()),
            },
            nexus_chat::precondition::Precondition {
                name: "baum-sauber".to_string(),
                run: "git status --porcelain".to_string(),
                expect: Some(String::new()),
            },
            nexus_chat::precondition::Precondition {
                name: "kein-lauf".to_string(),
                run: "test ! -f .nxs/build.lock".to_string(),
                expect: None,
            },
        ],
        "`expect: \"\"` and an omitted `expect:` are different declarations and must stay so"
    );
}

/// A channel declared before this field existed reads unchanged — the brought-along hurdles still
/// apply to it, which is exactly why they are not expressible here.
#[test]
fn a_channel_that_declares_no_preconditions_loads_with_none() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("channels.yaml"),
        "- name: standup\n  members: [pm, coder]\n",
    )
    .unwrap();
    assert!(load_all_channels(tmp.path()).unwrap()[0]
        .preconditions
        .is_empty());
}
