//! Offline/deterministic validation for the `role-runtime-v3` example team (nxf ticket 6j6v.3nha):
//! proves the declared content — 6 roles and one `channels.yaml` (the `review` quorum) — loads
//! cleanly and is referentially sound, mirroring the same "load + validate, no I/O beyond reading
//! these exact files" discipline `channel_validate.rs` already established for hand-built
//! fixtures. It also covered the example's two declared WORKFLOWS until 6j6v.dvyq §3 removed the
//! run engine and the declarations with it. Here the fixture IS the real example content under
//! `examples/role-runtime-v3/.nxs-personas/`, so a broken declaration in the shipped example itself
//! — not just a synthetic test fixture — fails loudly.
//!
//! This is deliberately NOT a live SDK smoke (that is a separate, later ticket, 6j6v.8e45, which
//! depends on this one): every assertion here is pure load/validate over YAML content, no `nxc`
//! subprocess, no worker, no database.

use nexus_chat::channel::{self, ConsolidatorOutput, OnComplete};
use nexus_chat::role::{self, Model, WorkingTree};
use std::path::{Path, PathBuf};

fn v3_personas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/role-runtime-v3/.nxs-personas")
}

#[test]
fn all_six_v3_roles_load_cleanly() {
    let roles = role::load_all_roles(&v3_personas_dir())
        .unwrap_or_else(|e| panic!("loading v3 roles: {e}"));
    let mut handles: Vec<&str> = roles.iter().map(|r| r.handle.as_str()).collect();
    handles.sort_unstable();
    assert_eq!(
        handles,
        vec![
            "code-quality",
            "coder",
            "general",
            "integrity",
            "pm",
            "test-quality",
        ],
        "expected exactly the 6 declared v3 roles, got: {handles:?}"
    );
}

#[test]
fn v3_channels_yaml_loads_both_declared_channels_cleanly() {
    let channels = channel::load_all_channels(&v3_personas_dir())
        .unwrap_or_else(|e| panic!("loading v3 channels.yaml: {e}"));
    let names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["build-and-ship", "review"],
        "the ORDER a work order runs through, and the quorum its second step addresses — declared \
         in that order because `flow: sequential` reads its members in declaration order"
    );
    let ship = &channels[0];
    assert_eq!(ship.members, ["coder", "review"]);
    assert_eq!(
        ship.flow,
        channel::Flow::Sequential,
        "the order is DECLARED, which is what makes `send --to build-and-ship` a flow"
    );
    let review = &channels[1];
    assert_eq!(review.name, "review");
    assert_eq!(
        review.members,
        vec!["general", "code-quality", "test-quality", "integrity"],
        "in DECLARATION order, which is now load-bearing: the quorum is ordered (nxf 6j6v.2d00) \
         and `flow: sequential` reads its members in exactly this order"
    );
    assert_eq!(
        review.flow,
        channel::Flow::Sequential,
        "ONE reviewer in the checkout at a time (nxf 6j6v.2d00). Three of these four run the \
         project's own quality gates themselves, so a parallel quorum hands one working copy and \
         one `target/` to three concurrent cargo runs — two of different shapes overwrite each \
         other's artifacts, which this repo has measured. The channel's own comment carries the \
         whole argument and what it costs; this is the pin, because the field is the entire \
         mechanism and a stray deleted line rots it silently"
    );
}

/// nxf 6j6v.3nzw (task 14, round 2). This example was given `working_tree: exclusive` on
/// `coder.yaml` and the `review` channel precisely so the declaration is reachable by a real role
/// author, not documented only in a changelog fragment that gets deleted at release time — but a
/// declaration only `load_all_roles`/`load_all_channels` parse without reading is a declaration a
/// typo'd `sahred` or a stray deleted line rots silently, since every OTHER assertion here checks
/// referential soundness, never this field's actual value. Pin both: `coder` writes the branch
/// across `implement`/`mitigate`/`merge`, and `review`'s four members each run the project's own
/// quality gates directly against that same working copy — the exact coder-vs-review collision
/// this epic exists to serialize instead of let race.
#[test]
fn v3_coder_and_review_declare_working_tree_exclusive() {
    let roles = role::load_all_roles(&v3_personas_dir()).unwrap();
    let channels = channel::load_all_channels(&v3_personas_dir()).unwrap();

    let coder = roles.iter().find(|r| r.handle == "coder").expect("coder");
    assert_eq!(
        coder.working_tree,
        WorkingTree::Exclusive,
        "coder holds the branch across implement/mitigate/merge and must lease the working \
         copy for the whole chain"
    );

    let review = channels
        .iter()
        .find(|c| c.name == "review")
        .expect("a channel named review must be declared");
    assert_eq!(
        review.working_tree,
        WorkingTree::Exclusive,
        "the review quorum runs the project's own quality gates against the same working copy \
         the coder is building on"
    );
}

/// nxf 6j6v.e9qj (fix round 1, review finding F7) — the same pin, for the same reason, on the field
/// that decides which model this example's fold actually runs on.
///
/// There is no drift gate over channel-declaration CONTENT anywhere in this repo: `validate_channels`
/// is a validator (it checks referential soundness and the `summarize`/`summary_model` pairing, never
/// a particular value), and `docs/generated/` renders the `nxf` clap tree only. This file IS the gate
/// for the shipped example, by hand — which is why the field a reader is meant to LEARN from here
/// gets an assertion rather than a comment.
#[test]
fn v3_review_declares_the_model_its_consolidator_folds_with() {
    let channels = channel::load_all_channels(&v3_personas_dir()).unwrap();
    let review = channels
        .iter()
        .find(|c| c.name == "review")
        .expect("a channel named review must be declared");

    assert_eq!(
        review.summary_model,
        Some(Model::Opus),
        "this fold does not restate a verdict, it DECIDES one — it aggregates four reviewers into \
         the `outcome:` line that routes `build-and-ship` to merge or back to mitigate — so it \
         declares its model instead of taking the junior-band default"
    );
    assert_eq!(
        review.consolidator(),
        ConsolidatorOutput::Fold {
            model: Model::Opus,
            prompt: review.summary_prompt.clone().expect("a declared prompt"),
        },
        "…and the declared pair is what the resolved consolidator carries, so the example cannot \
         drift from what the engine would run for it"
    );
}

#[test]
fn the_shipped_example_team_shows_the_cut_nxc_list_actually_makes() {
    // nxf 6j6v.frek. The repo's own example is the one team a reader can look at, and until this
    // was declared it exhibited the exact bug frek exists to fix: none of the four reviewers
    // declared `addressable`, so `Addressable::default()` (General) made every one of them a direct
    // entry — `nxc send --to code-quality` invited, and refused. The new cut was demonstrable only
    // in an out-of-repo workspace.
    let roles = role::load_all_roles(&v3_personas_dir()).expect("v3 roles");
    let channels = channel::load_all_channels(&v3_personas_dir()).expect("v3 channels");

    for handle in ["general", "code-quality", "test-quality", "integrity"] {
        let decl = roles.iter().find(|r| r.handle == handle).expect(handle);
        assert_eq!(
            decl.addressable,
            nexus_chat::role::Addressable::Nobody,
            "{handle} is reached through the quorum, never alone"
        );
        assert!(!decl.addressable.allows_direct_from_anyone(), "{handle}");
        // The route in is DERIVED now (nxf 6j6v.g0yn) — the persona says only that nobody may send
        // it a direct message, and `review` is what casts it.
        assert_eq!(
            nexus_chat::channel::channels_casting(&channels, handle),
            ["review"],
            "{handle} is cast by the quorum it answers in"
        );
    }

    let md = nexus_chat::persona::Directory::full(&roles, &channels).render_markdown();
    for hidden in ["general", "code-quality", "test-quality", "integrity"] {
        assert!(
            !md.contains(&format!("(handle: `{hidden}`")),
            "a reviewer must not be an entry of its own:\n{md}"
        );
    }
    // Two personas and the channel — and the channel names the four it reaches, which is where
    // their membership went (acceptance 4).
    assert!(md.contains("(handle: `coder`)"), "{md}");
    assert!(md.contains("(handle: `pm`)"), "{md}");
    assert!(
        md.contains(
            "**Review** (handle: `review`, members: general, code-quality, test-quality, integrity)"
        ),
        "{md}"
    );
}

#[test]
fn v3_channels_validate_cleanly_against_v3_roles() {
    let roles = role::load_all_roles(&v3_personas_dir()).unwrap();
    let channels = channel::load_all_channels(&v3_personas_dir()).unwrap();
    let errors = channel::validate_channels(&roles, &channels);
    assert!(
        errors.is_empty(),
        "expected no validation errors, got: {errors:?}"
    );
}

/// The ONE mechanically-critical content requirement (brief §6): the `review` channel's
/// `on_complete` must be `Summarize`, its `summary_prompt` must be present, and that prompt's text
/// must contain the EXACT literal lines `parse_outcome_token`/`Transition.when` depend on —
/// `outcome: approved` and `outcome: changes` — verbatim, matching this workflow's own `when:`
/// values. A malformed synthesizer prompt would fail loudly at runtime (an unmatched outcome with
/// no default arm on `review` is a `validation` error), but this guard catches it long before that.
#[test]
fn v3_review_channel_summary_prompt_asks_for_one_merge_verdict_on_the_last_line() {
    let channels = channel::load_all_channels(&v3_personas_dir()).unwrap();
    let review = channels
        .iter()
        .find(|c| c.name == "review")
        .expect("a channel named review must be declared");
    assert_eq!(review.on_complete, OnComplete::Summarize);
    let prompt = review
        .summary_prompt
        .as_deref()
        .expect("review channel must declare a summary_prompt");
    // The fold DECIDES, so the shape of what it must say is part of the declaration rather than
    // left to the model. It used to be an `outcome: <token>` line an engine parsed; since 6j6v.dvyq
    // §3 the reader is the REQUESTER, so the line is the same verdict the four members give.
    assert!(
        prompt.contains("Ready to merge? yes") && prompt.contains("Ready to merge? no"),
        "summary_prompt must name both verdicts it can end on, got:\n{prompt}"
    );
    assert!(
        prompt.contains("LAST line"),
        "…and must say where the verdict goes, got:\n{prompt}"
    );
}
