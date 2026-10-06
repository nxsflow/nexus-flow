//! **One definition, many repositories** (nxf 6j6v.7k58) — the user-level declaration folder beside
//! a workspace's own, merged per name.
//!
//! Everything here resolves against a NAMED user-level folder
//! ([`Definitions::resolve_with_user_dir`]), so no test reads the machine's real one. The seam an
//! embedding app reads — `Engine::definitions`, `Engine::directory`, `Engine::prime_as` — resolves
//! the folder from `$HOME` and is proved in its own test binary,
//! `tests/one_definition_at_the_engine_seam.rs`, because pinning `$HOME` is process-wide.

use nexus_chat::definitions::{
    DeclarationKind, DeclarationOrigin, DeclarationSourceKind, DeclaredName, Definitions,
};
use std::path::Path;
use tempfile::TempDir;

const CENTRAL_PM: &str = "handle: pm\njob_title: Product manager\nsystem_prompt: The central PM.\n";
const REPO_PM: &str = "handle: pm\nsystem_prompt: The repository's own PM.\n";
const CODER: &str = "handle: coder\nsystem_prompt: You are the coder.\n";
const PLANNING: &str = "- name: planning\n  members: [pm, coder]\n";

fn declare(dir: &Path, file: &str, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), body).unwrap();
}

struct Machine {
    repo: TempDir,
    user: TempDir,
}

impl Machine {
    fn new() -> Machine {
        Machine {
            repo: TempDir::new().unwrap(),
            user: TempDir::new().unwrap(),
        }
    }
    fn repo_personas(&self) -> std::path::PathBuf {
        self.repo.path().join(".nxs-personas")
    }
    fn resolve(&self) -> nexus_chat::error::Result<Definitions> {
        Definitions::resolve_with_user_dir(self.repo.path(), Some(self.user.path()))
    }
}

fn persona(name: &str) -> DeclaredName {
    DeclaredName {
        kind: DeclarationKind::Persona,
        name: name.to_string(),
    }
}

fn channel(name: &str) -> DeclaredName {
    DeclaredName {
        kind: DeclarationKind::Channel,
        name: name.to_string(),
    }
}

#[test]
fn a_repository_without_a_file_of_its_own_runs_the_user_level_persona_and_its_channel() {
    let m = Machine::new();
    declare(m.user.path(), "pm.yaml", CENTRAL_PM);
    declare(m.user.path(), "channels.yaml", PLANNING);
    declare(&m.repo_personas(), "coder.yaml", CODER);

    let defs = m.resolve().unwrap();
    assert_eq!(defs.role("pm").unwrap().system_prompt, "The central PM.");
    assert_eq!(
        defs.role("coder").unwrap().system_prompt,
        "You are the coder."
    );
    // The channel arrives with the persona it presupposes, and its members resolve against the
    // MERGED team — the repository's own coder included.
    let planning = defs
        .declared_channel("planning")
        .unwrap()
        .expect("declared");
    assert_eq!(planning.members, vec!["pm", "coder"]);
    // Roles stay handle-sorted, the loader's own contract.
    let handles: Vec<_> = defs.roles().iter().map(|r| r.handle.as_str()).collect();
    assert_eq!(handles, ["coder", "pm"]);

    let source = defs.source().unwrap();
    assert_eq!(source.kind, DeclarationSourceKind::Personas);
    assert_eq!(source.count, 3);
    assert_eq!(source.user_path.as_deref(), Some(m.user.path()));
    assert_eq!(source.from_user, vec![persona("pm"), channel("planning")]);
    assert!(source.shadowed.is_empty());
    assert_eq!(
        source.origin_of(DeclarationKind::Persona, "pm"),
        DeclarationOrigin::User
    );
    assert_eq!(
        source.origin_of(DeclarationKind::Persona, "coder"),
        DeclarationOrigin::Workspace
    );
    let said = source.explain();
    assert!(
        said.contains("(1 declared)"),
        "the workspace's OWN count: {said}"
    );
    assert!(
        said.contains("persona `pm`, channel `planning`") && said.contains("not versioned"),
        "{said}"
    );
    assert!(source.worth_saying());
}

#[test]
fn a_repository_that_declares_nothing_of_its_own_is_not_told_that_nobody_is_declared() {
    let m = Machine::new();
    declare(m.user.path(), "pm.yaml", CENTRAL_PM);
    let defs = m.resolve().unwrap();
    let source = defs.source().unwrap();
    assert_eq!(source.kind, DeclarationSourceKind::None);
    assert_eq!(source.count, 1);
    let said = source.explain();
    assert!(!said.contains("nobody is declared here yet"), "{said}");
    assert!(said.contains("declares nobody of its own"), "{said}");
    assert!(said.contains("persona `pm`"), "{said}");
}

#[test]
fn a_repository_file_hides_the_user_level_entry_of_the_same_name_and_says_so() {
    let m = Machine::new();
    declare(m.user.path(), "pm.yaml", CENTRAL_PM);
    declare(m.user.path(), "channels.yaml", PLANNING);
    declare(&m.repo_personas(), "pm.yaml", REPO_PM);
    declare(&m.repo_personas(), "coder.yaml", CODER);
    declare(
        &m.repo_personas(),
        "channels.yaml",
        "- name: planning\n  members: [pm]\n",
    );

    let defs = m.resolve().unwrap();
    assert_eq!(
        defs.role("pm").unwrap().system_prompt,
        "The repository's own PM."
    );
    assert_eq!(defs.channel("planning").unwrap().members, vec!["pm"]);
    assert_eq!(defs.roles().len(), 2, "hidden, not added twice");
    let source = defs.source().unwrap();
    assert!(source.from_user.is_empty());
    assert_eq!(source.shadowed, vec![persona("pm"), channel("planning")]);
    let said = source.explain();
    assert!(
        said.contains("HIDES the user-level persona `pm`, channel `planning`"),
        "{said}"
    );
    assert!(said.contains("adds nothing here"), "{said}");
}

/// Acceptance point 8: no user-level folder — or one that declares nothing — and a repository reads
/// exactly as it did before the folder existed, down to the sentence.
#[test]
fn without_a_user_level_declaration_the_resolution_is_the_one_from_before() {
    let m = Machine::new();
    declare(&m.repo_personas(), "pm.yaml", REPO_PM);
    let before = Definitions::resolve_with_user_dir(m.repo.path(), None).unwrap();
    // A folder that exists but carries nothing (only a README) is the same as no folder.
    declare(m.user.path(), "README.md", "# mine\n");
    let after = m.resolve().unwrap();
    assert_eq!(before, after);
    let source = after.source().unwrap();
    assert_eq!(source.user_path, None);
    assert!(!source.worth_saying());
    assert_eq!(
        source.explain(),
        format!("read from {} (1 declared).", m.repo_personas().display())
    );
    let wire = source.to_value();
    for key in ["user_path", "from_user", "shadowed"] {
        assert!(wire.get(key).is_none(), "{key} stays off the wire: {wire}");
    }
}

#[test]
fn two_user_level_files_declaring_one_handle_are_refused_naming_both_files() {
    let m = Machine::new();
    declare(m.user.path(), "pm.yaml", CENTRAL_PM);
    declare(m.user.path(), "pm-copy.yaml", CENTRAL_PM);
    let err = m.resolve().unwrap_err();
    assert!(
        err.msg
            .contains(&m.user.path().join("pm.yaml").display().to_string())
            && err
                .msg
                .contains(&m.user.path().join("pm-copy.yaml").display().to_string())
            && err.msg.contains("declared twice"),
        "{}",
        err.msg
    );
}

#[test]
fn a_malformed_user_level_file_fails_the_catalogue_with_its_path() {
    let m = Machine::new();
    declare(&m.repo_personas(), "coder.yaml", CODER);
    declare(m.user.path(), "pm.yaml", "handle: [not, a, handle]\n");
    let err = m.resolve().unwrap_err();
    assert!(
        err.msg
            .contains(&m.user.path().join("pm.yaml").display().to_string()),
        "{}",
        err.msg
    );
}

/// A cycle is judged on the team that RUNS: a repository channel hiding a user-level step relieves
/// the user-level flow of the cycle it would otherwise form.
#[test]
fn a_flow_is_judged_on_the_merged_team() {
    let m = Machine::new();
    declare(&m.repo_personas(), "coder.yaml", CODER);
    declare(
        m.user.path(),
        "channels.yaml",
        "- name: a\n  members: [b]\n- name: b\n  members: [a]\n",
    );
    let err = m.resolve().unwrap_err();
    assert!(err.msg.contains("reaches itself"), "{}", err.msg);
    declare(
        &m.repo_personas(),
        "channels.yaml",
        "- name: b\n  members: [coder]\n",
    );
    let defs = m.resolve().expect("the repository's b breaks the cycle");
    assert_eq!(defs.source().unwrap().shadowed, vec![channel("b")]);
}

#[test]
fn from_source_reads_the_same_merged_catalogue_again() {
    let m = Machine::new();
    declare(m.user.path(), "pm.yaml", CENTRAL_PM);
    declare(&m.repo_personas(), "coder.yaml", CODER);
    let defs = m.resolve().unwrap();
    let again = Definitions::from_source(defs.source().unwrap()).unwrap();
    assert_eq!(defs, again);
}

// ---- the user-level loader's failing cases (review of PR #17, Test Quality #2) ----------------

#[test]
fn a_malformed_user_level_channels_file_fails_the_catalogue_with_its_path() {
    let m = Machine::new();
    declare(
        m.user.path(),
        "channels.yaml",
        "- name: planning\n  members: pm\n",
    );
    let err = m.resolve().unwrap_err();
    assert!(
        err.msg
            .contains(&m.user.path().join("channels.yaml").display().to_string()),
        "{}",
        err.msg
    );
}

#[test]
fn two_user_level_channels_of_one_name_are_refused_naming_the_file() {
    let m = Machine::new();
    declare(
        m.user.path(),
        "channels.yaml",
        "- name: planning\n  members: [pm]\n- name: planning\n  members: [pm]\n",
    );
    declare(m.user.path(), "pm.yaml", CENTRAL_PM);
    let err = m.resolve().unwrap_err();
    assert!(
        err.msg
            .contains(&m.user.path().join("channels.yaml").display().to_string())
            && err.msg.contains("declared twice"),
        "{}",
        err.msg
    );
}

/// Referential integrity stays advisory for the merged team exactly as for one folder: a user-level
/// channel naming a persona nobody declares still loads (`prime` reports it; a send to it refuses).
#[test]
fn a_user_level_channel_naming_a_persona_nobody_declares_still_loads() {
    let m = Machine::new();
    declare(m.user.path(), "channels.yaml", PLANNING);
    let defs = m.resolve().expect("advisory, not a construction error");
    assert!(defs.declared_channel("planning").unwrap().is_some());
    assert!(defs.role("coder").is_err());
}

#[test]
fn a_user_level_folder_that_does_not_exist_is_no_user_level_folder() {
    let m = Machine::new();
    declare(&m.repo_personas(), "pm.yaml", REPO_PM);
    let missing = m.user.path().join("not-there");
    let with_missing = Definitions::resolve_with_user_dir(m.repo.path(), Some(&missing)).unwrap();
    let without = Definitions::resolve_with_user_dir(m.repo.path(), None).unwrap();
    assert_eq!(with_missing, without);
}

#[cfg(unix)]
#[test]
fn an_unreadable_user_level_folder_is_an_error_not_a_silence() {
    use std::os::unix::fs::PermissionsExt as _;
    let m = Machine::new();
    declare(m.user.path(), "pm.yaml", CENTRAL_PM);
    std::fs::set_permissions(m.user.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let result = m.resolve();
    std::fs::set_permissions(m.user.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    // A process that may read anything (root in a container) reads it fine; only a refused read
    // is the case under test.
    if std::fs::metadata("/root").is_ok_and(|_| std::fs::read_dir("/root").is_ok()) {
        return;
    }
    let err = result.unwrap_err();
    assert!(
        err.msg.contains("user-level declaration folder"),
        "{}",
        err.msg
    );
}

/// The read `threads show` and `search` resolve their channel policy through depends on the two
/// `channels.yaml` files only (review of PR #17, Code Quality #1): a malformed PERSONA file in the
/// user-level folder fails the catalogue, and must not make a thread unreadable.
#[test]
fn the_channel_policy_read_does_not_fail_on_a_malformed_persona_file() {
    let m = Machine::new();
    declare(m.user.path(), "channels.yaml", PLANNING);
    declare(m.user.path(), "pm.yaml", "handle: [not, a, handle]\n");
    assert!(m.resolve().is_err());
    let channels =
        nexus_chat::definitions::merged_channels_with_user_dir(m.repo.path(), Some(m.user.path()))
            .expect("channels only");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].name, "planning");
}
