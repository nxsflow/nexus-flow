//! `init` ends with the factory's install and then GENESIS's create, both without `update` (RFC
//! 0006, "Installing the shipped defaults"; RFC 0004, "Installing the template"): a new
//! repository gets the shipped defaults, GENESIS and the guide, and `init` run again, or after the
//! team changed them, leaves every one of those files as it was. An existing GENESIS, with any
//! bytes, is unchanged after `init` and after `Engine::factory_install(true)`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{block_on, options, scratch};
use nexus_flow_seam::genesis::{FileState, GenesisState, GUIDE_PATH, PATH, TEMPLATE};
use nexus_flow_seam::install::factory::Defaults;
use nexus_flow_seam::install::shipped::State;
use nexus_flow_seam::{init_with, Engine, Init};

/// Every file under `dir`, relative to it, with its bytes, in bytewise order of path.
fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path.strip_prefix(base).unwrap().to_path_buf();
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn init(base: &Path) -> Init {
    block_on(init_with(&base.join("repo"), options(&base.join("data")))).unwrap()
}

fn genesis_state(report: &Init) -> FileState {
    report
        .genesis
        .files
        .iter()
        .find(|f| f.path == PATH)
        .expect("GENESIS is in the report")
        .state
}

#[test]
fn init_installs_the_factory_defaults_then_genesis_and_its_guide() {
    let base = scratch("init-installs");
    let repo = base.join("repo");

    let report = init(&base);

    let defaults = Defaults::embedded();
    for path in defaults.shipped().paths() {
        let written = fs::read(repo.join("factory").join(path)).unwrap();
        assert_eq!(Some(written.as_slice()), defaults.text(path), "{path}");
    }
    assert!(report
        .factory
        .files
        .iter()
        .all(|f| f.state == State::Written));
    assert_eq!(fs::read_to_string(repo.join(PATH)).unwrap(), TEMPLATE);
    assert!(repo.join(GUIDE_PATH).is_file());
    assert_eq!(
        genesis_state(&report),
        FileState::Genesis(GenesisState::Written)
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_run_twice_leaves_every_factory_file_and_genesis_as_the_first_run_left_them() {
    let base = scratch("init-installs-twice");
    let repo = base.join("repo");
    init(&base);
    let first = snapshot(&repo.join("factory"));

    let report = init(&base);

    assert_eq!(snapshot(&repo.join("factory")), first);
    assert!(report
        .factory
        .files
        .iter()
        .all(|f| f.state == State::Current));
    assert_eq!(
        genesis_state(&report),
        FileState::Genesis(GenesisState::Team)
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_run_again_leaves_the_teams_factory_files_and_genesis_as_the_team_left_them() {
    let base = scratch("init-installs-team");
    let repo = base.join("repo");
    init(&base);
    for path in [
        "CONFIG.md",
        "roles/coder.md",
        "GENESIS.md",
        "genesis-guide.md",
    ] {
        fs::write(repo.join("factory").join(path), format!("# ours: {path}\n")).unwrap();
    }
    fs::remove_file(repo.join("factory/roles/pm.md")).unwrap();
    fs::write(repo.join("factory/roles/legal.md"), "# Legal\n").unwrap();
    let team = snapshot(&repo.join("factory"));

    let report = init(&base);

    let mut expected = team;
    let pm = defaults_text("roles/pm.md");
    expected.push((PathBuf::from("roles/pm.md"), pm));
    expected.sort();
    assert_eq!(snapshot(&repo.join("factory")), expected);
    assert_eq!(
        genesis_state(&report),
        FileState::Genesis(GenesisState::Team)
    );
    fs::remove_dir_all(&base).unwrap();
}

fn defaults_text(path: &str) -> Vec<u8> {
    Defaults::embedded().text(path).unwrap().to_vec()
}

#[test]
fn init_leaves_an_existing_genesis_unchanged_whatever_its_bytes() {
    for (name, bytes) in [
        ("template", TEMPLATE.as_bytes().to_vec()),
        ("arbitrary", b"\xff\x00 not even UTF-8\r\n".to_vec()),
    ] {
        let base = scratch(&format!("init-installs-genesis-{name}"));
        let repo = base.join("repo");
        fs::create_dir_all(repo.join("factory")).unwrap();
        fs::write(repo.join(PATH), &bytes).unwrap();

        for _ in 0..2 {
            let report = init(&base);

            assert_eq!(fs::read(repo.join(PATH)).unwrap(), bytes, "{name}");
            assert_eq!(
                genesis_state(&report),
                FileState::Genesis(GenesisState::Team)
            );
        }
        fs::remove_dir_all(&base).unwrap();
    }
}

#[test]
fn factory_install_with_update_leaves_genesis_unchanged() {
    for (name, bytes) in [
        ("template", TEMPLATE.as_bytes().to_vec()),
        ("arbitrary", b"# Our product\n".to_vec()),
    ] {
        let base = scratch(&format!("init-installs-update-{name}"));
        let repo = base.join("repo");
        init(&base);
        fs::write(repo.join(PATH), &bytes).unwrap();
        let engine = block_on(Engine::open_with(&repo, options(&base.join("data")))).unwrap();

        engine.factory_install(true).unwrap();

        assert_eq!(fs::read(repo.join(PATH)).unwrap(), bytes, "{name}");
        fs::remove_dir_all(&base).unwrap();
    }
}
