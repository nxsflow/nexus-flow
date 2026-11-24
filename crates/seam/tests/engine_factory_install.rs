//! `Engine::factory_install(repository, update)` (RFC 0006, "Public surfaces this RFC brings"; its
//! form waits for the owner in NEX-232): the seam's counterpart of `nxs factory install`, which
//! installs the defaults `nxs` embeds from `templates/factory/` into the `factory/` of the
//! repository at or above `repository`. It opens no database.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{block_on, options, scratch};
use nexus_flow_seam::install::factory::{Defaults, FactoryError};
use nexus_flow_seam::install::shipped::{Shipped, State};
use nexus_flow_seam::{init_with, Engine, RootError};

/// The shipped defaults in nexus-flow's source tree.
fn templates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../templates/factory")
}

/// Every file under `dir`, relative to it with `/`, in bytewise order.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let rel = path.strip_prefix(base).unwrap();
                out.push(rel.to_str().unwrap().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// A repository given its space by `init`, and the data directory, which the install never needs.
fn repository(test: &str) -> PathBuf {
    let base = scratch(test);
    let repo = base.join("repo");
    block_on(init_with(&repo, options(&base.join("data")))).unwrap();
    repo
}

// --- the embedded defaults -----------------------------------------------------------------------

#[test]
fn the_embedded_defaults_are_the_files_in_templates_factory() {
    let defaults = Defaults::embedded();
    let on_disk = fs::read_to_string(templates().join("shipped.toml")).unwrap();
    assert_eq!(defaults.shipped(), &Shipped::parse(&on_disk).unwrap());

    let files: Vec<String> = tree(&templates())
        .into_iter()
        .filter(|p| p != "shipped.toml")
        .collect();
    let shipped: Vec<&str> = defaults
        .shipped()
        .paths()
        .filter(|p| !defaults.shipped().is_retired(p))
        .collect();
    assert_eq!(files, shipped, "every file in templates/factory is shipped");
    for path in &files {
        let text = fs::read(templates().join(path)).unwrap();
        assert_eq!(defaults.text(path), Some(text.as_slice()), "{path}");
    }
}

#[test]
fn the_shipped_defaults_are_the_least_a_factory_needs() {
    let defaults = Defaults::embedded();
    let paths: Vec<&str> = defaults.shipped().paths().collect();
    for path in [
        "CONFIG.md",
        "FACTORY.md",
        "roles/coder.md",
        "roles/pm.md",
        "status-note.md",
    ] {
        assert!(paths.contains(&path), "{path} is shipped: {paths:?}");
    }
}

#[test]
fn neither_genesis_nor_the_genesis_guide_is_a_factory_default() {
    let defaults = Defaults::embedded();
    for path in defaults.shipped().paths() {
        assert!(
            !path.to_ascii_lowercase().contains("genesis"),
            "{path} belongs to RFC 0004's genesis, not to the factory"
        );
    }
}

// --- the engine ----------------------------------------------------------------------------------

#[test]
fn factory_install_writes_the_embedded_defaults_into_the_checkout() {
    let repo = repository("engine-factory-install");

    let report = Engine::factory_install(&repo, false).unwrap();

    let defaults = Defaults::embedded();
    let expected: Vec<String> = defaults
        .shipped()
        .paths()
        .map(|p| format!("factory/{p}"))
        .collect();
    let paths: Vec<String> = report.files.iter().map(|f| f.path.clone()).collect();
    assert_eq!(paths, expected);
    assert!(report.files.iter().all(|f| f.state == State::Written));
    assert_eq!(report.codeowners, None);
    for path in defaults.shipped().paths() {
        let written = fs::read(repo.join("factory").join(path)).unwrap();
        assert_eq!(Some(written.as_slice()), defaults.text(path), "{path}");
    }
}

#[test]
fn factory_install_again_with_update_finds_every_file_current() {
    let repo = repository("engine-factory-again");
    Engine::factory_install(&repo, false).unwrap();

    let report = Engine::factory_install(&repo, true).unwrap();

    assert!(report.files.iter().all(|f| f.state == State::Current));
}

#[test]
fn factory_install_keeps_a_team_changed_role_file_byte_identical() {
    let repo = repository("engine-factory-team");
    let file = repo.join("factory/roles/coder.md");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "# Our coder\n").unwrap();

    for update in [false, true] {
        let report = Engine::factory_install(&repo, update).unwrap();
        let coder = report
            .files
            .iter()
            .find(|f| f.path == "factory/roles/coder.md")
            .unwrap();
        assert_eq!(coder.state, State::Team);
        assert_eq!(fs::read_to_string(&file).unwrap(), "# Our coder\n");
    }
}

#[test]
fn factory_install_from_a_folder_below_the_root_installs_at_the_root() {
    let repo = repository("engine-factory-below");
    let below = repo.join("src/deep");
    fs::create_dir_all(&below).unwrap();

    Engine::factory_install(&below, false).unwrap();

    assert!(repo.join("factory/roles/pm.md").is_file());
    assert!(!below.join("factory").exists());
}

#[test]
fn factory_install_opens_no_database() {
    let base = scratch("engine-factory-no-database");
    let repo = base.join("repo");
    fs::write(
        repo.join("nexus-flow.toml"),
        "format = 1\nrepository = \"not read by the install\"\n",
    )
    .unwrap();

    let report = Engine::factory_install(&repo, false).unwrap();

    assert!(report.files.iter().all(|f| f.state == State::Written));
    assert!(!base.join("data").exists());
}

#[test]
fn factory_install_outside_a_repository_writes_nothing() {
    let base = scratch("engine-factory-no-repo");
    let repo = base.join("repo");

    let result = Engine::factory_install(&repo, false);

    assert!(
        matches!(result, Err(FactoryError::Root(RootError::NotFound { .. }))),
        "{result:?}"
    );
    assert!(!repo.join("factory").exists());
}
