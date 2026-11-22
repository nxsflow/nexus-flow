//! Writing `nexus-flow.toml` never overwrites a file that is already there (RFC 0001, "Creating
//! and finding a space"; GENESIS.md: never overwrite a team's configuration).

use std::fs;
use std::path::PathBuf;

use nexus_flow_store::config::{Config, Problem};
use nexus_flow_store::repo_root::RepoRoot;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-config-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_new_file_is_written_and_read_back_from_the_root() {
    let dir = scratch("new");
    let config = Config {
        repository: "sp_repo".into(),
        coordination: None,
        base: "v2".into(),
    };

    config.write_new(&dir).expect("the file is written");
    let root = RepoRoot::find(&dir).expect("the written file marks the root");
    assert_eq!(Config::read(&root), Ok(config));

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_that_names_a_repository_space_is_never_overwritten() {
    let dir = scratch("names");
    let file = dir.join("nexus-flow.toml");
    let existing = b"# the team's own words\nformat = 1\n\n[space]\nrepository = \"sp_team\"\n\n[git]\nbase = \"develop\"\n";
    fs::write(&file, existing).unwrap();

    let err = Config::new("sp_other").write_new(&dir).unwrap_err();
    assert_eq!(err.file, file);
    assert_eq!(
        err.problem,
        Problem::AlreadyNamesSpace {
            repository: "sp_team".into()
        }
    );
    assert!(err.to_string().contains("sp_team"), "{err}");
    assert_eq!(
        fs::read(&file).unwrap(),
        existing,
        "the bytes are unchanged"
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_file_of_a_future_format_that_names_a_space_is_never_overwritten() {
    let dir = scratch("future");
    let file = dir.join("nexus-flow.toml");
    let existing = b"format = 2\n[space]\nrepository = \"sp_future\"\nshards = 4\n";
    fs::write(&file, existing).unwrap();

    let err = Config::new("sp_other").write_new(&dir).unwrap_err();
    assert_eq!(
        err.problem,
        Problem::AlreadyNamesSpace {
            repository: "sp_future".into()
        }
    );
    assert_eq!(
        fs::read(&file).unwrap(),
        existing,
        "the bytes are unchanged"
    );

    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn any_other_existing_file_is_not_overwritten_either() {
    let dir = scratch("other");
    let file = dir.join("nexus-flow.toml");
    let existing = b"[git]\nbase = \"develop\"\n";
    fs::write(&file, existing).unwrap();

    let err = Config::new("sp_other").write_new(&dir).unwrap_err();
    assert_eq!(err.problem, Problem::AlreadyExists);
    assert_eq!(
        fs::read(&file).unwrap(),
        existing,
        "the bytes are unchanged"
    );

    fs::remove_dir_all(&dir).unwrap();
}
