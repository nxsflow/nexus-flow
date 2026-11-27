//! Installing the shipped defaults into a repository's `factory/` (RFC 0006, "The factory's
//! files"; "Installing the shipped defaults"): a missing file is written, an earlier default is
//! replaced only with `--update`, a team's file and a retired path are never touched, nothing is
//! deleted, no path the list does not name is read or written, and CODEOWNERS is reported.
//!
//! These tests give the install a set of defaults of their own, so that earlier and retired
//! versions exist; `engine_factory_install.rs` runs the defaults `nxs` embeds through the engine.

mod common;

use std::fs;
use std::path::Path;

use common::scratch;
use nexus_flow_seam::install::factory::{install, Defaults, FileReport, Report};
use nexus_flow_seam::install::shipped::{sha256_lf, Shipped, State};

const CODER_V1: &str = "# Coder\n\nVersion one.\n";
const CODER_V2: &str = "# Coder\n\nVersion two.\n";
const PM: &str = "# pm\n";
const FACTORY: &str = "# Factory\n";
const OLD: &str = "# Old role\n";

/// Defaults with `roles/coder.md` at two versions, `roles/pm.md` and `FACTORY.md` at one, and
/// `roles/old.md` retired.
fn defaults() -> Defaults {
    let toml = format!(
        r#"
[[file]]
path = "FACTORY.md"
sha256 = ["{factory}"]

[[file]]
path = "roles/coder.md"
sha256 = ["{v1}", "{v2}"]

[[file]]
path = "roles/old.md"
sha256 = ["{old}"]
retired = true

[[file]]
path = "roles/pm.md"
sha256 = ["{pm}"]
"#,
        factory = sha256_lf(FACTORY.as_bytes()),
        v1 = sha256_lf(CODER_V1.as_bytes()),
        v2 = sha256_lf(CODER_V2.as_bytes()),
        old = sha256_lf(OLD.as_bytes()),
        pm = sha256_lf(PM.as_bytes()),
    );
    let shipped = Shipped::parse(&toml).unwrap();
    Defaults::new(
        shipped,
        [
            ("FACTORY.md", FACTORY.as_bytes().to_vec()),
            ("roles/coder.md", CODER_V2.as_bytes().to_vec()),
            ("roles/pm.md", PM.as_bytes().to_vec()),
        ],
    )
    .unwrap()
}

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap()
}

fn file(path: &str, state: State) -> FileReport {
    FileReport {
        path: path.to_string(),
        state,
    }
}

/// Every file under `dir`, relative to it with `/`, in bytewise order.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path.strip_prefix(base).unwrap();
                out.push(rel.to_str().unwrap().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    if dir.exists() {
        walk(dir, dir, &mut out);
    }
    out.sort();
    out
}

// --- the states and what is written -------------------------------------------------------------

#[test]
fn an_empty_repository_gets_every_current_default_written() {
    let root = scratch("factory-empty").join("repo");

    let report = install(&root, &defaults(), false).unwrap();

    assert_eq!(
        report,
        Report {
            files: vec![
                file("factory/FACTORY.md", State::Written),
                file("factory/roles/coder.md", State::Written),
                file("factory/roles/pm.md", State::Written),
            ],
            codeowners: None,
        }
    );
    assert_eq!(read(&root, "factory/FACTORY.md"), FACTORY);
    assert_eq!(read(&root, "factory/roles/coder.md"), CODER_V2);
    assert_eq!(read(&root, "factory/roles/pm.md"), PM);
}

#[test]
fn a_second_install_finds_every_file_current_and_writes_nothing() {
    let root = scratch("factory-again").join("repo");
    install(&root, &defaults(), false).unwrap();

    let report = install(&root, &defaults(), true).unwrap();

    let states: Vec<State> = report.files.iter().map(|f| f.state).collect();
    assert_eq!(states, vec![State::Current; 3]);
}

#[test]
fn a_default_checked_out_with_crlf_is_current_and_left_as_it_is() {
    let root = scratch("factory-crlf").join("repo");
    let crlf = CODER_V2.replace('\n', "\r\n");
    write(&root, "factory/roles/coder.md", &crlf);

    let report = install(&root, &defaults(), true).unwrap();

    assert!(report
        .files
        .contains(&file("factory/roles/coder.md", State::Current)));
    assert_eq!(read(&root, "factory/roles/coder.md"), crlf);
}

#[test]
fn an_earlier_default_is_reported_and_kept_without_update() {
    let root = scratch("factory-older").join("repo");
    write(&root, "factory/roles/coder.md", CODER_V1);

    let report = install(&root, &defaults(), false).unwrap();

    assert!(report
        .files
        .contains(&file("factory/roles/coder.md", State::OlderDefault)));
    assert_eq!(read(&root, "factory/roles/coder.md"), CODER_V1);
}

#[test]
fn update_replaces_an_earlier_default_with_the_current_one() {
    let root = scratch("factory-updated").join("repo");
    write(&root, "factory/roles/coder.md", CODER_V1);

    let report = install(&root, &defaults(), true).unwrap();

    assert!(report
        .files
        .contains(&file("factory/roles/coder.md", State::Updated)));
    assert_eq!(read(&root, "factory/roles/coder.md"), CODER_V2);
}

#[test]
fn a_team_changed_role_file_stays_byte_identical_with_and_without_update() {
    let root = scratch("factory-team").join("repo");
    let team = "# Coder\r\n\r\nOur own way, with a trailing space. \n";
    write(&root, "factory/roles/coder.md", team);
    let before = fs::read(root.join("factory/roles/coder.md")).unwrap();

    for update in [false, true] {
        let report = install(&root, &defaults(), update).unwrap();
        assert!(report
            .files
            .contains(&file("factory/roles/coder.md", State::Team)));
        assert_eq!(
            fs::read(root.join("factory/roles/coder.md")).unwrap(),
            before
        );
    }
}

#[test]
fn a_present_retired_file_is_reported_and_never_touched() {
    let root = scratch("factory-retired").join("repo");
    write(&root, "factory/roles/old.md", OLD);

    let report = install(&root, &defaults(), true).unwrap();

    assert!(report
        .files
        .contains(&file("factory/roles/old.md", State::Retired)));
    assert_eq!(read(&root, "factory/roles/old.md"), OLD);
}

/// NEX-235 asks the owner whether the report lists a retired path whose file is missing; until
/// the answer comes, the safe subset leaves it out, and nothing is written there either way.
#[test]
fn a_missing_retired_file_is_neither_written_nor_listed() {
    let root = scratch("factory-retired-missing").join("repo");

    let report = install(&root, &defaults(), true).unwrap();

    assert!(report
        .files
        .iter()
        .all(|f| f.path != "factory/roles/old.md"));
    assert!(!root.join("factory/roles/old.md").exists());
}

#[test]
fn files_are_ordered_by_path_bytewise() {
    let root = scratch("factory-order").join("repo");
    write(&root, "factory/roles/coder.md", "team\n");

    let report = install(&root, &defaults(), false).unwrap();

    let paths: Vec<&str> = report.files.iter().map(|f| f.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
    assert_eq!(paths, sorted);
    assert_eq!(paths[0], "factory/FACTORY.md");
}

// --- what install never does ---------------------------------------------------------------------

#[test]
fn a_teams_own_role_is_never_touched_or_listed() {
    let root = scratch("factory-legal").join("repo");
    let legal = "# Legal\n\nOur own role.\n";
    write(&root, "factory/roles/legal.md", legal);

    for update in [false, true] {
        let report = install(&root, &defaults(), update).unwrap();
        assert!(report.files.iter().all(|f| !f.path.contains("legal")));
        assert_eq!(read(&root, "factory/roles/legal.md"), legal);
    }
}

#[test]
fn only_paths_the_list_names_are_written_and_nothing_is_deleted() {
    let root = scratch("factory-only-listed").join("repo");
    write(&root, "factory/roles/legal.md", "legal\n");
    write(&root, "factory/notes/today.md", "notes\n");
    write(&root, "README.md", "readme\n");

    install(&root, &defaults(), true).unwrap();

    assert_eq!(
        tree(&root.join("factory")),
        vec![
            "FACTORY.md",
            "notes/today.md",
            "roles/coder.md",
            "roles/legal.md",
            "roles/pm.md",
        ]
    );
    assert_eq!(read(&root, "README.md"), "readme\n");
}

/// A file that cannot be read fails the read; each of these is a link to itself, so install
/// succeeds only if it never reads them.
#[cfg(unix)]
#[test]
fn neither_genesis_nor_the_genesis_guide_is_read_or_written() {
    let root = scratch("factory-genesis").join("repo");
    fs::create_dir_all(root.join("factory")).unwrap();
    for name in ["GENESIS.md", "genesis-guide.md"] {
        std::os::unix::fs::symlink(name, root.join("factory").join(name)).unwrap();
    }

    for update in [false, true] {
        let report = install(&root, &defaults(), update).unwrap();
        assert!(report.files.iter().all(|f| !f.path.contains("enesis")));
    }
    for name in ["GENESIS.md", "genesis-guide.md"] {
        let link = fs::read_link(root.join("factory").join(name)).unwrap();
        assert_eq!(link, Path::new(name));
    }
}

#[test]
fn nothing_is_written_when_a_listed_file_cannot_be_read() {
    let root = scratch("factory-unreadable").join("repo");
    // A folder where the role file should be: it cannot be read as a file.
    fs::create_dir_all(root.join("factory/roles/pm.md")).unwrap();

    assert!(install(&root, &defaults(), false).is_err());
    assert!(!root.join("factory/FACTORY.md").exists());
    assert!(!root.join("factory/roles/coder.md").exists());
}

// --- CODEOWNERS ----------------------------------------------------------------------------------

#[test]
fn codeowners_lists_the_reported_paths_no_rule_covers_in_their_order() {
    let root = scratch("factory-codeowners").join("repo");
    write(
        &root,
        ".github/CODEOWNERS",
        "/factory/FACTORY.md @owner\n/factory/roles/legal.md @owner\n",
    );
    write(&root, "factory/roles/legal.md", "legal\n");

    let report = install(&root, &defaults(), false).unwrap();

    let codeowners = report.codeowners.unwrap();
    assert_eq!(codeowners.file, ".github/CODEOWNERS");
    assert_eq!(
        codeowners.uncovered,
        vec!["factory/roles/coder.md", "factory/roles/pm.md"]
    );
}

#[test]
fn codeowners_is_never_written() {
    let root = scratch("factory-codeowners-kept").join("repo");
    write(&root, "CODEOWNERS", "* @owner\n");

    let report = install(&root, &defaults(), true).unwrap();

    let codeowners = report.codeowners.unwrap();
    assert_eq!(codeowners.file, "CODEOWNERS");
    assert!(codeowners.uncovered.is_empty());
    assert_eq!(read(&root, "CODEOWNERS"), "* @owner\n");
    assert!(!root.join(".github").exists());
}

// --- the defaults --------------------------------------------------------------------------------

#[test]
fn defaults_refuse_a_text_that_is_not_the_current_default() {
    let shipped = Shipped::parse(&format!(
        "[[file]]\npath = \"roles/coder.md\"\nsha256 = [\"{}\", \"{}\"]\n",
        sha256_lf(CODER_V1.as_bytes()),
        sha256_lf(CODER_V2.as_bytes()),
    ))
    .unwrap();

    let stale = Defaults::new(
        shipped.clone(),
        [("roles/coder.md", CODER_V1.as_bytes().to_vec())],
    );
    assert!(stale.is_err());
    let missing = Defaults::new(shipped, Vec::<(&str, Vec<u8>)>::new());
    assert!(missing.is_err());
}

#[test]
fn defaults_refuse_a_text_for_a_path_the_list_does_not_name_or_retired() {
    let shipped = Shipped::parse(&format!(
        "[[file]]\npath = \"roles/old.md\"\nsha256 = [\"{}\"]\nretired = true\n",
        sha256_lf(OLD.as_bytes()),
    ))
    .unwrap();

    let retired = Defaults::new(shipped.clone(), [("roles/old.md", OLD.as_bytes().to_vec())]);
    assert!(retired.is_err());
    let unnamed = Defaults::new(shipped, [("roles/new.md", b"new\n".to_vec())]);
    assert!(unnamed.is_err());
}
