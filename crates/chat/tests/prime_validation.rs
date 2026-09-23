//! Black-box tests for `nxc prime`'s referential-integrity partition (nxf ticket 6j6v.v9k3): clean
//! declared roles/channels are offered in the prime roster; broken ones are excluded, and
//! their errors surface only in an interactive (human) context — silently excluded in a spawned
//! (role-session) context, detected via NXC_ACTOR/NXC_WORKER/NXC_SESSION presence. Mirrors
//! `contract.rs`'s `prime_*` conventions.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nexus_chat::workspace::setup(tmp.path(), &nexus_chat::workspace::chat_config())
        .expect("seed chat workspace");
    tmp
}

/// Base helper: `NXC_ACTOR=alice` by default (reproducibility, mirrors this crate's other CLI
/// `nxc()` convention). Under this ticket's new rule, `NXC_ACTOR` being SET means this default
/// helper is a SPAWNED context — tests that need an INTERACTIVE context must explicitly
/// `.env_remove` all three of `NXC_ACTOR`/`NXC_WORKER`/`NXC_SESSION` on top of this, not merely
/// avoid setting them (inherited process env could otherwise carry one in).
fn nxc(tmp: &TempDir) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(tmp.path())
        .env("NXC_ACTOR", "alice")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", "2026-07-08T00:00:00Z");
    c
}

/// The explicitly-interactive variant: unsets all three spawned-context signals on top of the base
/// helper, per the important test-writing nuance called out in this ticket's brief.
fn nxc_interactive(tmp: &TempDir) -> Command {
    let mut c = nxc(tmp);
    c.env_remove("NXC_ACTOR")
        .env_remove("NXC_WORKER")
        .env_remove("NXC_SESSION");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

fn write_role(tmp: &TempDir, handle: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join(format!("{handle}.yaml")),
        format!("handle: {handle}\nsystem_prompt: be helpful\n"),
    )
    .unwrap();
}

fn write_channels(tmp: &TempDir, yaml: &str) {
    std::fs::write(tmp.path().join(".nxs-personas/channels.yaml"), yaml).unwrap();
}

#[test]
fn clean_declared_team_is_offered_with_no_declaration_errors_key() {
    let tmp = workspace();
    write_role(&tmp, "pm");
    write_role(&tmp, "coder");
    write_channels(&tmp, "- name: standup\n  members: [pm, coder]\n");

    let out = nxc(&tmp).args(["--json", "prime"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(
        v["roles"],
        serde_json::json!(["coder", "pm"]),
        "sorted clean role handles: {v}"
    );
    assert_eq!(
        v["channels"],
        serde_json::json!(["standup"]),
        "clean channel names in declaration order: {v}"
    );
    assert!(
        v.get("declaration_errors").is_none(),
        "no errors at all means the key is absent, not empty: {v}"
    );

    // The same clean set, but in an EXPLICITLY interactive context: still no key at all — the
    // gate is "is there an error to report", not "are we spawned" alone.
    let out_interactive = nxc_interactive(&tmp)
        .args(["--json", "prime"])
        .assert()
        .success();
    let v_interactive = json_of(&out_interactive.get_output().stdout);
    assert!(
        v_interactive.get("declaration_errors").is_none(),
        "interactive context with zero errors must still omit the key: {v_interactive}"
    );
}

#[test]
fn broken_channels_yaml_excludes_all_its_channels_not_just_the_broken_one() {
    let tmp = workspace();
    write_role(&tmp, "pm");
    write_role(&tmp, "coder");
    // "standup" is clean; "broken" references a nonexistent member "ghost". Whole-file scoping
    // (this ticket's deliberate design decision) means BOTH must be excluded, not just "broken".
    write_channels(
        &tmp,
        "- name: standup\n  members: [pm, coder]\n- name: broken\n  members: [pm, ghost]\n",
    );

    let out = nxc_interactive(&tmp)
        .args(["--json", "prime"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert!(
        v.get("channels").is_none(),
        "the whole channels.yaml is excluded, including the clean 'standup' channel: {v}"
    );
    let errors = v["declaration_errors"].as_array().unwrap();
    assert!(errors
        .iter()
        .any(|e| e["what"].as_str().unwrap().contains("ghost")));
}

/// Write one persona file with an arbitrary body, for the cases that need more than `write_role`'s
/// two lines.
fn write_role_yaml(tmp: &TempDir, handle: &str, yaml: &str) {
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(roles.join(format!("{handle}.yaml")), yaml).unwrap();
}

/// **The PERSONA side of the referential pass reaches this surface** (nxf 6j6v.st83; added after
/// the review of PR #472 found `validate_roles` unit-tested but its wiring untested).
///
/// The measured case the item was raised on: a persona handle written into `addressable:`, which
/// the loader reads as a CHANNEL name. Before this, `nxc list` and `nxc prime` both exited 0 and
/// the mistake surfaced only when somebody tried to send.
#[test]
fn a_persona_pointing_at_no_declared_channel_is_reported_by_prime() {
    let tmp = workspace();
    write_role(&tmp, "head-of-marketing");
    write_role_yaml(
        &tmp,
        "specialist",
        "handle: specialist\nsystem_prompt: be helpful\naddressable: [head-of-marketing]\n",
    );

    let out = nxc_interactive(&tmp)
        .args(["--json", "prime"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    let errors = v["declaration_errors"].as_array().unwrap();
    let found = errors
        .iter()
        .find(|e| e["file"].as_str() == Some("specialist.yaml"))
        .unwrap_or_else(|| panic!("{v}"));
    assert!(
        found["what"]
            .as_str()
            .unwrap()
            .contains("names no declared channel"),
        "{found}"
    );
}

/// **A persona's referential error does not blank the CHANNEL roster, and the reverse** — the
/// partition claim `facade::validate_declared_team` makes in a comment and that nothing asserted
/// through the real surface until the review of PR #472 asked for it.
#[test]
fn the_two_referential_passes_do_not_exclude_each_other() {
    let tmp = workspace();
    write_role(&tmp, "pm");
    write_role_yaml(
        &tmp,
        "specialist",
        "handle: specialist\nsystem_prompt: be helpful\naddressable: [nowhere]\n",
    );
    // A perfectly good channels.yaml beside the broken persona.
    write_channels(&tmp, "- name: standup\n  members: [pm, specialist]\n");

    let out = nxc_interactive(&tmp)
        .args(["--json", "prime"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    let channels: Vec<&str> = v["channels"]
        .as_array()
        .unwrap_or_else(|| panic!("the clean channel must still be offered: {v}"))
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(
        channels,
        ["standup"],
        "a persona's dangling reference says nothing about channels.yaml: {v}"
    );
    assert!(
        v["declaration_errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["file"].as_str() == Some("specialist.yaml")),
        "…and the persona error is still reported: {v}"
    );
    // The roles roster is unaffected either way — a role file fails at LOAD time or not at all.
    let roles: Vec<&str> = v["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(roles, ["pm", "specialist"], "{v}");
}

/// **The two warnings this change added reach an interactive session**, like the three beside them
/// (review of PR #472, Test Quality): the deprecated channel list on a persona, and a step target
/// repeated under `members:`.
#[test]
fn the_two_new_declaration_warnings_reach_an_interactive_session() {
    let tmp = workspace();
    write_role(&tmp, "coder");
    write_role_yaml(
        &tmp,
        "reviewer",
        "handle: reviewer\nsystem_prompt: be helpful\naddressable: [coding]\n",
    );
    write_channels(
        &tmp,
        "- name: coding\n  members: [coder, reviewer]\n  steps:\n    - id: build\n      \
         target: coder\n      next: judge\n    - id: judge\n      target: reviewer\n",
    );

    let out = nxc_interactive(&tmp)
        .args(["--json", "prime"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    let warnings = v["declaration_warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("{v}"));
    assert!(
        warnings
            .iter()
            .any(|w| w["what"].as_str().unwrap().contains("old spelling")),
        "the deprecated channel list is named: {v}"
    );
    assert!(
        warnings.iter().any(|w| w["what"]
            .as_str()
            .unwrap()
            .contains("as well as in its steps")),
        "…and so is the step target repeated under members: {v}"
    );
    // A warning excludes nothing: the channel is still offered.
    assert!(
        v["channels"].as_array().is_some_and(|c| !c.is_empty()),
        "{v}"
    );
}

#[test]
fn a_missing_channels_yaml_yields_a_clean_roles_only_roster_with_no_spurious_errors() {
    let tmp = workspace();
    write_role(&tmp, "pm");
    // Deliberately no channels.yaml at all.

    let out = nxc(&tmp).args(["--json", "prime"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["roles"], serde_json::json!(["pm"]));
    assert!(v.get("channels").is_none());
    assert!(v.get("declaration_errors").is_none());
}

#[test]
fn human_output_renders_the_address_book_and_gates_declaration_errors_section() {
    let tmp = workspace();
    write_role(&tmp, "pm");
    write_channels(&tmp, "- name: standup\n  members: [pm, ghost]\n");

    let out = nxc_interactive(&tmp).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out.get_output().stdout);
    // "## Declared Team" no longer renders (nxf h4d3, task 3): the address book below is the
    // section that names declared roles/channels now — see `render_declared_team`'s doc comment in
    // `crates/chat/src/facade.rs`.
    assert!(!text.contains("## Declared Team"), "{text}");
    assert!(text.contains("## Who you can address"), "{text}");
    assert!(text.contains("pm"), "{text}");
    assert!(text.contains("## Declaration Errors"), "{text}");
    assert!(text.contains("ghost"), "{text}");

    // Spawned: the Declaration Errors section must not render at all.
    let out2 = nxc(&tmp).arg("prime").assert().success();
    let text2 = String::from_utf8_lossy(&out2.get_output().stdout);
    assert!(!text2.contains("## Declaration Errors"), "{text2}");
}

// ---- the quality warnings (nxf 6j6v.9w08) ----------------------------------------------------
//
// A second stage beside the referential partition above, asking a different question — "is this
// declaration WRITTEN so that it works" — and answering it without excluding anything. Same
// interactive gate, its own `--json` key.

/// **THE ACCEPTANCE, direction 1.** All three checks, on one workspace, reaching a human at the
/// keyboard: engine text copied into a persona (guide class 1), a persona with no routing surface
/// and a channel with none (guide class 3). The declarations are otherwise sound, so nothing here
/// is an error and nothing is excluded from the roster.
#[test]
fn the_three_quality_warnings_reach_an_interactive_session_without_excluding_anything() {
    let tmp = workspace();
    // The two-line minimum: nothing copied, and nothing a caller could route on.
    write_role(&tmp, "pm");
    // …and one that copies the engine's own answering rule into its prompt while describing itself
    // well enough to be chosen from a list — so it trips check (a) and nothing else.
    std::fs::write(
        tmp.path().join(".nxs-personas/coder.yaml"),
        "handle: coder\njob_description: Implements a work order on a branch and merges it.\n\
         system_prompt: When you are done, answer with `nxc reply --thread <id>`.\n",
    )
    .unwrap();
    // A channel with real members and no way for a caller to tell what it is for.
    write_channels(&tmp, "- name: standup\n  members: [pm, coder]\n");

    let out = nxc_interactive(&tmp)
        .args(["--json", "prime"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);

    assert!(
        v.get("declaration_errors").is_none(),
        "every declaration here RESOLVES — a warning is not an error: {v}"
    );
    assert_eq!(
        v["roles"],
        serde_json::json!(["coder", "pm"]),
        "and nothing is excluded from the roster by a warning: {v}"
    );
    assert_eq!(v["channels"], serde_json::json!(["standup"]), "{v}");

    let warnings = v["declaration_warnings"]
        .as_array()
        .unwrap_or_else(|| panic!("the three warnings are reported: {v}"));
    let said: Vec<(&str, &str)> = warnings
        .iter()
        .map(|w| (w["file"].as_str().unwrap(), w["what"].as_str().unwrap()))
        .collect();
    assert_eq!(said.len(), 3, "{said:?}");
    // (a) engine text copied in, named with the invocation it copied.
    assert_eq!(said[0].0, "coder.yaml");
    assert!(said[0].1.contains("`nxc reply`"), "{said:?}");
    // (b) a persona with no routing surface — `write_role` writes the two-line minimum.
    assert_eq!(said[1].0, "pm.yaml");
    assert!(
        said[1].1.contains("`job_description` is not declared"),
        "{said:?}"
    );
    // (c) the same surface on a channel.
    assert_eq!(said[2].0, "channels.yaml");
    assert!(
        said[2]
            .1
            .contains("channel \"standup\" declares no `description`"),
        "{said:?}"
    );

    // …and every one of them is still fully usable: a warned persona and a warned channel are both
    // offered as targets, by name, in the directory a caller actually reads. "It loads" would be a
    // weaker claim than the one this stage makes.
    let listed = String::from_utf8_lossy(
        &nxc_interactive(&tmp)
            .arg("list")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .to_string();
    for offered in ["`coder`", "`pm`", "`standup`"] {
        assert!(
            listed.contains(offered),
            "a warning excludes nothing — {offered} is still addressable:\n{listed}"
        );
    }
}

/// **THE ACCEPTANCE, direction 2.** The same defects, read from a SPAWNED context: the `--json` key
/// is absent entirely — not an empty array — exactly as `declaration_errors` is, and neither the
/// section nor the guide pointer reaches the human view. A spawned session has no terminal and no
/// author to tell.
///
/// **Both halves run against the SAME fixture, and that is the point** (review of PR #465, Test
/// Quality #1). Asserting only the absence would pass unchanged with the whole feature deleted —
/// it would prove that nothing is there, not that something is being SUPPRESSED. So the interactive
/// half below establishes that this workspace really does produce warnings, and the spawned half
/// then means what its name says.
#[test]
fn a_spawned_context_is_told_none_of_them() {
    let tmp = workspace();
    write_role(&tmp, "pm");
    write_channels(&tmp, "- name: standup\n  members: [pm]\n");

    // First: there IS something to suppress. Without this the assertions below are vacuous.
    let interactive = json_of(
        &nxc_interactive(&tmp)
            .args(["--json", "prime"])
            .assert()
            .success()
            .get_output()
            .stdout,
    );
    assert_eq!(
        interactive["declaration_warnings"].as_array().map(Vec::len),
        Some(2),
        "the fixture trips both routing-surface checks when nobody suppresses them: {interactive}"
    );
    let interactive_text = String::from_utf8_lossy(
        &nxc_interactive(&tmp)
            .arg("prime")
            .assert()
            .success()
            .get_output()
            .stdout,
    )
    .to_string();
    assert!(
        interactive_text.contains("## Declaration Warnings")
            && interactive_text.contains("nxc guide writing-declarations"),
        "…and the human view carries both the section and the pointer:\n{interactive_text}"
    );

    // Then: the spawned view of that same workspace is told none of it.
    let out = nxc(&tmp).args(["--json", "prime"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert!(
        v.get("declaration_warnings").is_none(),
        "absent, not empty, in a spawned context: {v}"
    );

    let out2 = nxc(&tmp).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out2.get_output().stdout);
    assert!(!text.contains("## Declaration Warnings"), "{text}");
    assert!(!text.contains("nxc guide writing-declarations"), "{text}");
}

/// The gate is "is there something to report", not "are we interactive" — a workspace whose
/// declarations are written properly keeps the pre-9w08 shape byte for byte, while the human view
/// still carries the one line that points at the topic. Both halves matter: the key must not appear
/// empty, and the pointer must not hang on there being a complaint.
#[test]
fn a_clean_workspace_omits_the_key_and_still_gets_the_pointer() {
    let tmp = workspace();
    std::fs::create_dir_all(tmp.path().join(".nxs-personas")).unwrap();
    std::fs::write(
        tmp.path().join(".nxs-personas/pm.yaml"),
        "handle: pm\n\
         job_description: Hands a work order over, and reads the verdict that comes back.\n\
         system_prompt: be helpful\n",
    )
    .unwrap();
    write_channels(
        &tmp,
        "- name: standup\n  members: [pm]\n  description: the whole team's daily sync\n",
    );

    let out = nxc_interactive(&tmp)
        .args(["--json", "prime"])
        .assert()
        .success();
    let v = json_of(&out.get_output().stdout);
    assert!(
        v.get("declaration_warnings").is_none(),
        "nothing to report means no key at all: {v}"
    );

    let out2 = nxc_interactive(&tmp).arg("prime").assert().success();
    let text = String::from_utf8_lossy(&out2.get_output().stdout);
    assert!(!text.contains("## Declaration Warnings"), "{text}");
    assert!(
        text.contains("`nxc guide writing-declarations`"),
        "the pointer is standing advice, not a footnote to a complaint: {text}"
    );
}

// Six tests stood here and above, one per shape of a broken or duplicate WORKFLOW declaration: an
// unresolved role target, an unresolved `goto`, several clean ones in declaration order, one broken
// among several, and a duplicate name as a HARD prime failure rather than a partitioned error. All
// six went with the run engine (6j6v.dvyq §3) — there is no workflow declaration left to validate.
// The partition rule they exercised is the same one
// `broken_channels_yaml_excludes_all_its_channels_not_just_the_broken_one` above still holds, on
// the declaration kind that survives.
