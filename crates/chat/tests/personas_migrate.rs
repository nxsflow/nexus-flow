//! **`nxs personas migrate`** (nxf 6j6v.h9ee) — the older form rewritten into the new one, with the
//! comments kept, the retired and inert keys dropped and reported, nothing guessed, and a second
//! run that finds nothing to do. The library half; the verb is tested through the real binary in
//! `crates/nxs/tests/personas_migrate.rs`.

use std::path::Path;

use nexus_chat::definitions::{DeclarationForm, DeclarationKind, Definitions};
use nexus_chat::persona_migration::migrate_declarations;
use nexus_chat::role::RoleDecl;
use tempfile::TempDir;

const PM: &str = "\
# The PM of this project. Kept short on purpose.

handle: pm
job_title: Product Manager
job_description: >-
  Turns a product idea into board items. Write to me directly for small work.
expected_output: One message on your thread.
stage: senior
# Owner, 2026-09-20: the PM may run the shell without asking.
permissions: bypassPermissions
addressable:
  humans: true   # the owner talks to the PM directly
tools: [Bash, Read, 'Bash(nxf create:*)']
prime:
  chat: true
  memory: false
address_book:
  - to: coder
    why: hand over a work order
  - to: planning
    why: get a plan judged
session: continue
license: Apache-2.0
metadata:
  author: example-org
system_prompt: |
  You are the PM of this project.

  ## How you work
  Read the board first.
";

const CODER: &str = "\
handle: coder
job_description: Builds what the PM planned, test first, in a sentence.
working_tree: exclusive
addressable: none
address_book: []
reports_to: pm
system_prompt: You are the coder.
";

const CHANNELS: &str = "\
# The channels this project runs on.
# A work item walks planning -> coding.

# Planning: the PM designs, the reviewers judge.
- name: planning
  members: [pm]   # the PM seats itself
  steps:
    # The conversation lives HERE.
    - id: design
      target: pm

      # Lifted: the design is the dearest thinking.
      stage: principal
- name: coding
  description: Builds what was planned.
  members: [coder]
";

fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn team() -> TempDir {
    let repo = TempDir::new().unwrap();
    let dir = repo.path().join(".nxs-personas");
    write(&dir.join("pm.yaml"), PM);
    write(&dir.join("coder.yaml"), CODER);
    write(&dir.join("channels.yaml"), CHANNELS);
    write(&dir.join("knowledge/notes.md"), "Notes.\n");
    repo
}

fn without_dropped(mut decl: RoleDecl) -> RoleDecl {
    decl.session = Default::default();
    decl.sub_agents = false;
    decl.reports_to = None;
    decl
}

fn snapshot(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for entry in walk(dir) {
        let text = std::fs::read_to_string(&entry).unwrap_or_default();
        files.push((entry.strip_prefix(dir).unwrap().display().to_string(), text));
    }
    files.sort();
    files
}

fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        match path.is_dir() {
            true => out.extend(walk(&path)),
            false => out.push(path),
        }
    }
    out
}

#[test]
fn the_older_form_is_rewritten_with_its_comments_and_resolves_to_the_same_team() {
    let repo = team();
    let dir = repo.path().join(".nxs-personas");
    let before = Definitions::resolve_with_user_dir(repo.path(), None).unwrap();

    // The plan touches nothing.
    let untouched = snapshot(&dir);
    let plan = migrate_declarations(&dir, false).unwrap();
    assert!(!plan.applied);
    assert_eq!(snapshot(&dir), untouched, "the plan wrote something");
    assert_eq!(plan.personas.len(), 2);
    assert_eq!(plan.channels.len(), 2);

    let report = migrate_declarations(&dir, true).unwrap();
    assert!(report.applied);
    let pm = report.personas.iter().find(|p| p.name == "pm").unwrap();
    assert_eq!(pm.to, dir.join("pm/SKILL.md"));
    assert_eq!(
        pm.dropped,
        vec![
            "address_book (2 entries)".to_string(),
            "session".to_string()
        ]
    );
    let coder = report.personas.iter().find(|p| p.name == "coder").unwrap();
    assert_eq!(
        coder.dropped,
        vec!["address_book ([])".to_string(), "reports_to".to_string()]
    );
    assert_eq!(report.dropped_address_books(), 2);

    // The old files are gone, the new ones are where the loader reads them.
    assert!(!dir.join("pm.yaml").exists() && !dir.join("channels.yaml").exists());
    assert!(
        dir.join("knowledge/notes.md").is_file(),
        "a non-declaration is left alone"
    );
    let skill = std::fs::read_to_string(dir.join("pm/SKILL.md")).unwrap();
    assert!(
        skill.starts_with("---\n# The PM of this project."),
        "{skill}"
    );
    for kept in [
        "# Owner, 2026-09-20: the PM may run the shell without asking.",
        "humans: true   # the owner talks to the PM directly",
        "name: pm",
        "description: >-",
        "allowed-tools: [Bash, Read, 'Bash(nxf create:*)']",
        "license: Apache-2.0",
        "nxs:\n  title: Product Manager",
    ] {
        assert!(skill.contains(kept), "{kept:?} is missing from:\n{skill}");
    }
    assert!(
        !skill.contains("address_book") && !skill.contains("session:"),
        "{skill}"
    );
    assert!(
        skill.ends_with(
            "---\nYou are the PM of this project.\n\n## How you work\nRead the board first.\n"
        ),
        "the prompt is the body, as written:\n{skill}"
    );
    let planning = std::fs::read_to_string(dir.join("channels/planning.yaml")).unwrap();
    assert!(
        planning.starts_with("# Planning: the PM designs, the reviewers judge.\nname: planning\n"),
        "{planning}"
    );
    assert!(
        planning.contains("\n  # The conversation lives HERE.\n"),
        "{planning}"
    );
    let readme = std::fs::read_to_string(dir.join("channels/README.md")).unwrap();
    assert!(
        readme
            .contains("The channels this project runs on.\nA work item walks planning -> coding."),
        "{readme}"
    );
    assert_eq!(report.header_moved_to, Some(dir.join("channels/README.md")));

    // The same team, field for field — the dropped keys aside.
    let after = Definitions::resolve_with_user_dir(repo.path(), None).unwrap();
    let expected: Vec<RoleDecl> = before
        .roles()
        .iter()
        .cloned()
        .map(without_dropped)
        .collect();
    assert_eq!(after.roles(), expected.as_slice());
    // One file per channel is read in file-name order, so only the order of the list changes.
    let by_name = |defs: &Definitions| {
        let mut channels = defs.channels().to_vec();
        channels.sort_by(|a, b| a.name.cmp(&b.name));
        channels
    };
    assert_eq!(by_name(&after), by_name(&before));
    let source = after.source().unwrap();
    let file = source.file_of(DeclarationKind::Persona, "pm").unwrap();
    assert_eq!(file.form, DeclarationForm::Skill);
    assert_eq!(file.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(
        file.metadata,
        Some(serde_json::json!({"author": "example-org"}))
    );
    assert_eq!(
        source
            .file_of(DeclarationKind::Channel, "coding")
            .unwrap()
            .form,
        DeclarationForm::ChannelFile
    );

    // Idempotent.
    let again = migrate_declarations(&dir, true).unwrap();
    assert!(again.nothing_to_do(), "{again:?}");
}

#[test]
fn a_key_the_mapping_does_not_know_stops_the_run_before_anything_is_written() {
    let repo = team();
    let dir = repo.path().join(".nxs-personas");
    write(
        &dir.join("zz-odd.yaml"),
        "handle: zz-odd\nsystem_prompt: Odd.\nfavourite_colour: blue\n",
    );
    let before = snapshot(&dir);
    let err = migrate_declarations(&dir, true).unwrap_err().to_string();
    assert!(
        err.contains("zz-odd.yaml") && err.contains("`favourite_colour`"),
        "{err}"
    );
    assert_eq!(
        snapshot(&dir),
        before,
        "a file was written before the refusal"
    );
}

#[test]
fn an_existing_target_stops_the_run_before_anything_is_written() {
    let repo = team();
    let dir = repo.path().join(".nxs-personas");
    write(
        &dir.join("channels/coding.yaml"),
        "name: other\nmembers: [coder]\n",
    );
    let before = snapshot(&dir);
    let err = migrate_declarations(&dir, true).unwrap_err().to_string();
    assert!(
        err.contains("channels/coding.yaml") && err.contains("exists already"),
        "{err}"
    );
    assert_eq!(snapshot(&dir), before);
}

#[test]
fn a_missing_folder_or_a_migrated_one_has_nothing_to_do() {
    let repo = TempDir::new().unwrap();
    let report = migrate_declarations(&repo.path().join(".nxs-personas"), true).unwrap();
    assert!(report.nothing_to_do());
}
