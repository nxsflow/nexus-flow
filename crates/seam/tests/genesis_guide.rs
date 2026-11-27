//! `nxs genesis create` installs the guide, `factory/genesis-guide.md`, under RFC 0006's rules
//! for shipped defaults, and reports which of its paths CODEOWNERS leaves uncovered (RFC 0004,
//! "The guide", "Output"): a missing guide is written, an earlier shipped guide is replaced only
//! with `--update`, a guide the team changed or whose path is retired is never touched, and
//! `--update` never touches GENESIS. `codeowners` is `None` without a CODEOWNERS file, and names
//! GENESIS, never the guide, when no rule covers it.

use std::fs;
use std::path::{Path, PathBuf};

use nexus_flow_seam::genesis::{
    install_guide, FileReport, FileState, GenesisState, Report, GUIDE, GUIDE_PATH, PATH, SHIPPED,
    TEMPLATE,
};
use nexus_flow_seam::install::codeowners;
use nexus_flow_seam::install::shipped::{sha256_lf, Shipped, State};
use nexus_flow_seam::Engine;

/// An earlier guide, which the lists in these tests name before the current one.
const OLDER: &str = "# An earlier guide\n\nAs nexus-flow once shipped it.\n";

/// A fresh folder for one test, holding `repo/` with a `nexus-flow.toml`, outside git.
fn repository(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-seam-guide-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    fs::create_dir_all(repo.join("factory")).unwrap();
    fs::write(
        repo.join("nexus-flow.toml"),
        format!(
            "format = 1\n\n[space]\nrepository = \"{}\"\n",
            "0".repeat(32)
        ),
    )
    .unwrap();
    repo.canonicalize().unwrap()
}

fn cleanup(repo: &Path) {
    fs::remove_dir_all(repo.parent().unwrap()).unwrap();
}

/// A shipped list naming the guide at `OLDER`, then at the current `GUIDE`.
fn with_older() -> Shipped {
    Shipped::parse(&format!(
        "[[file]]\npath = \"genesis-guide.md\"\nsha256 = [\"{}\", \"{}\"]\n",
        sha256_lf(OLDER.as_bytes()),
        sha256_lf(GUIDE.as_bytes()),
    ))
    .unwrap()
}

/// A shipped list naming the guide's path as retired.
fn retired() -> Shipped {
    Shipped::parse(&format!(
        "[[file]]\npath = \"genesis-guide.md\"\nsha256 = [\"{}\"]\nretired = true\n",
        sha256_lf(GUIDE.as_bytes()),
    ))
    .unwrap()
}

fn shipped_state(file: &FileReport) -> State {
    match file.state {
        FileState::Shipped(state) => state,
        other => panic!("the guide has a shipped default's state, not {other:?}"),
    }
}

/// The guide's entry of a report.
fn guide(report: &Report) -> &FileReport {
    let mut found = report.files.iter().filter(|f| f.path == GUIDE_PATH);
    let file = found.next().expect("the report names the guide");
    assert!(found.next().is_none(), "{report:?}");
    file
}

fn read_guide(repo: &Path) -> Vec<u8> {
    fs::read(repo.join(GUIDE_PATH)).unwrap()
}

// --- the shipped list ----------------------------------------------------------------------------

#[test]
fn the_shipped_list_names_the_guide_at_its_current_text_and_not_genesis() {
    let shipped = Shipped::parse(SHIPPED).unwrap();

    assert_eq!(shipped.paths().collect::<Vec<_>>(), ["genesis-guide.md"]);
    assert_eq!(
        shipped.current("genesis-guide.md"),
        Some(sha256_lf(GUIDE.as_bytes()).as_str())
    );
    assert_eq!(GUIDE_PATH, "factory/genesis-guide.md");
}

// --- through create ------------------------------------------------------------------------------

#[test]
fn create_writes_a_missing_guide_and_lists_the_files_by_path() {
    let repo = repository("written");

    let report = Engine::genesis_create(&repo, false).unwrap();

    let paths: Vec<_> = report.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, [PATH, GUIDE_PATH]);
    assert_eq!(shipped_state(guide(&report)), State::Written);
    assert_eq!(read_guide(&repo), GUIDE.as_bytes());
    cleanup(&repo);
}

#[test]
fn the_current_guide_is_current_and_unchanged() {
    let repo = repository("current");
    fs::write(repo.join(GUIDE_PATH), GUIDE.replace('\n', "\r\n")).unwrap();

    for update in [false, true] {
        let report = Engine::genesis_create(&repo, update).unwrap();
        assert_eq!(shipped_state(guide(&report)), State::Current);
        assert_eq!(read_guide(&repo), GUIDE.replace('\n', "\r\n").as_bytes());
    }
    cleanup(&repo);
}

#[test]
fn a_team_changed_guide_is_unchanged_after_create_update() {
    let repo = repository("team");
    let ours = b"# Our own guide\r\n\xff not utf-8\n";
    fs::write(repo.join(GUIDE_PATH), ours).unwrap();

    for update in [false, true] {
        let report = Engine::genesis_create(&repo, update).unwrap();
        assert_eq!(shipped_state(guide(&report)), State::Team);
        assert_eq!(read_guide(&repo), ours);
    }
    cleanup(&repo);
}

#[test]
fn update_never_touches_genesis_even_when_it_reads_like_the_template() {
    let repo = repository("genesis-untouched");
    fs::write(repo.join(PATH), TEMPLATE).unwrap();
    fs::write(repo.join(GUIDE_PATH), "ours").unwrap();

    let report = Engine::genesis_create(&repo, true).unwrap();

    let genesis = report.files.iter().find(|f| f.path == PATH).unwrap();
    assert_eq!(genesis.state, FileState::Genesis(GenesisState::Team));
    assert_eq!(fs::read(repo.join(PATH)).unwrap(), TEMPLATE.as_bytes());
    cleanup(&repo);
}

// --- an earlier shipped guide --------------------------------------------------------------------

#[test]
fn an_older_shipped_guide_is_older_default_and_unchanged_without_update() {
    let repo = repository("older");
    fs::write(repo.join(GUIDE_PATH), OLDER).unwrap();

    let file = install_guide(&repo, &with_older(), GUIDE, false)
        .unwrap()
        .unwrap();

    assert_eq!(file.path, GUIDE_PATH);
    assert_eq!(shipped_state(&file), State::OlderDefault);
    assert_eq!(read_guide(&repo), OLDER.as_bytes());
    cleanup(&repo);
}

#[test]
fn an_older_shipped_guide_becomes_updated_with_update() {
    let repo = repository("updated");
    fs::write(repo.join(GUIDE_PATH), OLDER.replace('\n', "\r\n")).unwrap();

    let file = install_guide(&repo, &with_older(), GUIDE, true)
        .unwrap()
        .unwrap();

    assert_eq!(shipped_state(&file), State::Updated);
    assert_eq!(read_guide(&repo), GUIDE.as_bytes());
    let again = install_guide(&repo, &with_older(), GUIDE, true)
        .unwrap()
        .unwrap();
    assert_eq!(shipped_state(&again), State::Current);
    cleanup(&repo);
}

#[test]
fn a_symbolic_link_at_the_guide_path_is_the_teams_and_not_followed() {
    let repo = repository("symlink");
    let target = repo.parent().unwrap().join("outside.md");
    fs::write(&target, OLDER).unwrap();
    std::os::unix::fs::symlink(&target, repo.join(GUIDE_PATH)).unwrap();

    let file = install_guide(&repo, &with_older(), GUIDE, true)
        .unwrap()
        .unwrap();

    assert_eq!(shipped_state(&file), State::Team);
    assert_eq!(fs::read(&target).unwrap(), OLDER.as_bytes());
    assert!(fs::symlink_metadata(repo.join(GUIDE_PATH))
        .unwrap()
        .file_type()
        .is_symlink());
    cleanup(&repo);
}

#[test]
fn a_folder_at_the_guide_path_is_the_teams() {
    let repo = repository("folder");
    fs::create_dir_all(repo.join(GUIDE_PATH)).unwrap();

    let file = install_guide(&repo, &with_older(), GUIDE, true)
        .unwrap()
        .unwrap();

    assert_eq!(shipped_state(&file), State::Team);
    assert!(repo.join(GUIDE_PATH).is_dir());
    cleanup(&repo);
}

#[test]
fn the_guide_is_written_where_factory_does_not_exist_yet() {
    let repo = repository("no-factory");
    fs::remove_dir_all(repo.join("factory")).unwrap();

    let file = install_guide(&repo, &with_older(), GUIDE, false)
        .unwrap()
        .unwrap();

    assert_eq!(shipped_state(&file), State::Written);
    assert_eq!(read_guide(&repo), GUIDE.as_bytes());
    cleanup(&repo);
}

// --- a retired guide -----------------------------------------------------------------------------

#[test]
fn a_present_guide_whose_path_is_retired_is_never_touched() {
    let repo = repository("retired");
    fs::write(repo.join(GUIDE_PATH), GUIDE).unwrap();

    let file = install_guide(&repo, &retired(), GUIDE, true)
        .unwrap()
        .unwrap();

    assert_eq!(shipped_state(&file), State::Retired);
    assert_eq!(read_guide(&repo), GUIDE.as_bytes());
    cleanup(&repo);
}

#[test]
fn a_missing_guide_whose_path_is_retired_is_not_written_or_reported() {
    let repo = repository("retired-missing");

    let file = install_guide(&repo, &retired(), GUIDE, true).unwrap();

    assert_eq!(file, None);
    assert!(!repo.join(GUIDE_PATH).exists());
    cleanup(&repo);
}

// --- codeowners ----------------------------------------------------------------------------------

#[test]
fn codeowners_is_none_without_a_codeowners_file() {
    let repo = repository("no-codeowners");

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(report.codeowners, None);
    cleanup(&repo);
}

#[test]
fn codeowners_names_genesis_when_no_rule_covers_it_and_never_the_guide() {
    let repo = repository("uncovered");
    fs::create_dir_all(repo.join(".github")).unwrap();
    fs::write(repo.join(".github/CODEOWNERS"), "/src/ @owner\n").unwrap();

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(
        report.codeowners,
        Some(codeowners::Report {
            file: ".github/CODEOWNERS".to_owned(),
            uncovered: vec![PATH.to_owned()],
        })
    );
    cleanup(&repo);
}

#[test]
fn codeowners_lists_nothing_when_a_rule_covers_genesis() {
    let repo = repository("covered");
    fs::write(repo.join("CODEOWNERS"), "/factory/GENESIS.md @owner\n").unwrap();

    let report = Engine::genesis_create(&repo, false).unwrap();

    assert_eq!(
        report.codeowners,
        Some(codeowners::Report {
            file: "CODEOWNERS".to_owned(),
            uncovered: Vec::new(),
        })
    );
    cleanup(&repo);
}
