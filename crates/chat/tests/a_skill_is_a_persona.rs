//! **A skill is a persona** (nxf 6j6v.dw16, 6j6v.k3qy, epic 6j6v.phcx) — the declaration form of
//! the Agent Skills specification, `<name>/SKILL.md`, beside the YAML form, and one file per
//! channel under `channels/` beside `channels.yaml`.
//!
//! The published skill this slice is accepted on is tested through the real binary
//! (`crates/nxs/tests/a_published_skill_is_a_persona.rs`); here the form is pinned field by field,
//! against the catalogue every seam resolves through and against `Engine`, the seam an embedding
//! app reads.

use std::path::Path;

use nexus_chat::definitions::{DeclarationForm, DeclarationKind, Definitions};
use nexus_chat::role::Stage;
use tempfile::TempDir;

const NOW: &str = "2026-10-05T10:00:00Z";

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn skill(repo: &Path, name: &str, frontmatter: &str, body: &str) {
    write(
        &repo.join(format!(".nxs-personas/{name}/SKILL.md")),
        &format!("---\n{frontmatter}---\n{body}"),
    );
}

fn resolve(repo: &Path) -> nexus_chat::error::Result<Definitions> {
    Definitions::resolve_with_user_dir(repo, None)
}

#[test]
fn allowed_tools_keeps_the_three_states_tools_has_and_takes_a_list() {
    let repo = TempDir::new().unwrap();
    skill(repo.path(), "absent", "name: absent\n", "A.\n");
    skill(
        repo.path(),
        "empty",
        "name: empty\nallowed-tools:\n",
        "E.\n",
    );
    skill(
        repo.path(),
        "blank",
        "name: blank\nallowed-tools: \"\"\n",
        "B.\n",
    );
    skill(
        repo.path(),
        "spaced",
        "name: spaced\nallowed-tools: Bash(git add *) Bash(git commit *) Read\n",
        "S.\n",
    );
    skill(
        repo.path(),
        "listed",
        "name: listed\nallowed-tools:\n  - Read\n  - Grep\n",
        "L.\n",
    );
    skill(
        repo.path(),
        "commas",
        "name: commas\nallowed-tools: Read, Grep\n",
        "C.\n",
    );
    let defs = resolve(repo.path()).unwrap();
    let tools = |h: &str| defs.role(h).unwrap().tools.clone();
    assert_eq!(
        tools("absent"),
        None,
        "absent: the runtime's full default set"
    );
    assert_eq!(tools("empty"), Some(vec![]), "an empty value: no tools");
    assert_eq!(tools("blank"), Some(vec![]));
    assert_eq!(
        tools("spaced"),
        Some(vec![
            "Bash(git add *)".to_string(),
            "Bash(git commit *)".to_string(),
            "Read".to_string()
        ]),
        "split outside parentheses"
    );
    assert_eq!(tools("listed"), Some(vec!["Read".into(), "Grep".into()]));
    assert_eq!(tools("commas"), Some(vec!["Read".into(), "Grep".into()]));
}

#[test]
fn the_specification_fields_and_the_nxs_block_map_onto_the_declaration() {
    let repo = TempDir::new().unwrap();
    skill(
        repo.path(),
        "pm",
        "name: pm\n\
         description: >-\n  Turns a product idea into board items.\n\
         allowed-tools: Bash Read\n\
         license: Apache-2.0\n\
         compatibility: Requires git\n\
         metadata:\n  author: example-org\n  version: \"1.0\"\n\
         when_to_use: A Claude Code field nexus-flow leaves alone.\n\
         nxs:\n  title: Product Manager\n  expected_output: One message.\n  stage: senior\n  \
         permissions: bypassPermissions\n  addressable:\n    humans: true\n  requires: [board, memory]\n",
        "You are the PM of this project.\n\nSee `references/guide.md`.\n",
    );
    let defs = resolve(repo.path()).unwrap();
    let pm = defs.role("pm").unwrap();
    assert_eq!(pm.handle, "pm");
    assert_eq!(pm.job_title.as_deref(), Some("Product Manager"));
    assert_eq!(
        pm.job_description.as_deref(),
        Some("Turns a product idea into board items.")
    );
    assert_eq!(
        pm.system_prompt, "You are the PM of this project.\n\nSee `references/guide.md`.\n",
        "the body is the prompt, unchanged"
    );
    assert_eq!(pm.tools, Some(vec!["Bash".into(), "Read".into()]));
    assert_eq!(pm.expected_output.as_deref(), Some("One message."));
    assert_eq!(pm.stage, Some(Stage::Senior));
    assert_eq!(pm.permissions.as_deref(), Some("bypassPermissions"));
    let humans_only: nexus_chat::role::Addressable = serde_yaml::from_str("humans: true").unwrap();
    assert_eq!(
        pm.addressable, humans_only,
        "the same parser as the YAML form"
    );

    let file = defs
        .source()
        .unwrap()
        .file_of(DeclarationKind::Persona, "pm")
        .unwrap();
    assert_eq!(file.form, DeclarationForm::Skill);
    assert_eq!(file.folder, repo.path().join(".nxs-personas/pm"));
    assert_eq!(
        file.requires,
        vec!["board".to_string(), "memory".to_string()]
    );
    assert_eq!(file.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(file.compatibility.as_deref(), Some("Requires git"));
    assert_eq!(
        file.metadata,
        Some(serde_json::json!({"author": "example-org", "version": "1.0"}))
    );
}

#[test]
fn a_skill_without_nxs_is_a_persona_with_the_defaults() {
    let repo = TempDir::new().unwrap();
    skill(
        repo.path(),
        "helper",
        "name: helper\ndescription: Helps with things in a sentence long enough.\n",
        "Do the thing.\n",
    );
    let defs = resolve(repo.path()).unwrap();
    let helper = defs.role("helper").unwrap();
    let yaml: nexus_chat::role::RoleDecl = serde_yaml::from_str(
        "handle: helper\njob_description: Helps with things in a sentence long enough.\nsystem_prompt: \"Do the thing.\\n\"\n",
    )
    .unwrap();
    assert_eq!(
        helper, &yaml,
        "exactly the YAML declaration with the same three fields: every default unchanged"
    );
}

#[test]
fn a_key_that_belongs_at_the_top_is_refused_under_nxs() {
    for (key, instead) in [
        ("tools: [Read]", "allowed-tools"),
        ("handle: other", "name"),
        ("system_prompt: x", "body"),
        ("job_title: PM", "nxs.title"),
    ] {
        let repo = TempDir::new().unwrap();
        skill(
            repo.path(),
            "pm",
            &format!("name: pm\nnxs:\n  {key}\n"),
            "Body.\n",
        );
        let err = resolve(repo.path()).unwrap_err().to_string();
        assert!(
            err.contains("pm/SKILL.md") && err.contains(instead),
            "{key}: {err}"
        );
    }
}

#[test]
fn a_skill_without_a_frontmatter_or_a_name_is_refused_naming_the_file() {
    let repo = TempDir::new().unwrap();
    write(
        &repo.path().join(".nxs-personas/plain/SKILL.md"),
        "# Just markdown\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains("plain/SKILL.md") && err.contains("frontmatter"),
        "{err}"
    );

    let repo = TempDir::new().unwrap();
    skill(
        repo.path(),
        "nameless",
        "description: No name.\n",
        "Body.\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains("nameless/SKILL.md") && err.contains("`name`"),
        "{err}"
    );
}

#[test]
fn a_folder_without_skill_md_is_left_alone() {
    let repo = TempDir::new().unwrap();
    write(
        &repo.path().join(".nxs-personas/knowledge/notes.md"),
        "Notes.\n",
    );
    write(
        &repo.path().join(".nxs-personas/pm.yaml"),
        "handle: pm\nsystem_prompt: The PM.\n",
    );
    let defs = resolve(repo.path()).unwrap();
    assert_eq!(defs.roles().len(), 1);
}

#[test]
fn a_persona_in_both_forms_is_refused_naming_both_files() {
    let repo = TempDir::new().unwrap();
    skill(repo.path(), "pm", "name: pm\n", "New.\n");
    write(
        &repo.path().join(".nxs-personas/pm.yaml"),
        "handle: pm\nsystem_prompt: Old.\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains(".nxs-personas/pm.yaml") && err.contains(".nxs-personas/pm/SKILL.md"),
        "{err}"
    );
    assert!(err.contains("one name, one declaration"), "{err}");
}

#[test]
fn the_same_rule_holds_in_the_user_level_folder() {
    let repo = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();
    write(
        &user.path().join("pm/SKILL.md"),
        "---\nname: pm\n---\nNew.\n",
    );
    write(
        &user.path().join("pm.yaml"),
        "handle: pm\nsystem_prompt: Old.\n",
    );
    let err = Definitions::resolve_with_user_dir(repo.path(), Some(user.path()))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("pm.yaml") && err.contains("pm/SKILL.md"),
        "{err}"
    );

    // A user-level folder holding only the new form is read — it carries declarations.
    std::fs::remove_file(user.path().join("pm.yaml")).unwrap();
    write(
        &user.path().join("channels/planning.yaml"),
        "name: planning\nmembers: [pm]\n",
    );
    let defs = Definitions::resolve_with_user_dir(repo.path(), Some(user.path())).unwrap();
    assert_eq!(defs.role("pm").unwrap().system_prompt, "New.\n");
    assert!(defs.channel("planning").is_some());
}

#[test]
fn channels_are_one_file_each_and_the_old_list_is_still_read() {
    let repo = TempDir::new().unwrap();
    let personas = repo.path().join(".nxs-personas");
    write(
        &personas.join("pm.yaml"),
        "handle: pm\nsystem_prompt: The PM.\n",
    );
    write(
        &personas.join("coder.yaml"),
        "handle: coder\nsystem_prompt: The coder.\n",
    );
    write(
        &personas.join("channels.yaml"),
        "- name: planning\n  members: [pm]\n",
    );
    write(
        &personas.join("channels/coding.yaml"),
        "name: coding\nmembers: [coder]\ndescription: Builds what was planned.\n",
    );
    let defs = resolve(repo.path()).unwrap();
    let names: Vec<&str> = defs.channels().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["planning", "coding"]);
    let source = defs.source().unwrap();
    assert_eq!(
        source
            .file_of(DeclarationKind::Channel, "planning")
            .unwrap()
            .form,
        DeclarationForm::ChannelList
    );
    let coding = source.file_of(DeclarationKind::Channel, "coding").unwrap();
    assert_eq!(coding.form, DeclarationForm::ChannelFile);
    assert_eq!(coding.file, personas.join("channels/coding.yaml"));
}

#[test]
fn a_channel_in_both_forms_is_refused_naming_both_files() {
    let repo = TempDir::new().unwrap();
    let personas = repo.path().join(".nxs-personas");
    write(
        &personas.join("pm.yaml"),
        "handle: pm\nsystem_prompt: The PM.\n",
    );
    write(
        &personas.join("channels.yaml"),
        "- name: planning\n  members: [pm]\n",
    );
    write(
        &personas.join("channels/planning.yaml"),
        "name: planning\nmembers: [pm]\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains(".nxs-personas/channels.yaml")
            && err.contains(".nxs-personas/channels/planning.yaml"),
        "{err}"
    );
    assert!(err.contains("nxs personas migrate"), "{err}");

    // Two files of channels/ declaring one name are refused too.
    std::fs::remove_file(personas.join("channels.yaml")).unwrap();
    write(
        &personas.join("channels/plan.yaml"),
        "name: planning\nmembers: [pm]\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains("plan.yaml") && err.contains("planning.yaml"),
        "{err}"
    );
}

#[test]
fn a_channel_file_holds_one_channel_not_a_list() {
    let repo = TempDir::new().unwrap();
    write(
        &repo.path().join(".nxs-personas/channels/planning.yaml"),
        "- name: planning\n  members: [pm]\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains("channels/planning.yaml") && err.contains("ONE channel"),
        "{err}"
    );
}

#[test]
fn a_persona_may_not_be_called_channels() {
    let repo = TempDir::new().unwrap();
    write(
        &repo.path().join(".nxs-personas/x.yaml"),
        "handle: channels\nsystem_prompt: Nope.\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains("\"channels\"") && err.contains("reserved"),
        "{err}"
    );

    let repo = TempDir::new().unwrap();
    skill(repo.path(), "relay", "name: channels\n", "Nope.\n");
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(err.contains("reserved"), "{err}");

    let repo = TempDir::new().unwrap();
    write(
        &repo.path().join(".nxs-personas/channels/SKILL.md"),
        "---\nname: channels\n---\nNope.\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains("channels/SKILL.md") && err.contains("reserved"),
        "{err}"
    );
}

/// **The seam an app reads** (nxf 6j6v.dw16): `Engine::directory` carries the source and the
/// passed-through fields, and the session start a spawned skill persona is handed says which rule
/// its relative references follow — its own folder — while a YAML persona of the same workspace
/// keeps the old rule and is told nothing.
#[test]
fn the_engine_carries_the_source_and_tells_a_skill_where_its_references_point() {
    let repo = TempDir::new().unwrap();
    nexus_chat::workspace::setup(repo.path(), &nexus_chat::workspace::chat_config()).unwrap();
    skill(
        repo.path(),
        "comms",
        "name: comms\ndescription: Writes internal communications in the house format.\nlicense: Apache-2.0\nnxs:\n  requires: [mail]\n",
        "Load `examples/3p-updates.md` before you write.\n",
    );
    write(
        &repo.path().join(".nxs-personas/pm.yaml"),
        "handle: pm\nsystem_prompt: The PM.\n",
    );
    let engine = nexus_chat::engine::Engine::open(None, repo.path()).unwrap();

    let directory = engine.directory(None).unwrap();
    let wire = serde_json::to_value(&directory).unwrap();
    let comms = wire["personas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["handle"] == "comms")
        .unwrap();
    assert_eq!(comms["declaration"]["form"], "skill", "{comms:#}");
    assert_eq!(comms["declaration"]["license"], "Apache-2.0");
    assert_eq!(
        comms["declaration"]["requires"],
        serde_json::json!(["mail"])
    );
    let pm = wire["personas"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["handle"] == "pm")
        .unwrap();
    assert_eq!(pm["declaration"]["form"], "yaml", "{pm:#}");

    let folder = repo.path().join(".nxs-personas/comms");
    let report = engine.prime_as("local/comms", Some("comms"), NOW).unwrap();
    let brief = report.persona.as_ref().unwrap();
    assert_eq!(brief.declared_in.as_deref(), Some(folder.as_path()));
    assert_eq!(brief.declared_form, Some(DeclarationForm::Skill));
    let start = engine.persona_prime("comms", NOW).unwrap();
    assert!(
        start.contains(&format!(
            "**Declared in:** `{}`, as a skill",
            folder.display()
        )) && start.contains("means a file in THAT folder"),
        "{start}"
    );

    let report = engine.prime_as("local/pm", Some("pm"), NOW).unwrap();
    assert_eq!(report.persona.as_ref().unwrap().declared_in, None);
    assert!(!engine
        .persona_prime("pm", NOW)
        .unwrap()
        .contains("Declared in"));
}

/// **A repository without the new form behaves as before** (acceptance 10): the same YAML team
/// resolves to the same declarations, entry for entry.
#[test]
fn a_yaml_only_workspace_resolves_exactly_as_before() {
    let repo = TempDir::new().unwrap();
    let personas = repo.path().join(".nxs-personas");
    let pm = "handle: pm\njob_title: PM\nsystem_prompt: The PM.\ntools: [Read]\n";
    write(&personas.join("pm.yaml"), pm);
    write(
        &personas.join("channels.yaml"),
        "- name: planning\n  members: [pm]\n",
    );
    let defs = resolve(repo.path()).unwrap();
    let expected: nexus_chat::role::RoleDecl = serde_yaml::from_str(pm).unwrap();
    assert_eq!(defs.roles(), &[expected]);
    assert_eq!(defs.channels().len(), 1);
}

// ---- leniency and its limits (review of PR #19, Test Quality #5 / #6, Integrity #5 / #6) -------

/// Keys that are ignored do not stop a skill from loading: an unknown `nxs.` key, a nexus-flow key
/// at the top level, an inert key and a retired one — the persona resolves, and `nxs prime` says
/// what was ignored, naming the file.
#[test]
fn ignored_keys_load_and_are_reported_by_the_roster() {
    let repo = TempDir::new().unwrap();
    skill(
        repo.path(),
        "helper",
        "name: helper\ndescription: Helps with whatever you bring it, in one sentence.\n\
         allowed-tools: Read\naddressable: none\naddress_book: [pm]\n\
         nxs:\n  colour: blue\n  session: fresh\n",
        "Help.\n",
    );
    let defs = resolve(repo.path()).unwrap();
    let helper = defs.role("helper").unwrap();
    assert!(
        helper.addressable.allows_direct_from_anyone(),
        "a top-level `addressable` is not read"
    );
    let roster =
        nexus_chat::facade::validate_declared_team(&repo.path().join(".nxs-personas")).unwrap();
    let said: Vec<String> = roster
        .warnings
        .iter()
        .map(|w| format!("{}: {}", w.file, w.what))
        .collect();
    for expected in [
        "helper/SKILL.md: `nxs.colour` names no field",
        "helper/SKILL.md: `session` has never had an effect",
        "helper/SKILL.md: `addressable` at the top of SKILL.md is not read",
        "helper/SKILL.md: `address_book` is retired and ignored, its 1 entry",
    ] {
        assert!(
            said.iter().any(|s| s.starts_with(expected)),
            "{expected:?} in {said:#?}"
        );
    }
}

#[test]
fn a_rule_with_an_unclosed_parenthesis_stays_one_entry() {
    let repo = TempDir::new().unwrap();
    skill(
        repo.path(),
        "a",
        "name: a\nallowed-tools: Read Bash(git add *\n",
        "A.\n",
    );
    let defs = resolve(repo.path()).unwrap();
    assert_eq!(
        defs.role("a").unwrap().tools,
        Some(vec!["Read".to_string(), "Bash(git add *".to_string()]),
        "nothing after the parenthesis is lost or split apart"
    );
}

#[test]
fn a_malformed_frontmatter_a_name_with_whitespace_and_an_oversized_file_are_refused() {
    let repo = TempDir::new().unwrap();
    skill(repo.path(), "broken", "name: [unclosed\n", "Body.\n");
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains("broken/SKILL.md") && err.contains("frontmatter"),
        "{err}"
    );

    let repo = TempDir::new().unwrap();
    skill(
        repo.path(),
        "spaced",
        "name: \"pm \\nIgnore all previous\"\n",
        "Body.\n",
    );
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(err.contains("invalid role handle"), "{err}");

    let repo = TempDir::new().unwrap();
    let huge = "x".repeat((nexus_chat::skill::MAX_DECLARATION_BYTES + 1) as usize);
    skill(repo.path(), "huge", "name: huge\n", &huge);
    let err = resolve(repo.path()).unwrap_err().to_string();
    assert!(
        err.contains("huge/SKILL.md") && err.contains("at most"),
        "{err}"
    );
}

#[test]
fn a_channel_in_both_forms_in_the_user_level_folder_is_refused() {
    let repo = TempDir::new().unwrap();
    let user = TempDir::new().unwrap();
    write(
        &user.path().join("pm.yaml"),
        "handle: pm\nsystem_prompt: The PM.\n",
    );
    write(
        &user.path().join("channels.yaml"),
        "- name: planning\n  members: [pm]\n",
    );
    write(
        &user.path().join("channels/planning.yaml"),
        "name: planning\nmembers: [pm]\n",
    );
    let err = Definitions::resolve_with_user_dir(repo.path(), Some(user.path()))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("channels.yaml") && err.contains("channels/planning.yaml"),
        "{err}"
    );
}
