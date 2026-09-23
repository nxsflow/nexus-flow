//! `Definitions` — the declaration catalogue both seams resolve through (nxf 6j6v.h2fr).
//!
//! Personas, channels and flows are read from the on-disk `.nxs-personas/` folder — a legacy
//! `roles/` folder is still read, and reported as legacy, but never written (nxf 6j6v.dvyq step 4).
//!
//! **ONE catalogue, ONE source.** This header said "one catalogue, TWO sources" and described this
//! type as the entry point that lets a caller supply declarations *instead* of a folder, so an app
//! would not have to write one into a user's project. That was 6j6v.h2fr's design, and 6j6v.dvyq
//! step 6 reversed it: `DefinitionSource::Supplied` and `Engine::set_definitions` are gone, and an
//! app reads the same folder the CLI does — even a later in-app role editor writes into it. What
//! survives is the validating constructor ([`Definitions::new`]) that the folder loader and this
//! file's own cases both build through: a construction seam, not a second source.

use nexus_chat::channel::ChannelDecl;
use nexus_chat::definitions::Definitions;
use nexus_chat::error::ErrorKind;
use nexus_chat::role::RoleDecl;

fn role(yaml: &str) -> RoleDecl {
    serde_yaml::from_str(yaml).expect("test role parses")
}

fn channel(yaml: &str) -> ChannelDecl {
    serde_yaml::from_str(yaml).expect("test channel parses")
}

fn simple_role(handle: &str) -> RoleDecl {
    role(&format!("handle: {handle}\nsystem_prompt: be helpful\n"))
}

// ---- construction-time validation --------------------------------------------------------

#[test]
fn new_rejects_a_role_handle_with_a_path_separator() {
    // The path-traversal reason this guard originally existed for (a handle feeding
    // `roles.join(format!("{handle}.yaml"))`) disappears with host-supplied definitions — but the
    // handle still becomes half of a qualified `origin/handle` chat identity, and a `/` inside it
    // would make that identity unparseable back into its two halves. So the rejection stays, for a
    // reason that survives the move.
    for bad in ["a/b", "a\\b", ".."] {
        let err = Definitions::new(vec![simple_role(bad)], vec![])
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation, "handle {bad:?}");
        // Compare against the debug form the message itself uses — a backslash is escaped there.
        assert!(
            err.msg.contains(&format!("{bad:?}")),
            "error must name the handle: {}",
            err.msg
        );
    }
}

#[test]
fn new_rejects_an_empty_role_handle() {
    let err = Definitions::new(vec![simple_role("\"\"")], vec![])
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
}

#[test]
fn new_rejects_every_engine_internal_handle_as_a_class_not_as_a_list() {
    // The engine's own synthetic identities — `__synth__` and `__delivered__` (the completion
    // markers) and `__channel__` (the two-level channel's SUPERVISOR, nxf 6j6v.pf6j). A declared role
    // sharing one would collide with that mechanism once qualified into `expects_reply_from` or
    // compared against a thread's `opener`, which is how a member thread is recognised at all.
    //
    // Reserved as a CLASS (`__`-prefixed), which is why `__anything__` is in this list: `__channel__`
    // spent a whole round documented as reserved while an enumeration of two handles quietly let it
    // through, and the two items after pf6j each add engine-internal identities of their own. The
    // rule has to cover the marker nobody has written yet.
    for reserved in [
        "__synth__",
        "__delivered__",
        "__channel__",
        "__anything__",
        "__",
    ] {
        let err = Definitions::new(vec![simple_role(reserved)], vec![])
            .map(|_| ())
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Validation, "handle {reserved:?}");
        assert!(
            err.msg.contains("reserved"),
            "the error must say WHY, got: {}",
            err.msg
        );
    }
}

#[test]
fn new_rejects_duplicate_role_handles() {
    // The directory layout made this impossible (one file per handle); a caller-supplied `Vec` does
    // not. Two roles with one handle means `role()` would silently pick one of them.
    let err = Definitions::new(vec![simple_role("a"), simple_role("a")], vec![])
        .map(|_| ())
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains('a'), "{}", err.msg);
}

#[test]
fn new_rejects_duplicate_channel_names() {
    let err = Definitions::new(
        vec![simple_role("a")],
        vec![
            channel("name: review\nmembers: [a]\n"),
            channel("name: review\nmembers: [a]\n"),
        ],
    )
    .map(|_| ())
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("review"), "{}", err.msg);
}

// `new_rejects_duplicate_workflow_names` stood here, and `workflow_selection_mirrors_the_cli_rules`
// below the lookups. Both went with `Definitions::workflow` and the declared workflows themselves
// (6j6v.dvyq §3). Duplicate ROLE handles and CHANNEL names are still refused, right above.

#[test]
fn new_accepts_an_empty_catalogue() {
    // A fresh workspace declares nothing. That is not an error — it just means no declared team.
    let defs = Definitions::new(vec![], vec![]).unwrap();
    assert!(defs.roles().is_empty());
    assert!(defs.channels().is_empty());
}

// ---- lookups ------------------------------------------------------------------------------

#[test]
fn role_resolves_by_the_declared_handle_field() {
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    assert_eq!(defs.role("coder").unwrap().handle, "coder");
}

#[test]
fn role_returns_not_found_for_a_well_formed_unknown_handle() {
    // The error-kind split matters and is part of the contract both seams inherit: a MALFORMED
    // handle is the caller's `validation` mistake, a well-formed handle naming nothing declared is
    // `not_found`. Collapsing them would make an app unable to tell "you typed something illegal"
    // from "that role does not exist here".
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let err = defs.role("nope").map(|_| ()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert!(err.msg.contains("nope"), "{}", err.msg);
}

#[test]
fn role_returns_validation_for_a_malformed_handle() {
    let defs = Definitions::new(vec![simple_role("coder")], vec![]).unwrap();
    let err = defs.role("../etc/passwd").map(|_| ()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
}

#[test]
fn channel_is_a_pure_lookup_and_declared_channel_fails_closed_on_a_malformed_one() {
    // Two lookups on purpose, because the existing code genuinely wants both: `nxc tick`
    // resolves the channel backing a thread with a plain find (it is reporting on a board that
    // already exists), while every verb that is about to ACT on a channel re-checks it and refuses
    // a malformed declaration rather than silently degrading (`validate_channel_for_use`'s
    // fail-closed contract, independent review Integrity #4).
    // The one self-contained malformation `validate_channel_for_use` catches: `summarize` with no
    // prompt for the synthesizer to run on, which used to degrade silently into an empty system
    // prompt instead of failing.
    let malformed = channel("name: review\nmembers: [a]\non_complete: summarize\n");
    let defs = Definitions::new(vec![], vec![malformed]).unwrap();

    assert!(
        defs.channel("review").is_some(),
        "the pure lookup must still find it"
    );
    let err = defs.declared_channel("review").map(|_| ()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("review"), "{}", err.msg);
}

#[test]
fn declared_channel_is_none_for_an_undeclared_name() {
    // A workspace with no declared channels must fall straight through to the raw-channel-id
    // behaviour, never fail — the "unknown name is not an error" contract `resolve_declared_channel`
    // already carries.
    let defs = Definitions::new(vec![], vec![]).unwrap();
    assert!(defs.declared_channel("anything").unwrap().is_none());
}

// ---- from_dir -----------------------------------------------------------------------

#[test]
fn from_dir_over_a_missing_directory_is_empty_not_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let defs = Definitions::from_dir(&tmp.path().join("does-not-exist")).unwrap();
    assert!(defs.roles().is_empty());
    assert!(defs.channels().is_empty());
}

#[test]
fn from_dir_matches_the_individual_loaders() {
    // The differential that keeps the CLI's behaviour honest through the move: whatever the two
    // existing loaders produce for a directory, the catalogue must hold exactly that.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::write(dir.join("alice.yaml"), "handle: alice\nsystem_prompt: A\n").unwrap();
    std::fs::write(dir.join("bob.yaml"), "handle: bob\nsystem_prompt: B\n").unwrap();
    std::fs::write(
        dir.join("channels.yaml"),
        "- name: review\n  members: [alice, bob]\n",
    )
    .unwrap();
    let defs = Definitions::from_dir(dir).unwrap();
    assert_eq!(defs.roles(), nexus_chat::role::load_all_roles(dir).unwrap());
    assert_eq!(
        defs.channels(),
        nexus_chat::channel::load_all_channels(dir).unwrap()
    );
}

#[test]
fn from_dir_surfaces_a_duplicate_handle_across_two_files() {
    // Two files declaring the same `handle:` is ambiguous — resolvable only by which file the
    // scan happened to read first. The directory layout never prevented it; the catalogue refuses
    // it, so the ambiguity surfaces at load instead of silently picking a winner at trigger time.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.yaml"), "handle: dup\nsystem_prompt: A\n").unwrap();
    std::fs::write(tmp.path().join("b.yaml"), "handle: dup\nsystem_prompt: B\n").unwrap();
    let err = Definitions::from_dir(tmp.path()).map(|_| ()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(err.msg.contains("dup"), "{}", err.msg);
}

// ---- the working-tree derivation: upward from the declared members, ONE hop (nxf 6j6v.1xw1) ----

#[test]
fn a_channel_needs_the_working_tree_when_one_of_its_declared_members_does() {
    // Owner, 2026-08-14: "wenn die Kanal-Deklaration in seiner Kette eine Persona hat, die einen
    // geschuetzten Arbeitsbereich benoetigt, dann gilt der gesamte Kanal als schuetzenswert." The
    // author declares the need once, on the persona that builds, and does not repeat it on every
    // channel that persona belongs to.
    let defs = Definitions::new(
        vec![
            role("handle: coder\nsystem_prompt: build\nworking_tree: exclusive\n"),
            simple_role("coding-lead"),
        ],
        vec![
            channel("name: coding\nmembers: [coding-lead, coder]\n"),
            channel("name: talking\nmembers: [coding-lead]\n"),
        ],
    )
    .unwrap();

    assert!(defs.channel_needs_working_tree(defs.channel("coding").unwrap()));
    assert!(
        !defs.channel_needs_working_tree(defs.channel("talking").unwrap()),
        "a channel none of whose members declares the need does not have it"
    );
}

#[test]
fn the_derivation_is_one_hop_so_review_does_not_inherit_codings_need_through_a_shared_member() {
    // The stop, and the reason for it (6j6v.1xw1): without it `#coding` hands its need to `#review`
    // through the member the two share, `#review` hands it on again, and every channel in a
    // workspace ends up exclusive — including the ones whose members only READ. What counts is a
    // member's OWN declaration, never one it would inherit from another channel.
    let defs = Definitions::new(
        vec![
            role("handle: coder\nsystem_prompt: build\nworking_tree: exclusive\n"),
            simple_role("coding-lead"),
            simple_role("reviewer"),
        ],
        vec![
            channel("name: coding\nmembers: [coding-lead, coder]\n"),
            channel("name: review\nmembers: [coding-lead, reviewer]\n"),
        ],
    )
    .unwrap();

    assert!(defs.channel_needs_working_tree(defs.channel("coding").unwrap()));
    assert!(
        !defs.channel_needs_working_tree(defs.channel("review").unwrap()),
        "`coding-lead` is a member of the now-protected #coding and declares nothing itself, so it \
         carries nothing into #review"
    );
}

#[test]
fn a_channel_that_declares_the_need_itself_keeps_it_whatever_its_members_say() {
    // The pre-existing half, unchanged: the epic's own example declares `working_tree: exclusive`
    // on the `review` CHANNEL, whose four members declare nothing.
    let defs = Definitions::new(
        vec![simple_role("reviewer")],
        vec![channel(
            "name: review\nmembers: [reviewer]\nworking_tree: exclusive\n",
        )],
    )
    .unwrap();
    assert!(defs.channel_needs_working_tree(defs.channel("review").unwrap()));
}

#[test]
fn the_derivation_reads_the_declared_members_not_the_effective_fan_out() {
    // `expects: subset` narrows who is asked on a given turn. The need for a protected working copy
    // is a property of the CHANNEL and must not depend on which members a particular call reaches —
    // otherwise the same channel would be exclusive or shared depending on who sent to it.
    let defs = Definitions::new(
        vec![
            role("handle: coder\nsystem_prompt: build\nworking_tree: exclusive\n"),
            simple_role("coding-lead"),
        ],
        vec![channel(
            "name: coding\nmembers: [coding-lead, coder]\nexpects: [coding-lead]\n",
        )],
    )
    .unwrap();
    assert!(defs.channel_needs_working_tree(defs.channel("coding").unwrap()));
}

#[test]
fn a_member_handle_naming_no_declared_role_contributes_nothing_rather_than_erroring() {
    // Referential integrity is deliberately advisory here (see `Definitions::new`), so this has to
    // answer rather than fail — and the honest answer for a member nobody declared is "it says
    // nothing about the working copy".
    let defs =
        Definitions::new(vec![], vec![channel("name: ghosts\nmembers: [nobody]\n")]).unwrap();
    assert!(!defs.channel_needs_working_tree(defs.channel("ghosts").unwrap()));
}

// ---- the flow-cycle gate (nxf 6j6v.hq71) ------------------------------------------------------

#[test]
fn new_refuses_a_catalogue_whose_channel_flow_reaches_itself() {
    // Since a flow step may address another CHANNEL, a catalogue can describe a cycle — and a cycle
    // does not degrade the way a dangling member reference does: it RUNS, opening a real thread and
    // starting a real paid session at every hop until the depth guard stops the chain. So it is
    // refused where every seam builds its catalogue, which is the only gate there is over declared
    // channel content (`validate_channels` is advisory, and `cargo xtask channels check` enforces the
    // release promotion rings and never reads a declaration).
    let err = Definitions::new(
        vec![simple_role("reviewer")],
        vec![
            channel("name: a\nmembers: [b]\n"),
            channel("name: b\nmembers: [a, reviewer]\n"),
        ],
    )
    .map(|_| ())
    .expect_err("a flow that reaches itself is not a workspace anyone can open");
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("reaches itself") && err.msg.contains("channels.yaml"),
        "{err:?}"
    );
}

#[test]
fn new_accepts_a_channel_named_like_one_of_its_own_members_role() {
    // The tie-break that keeps `members` meaning what it always meant: a workspace declaring the
    // channel `pm` and the ROLE `pm` (a shipped shape — `tests/public_channel.rs`) is not a cycle,
    // because the member `pm` names the role.
    Definitions::new(
        vec![simple_role("pm"), simple_role("dev")],
        vec![channel("name: pm\nmembers: [pm, dev]\nkind: public\n")],
    )
    .expect("a role wins a name a channel also declares");
}
