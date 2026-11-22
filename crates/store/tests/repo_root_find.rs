//! Finding the repository's root from the working directory (RFC 0001, "Reading and writing"):
//! the nearest folder, the working directory or one of its ancestors, that holds
//! `nexus-flow.toml`.

use std::fs;
use std::path::PathBuf;

use nexus_flow_store::repo_root::{RepoRoot, RootError};

/// A fresh, empty folder of the test's own under the system's temporary directory.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-root-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_root_is_the_nearest_folder_that_holds_the_file() {
    let base = scratch("nearest");
    let repo = base.join("repo");
    let deep = repo.join("crates").join("store").join("src");
    fs::create_dir_all(&deep).unwrap();
    fs::write(repo.join("nexus-flow.toml"), "format = 1\n").unwrap();

    let found = RepoRoot::find(&deep).expect("the root is found from a folder inside it");
    assert_eq!(found.path(), repo.as_path());
    assert_eq!(found.config_file(), repo.join("nexus-flow.toml"));

    let found = RepoRoot::find(&repo).expect("the root is found from itself");
    assert_eq!(found.path(), repo.as_path());

    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_nested_file_wins_over_one_further_up() {
    let base = scratch("nested");
    let outer = base.join("outer");
    let inner = outer.join("vendor").join("inner");
    let start = inner.join("src");
    fs::create_dir_all(&start).unwrap();
    fs::write(outer.join("nexus-flow.toml"), "format = 1\n").unwrap();
    fs::write(inner.join("nexus-flow.toml"), "format = 1\n").unwrap();

    let found = RepoRoot::find(&start).unwrap();
    assert_eq!(found.path(), inner.as_path());

    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn no_file_up_to_the_filesystem_root_is_an_error_naming_the_start() {
    let base = scratch("none");
    let start = base.join("a").join("b");
    fs::create_dir_all(&start).unwrap();

    let err = RepoRoot::find(&start).unwrap_err();
    assert_eq!(
        err,
        RootError::NotFound {
            start: start.clone()
        }
    );
    let message = err.to_string();
    assert!(message.contains("nexus-flow.toml"), "{message}");
    assert!(message.contains(&start.display().to_string()), "{message}");

    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_folder_named_like_the_file_does_not_count() {
    let base = scratch("folder");
    let repo = base.join("repo");
    fs::create_dir_all(repo.join("nexus-flow.toml")).unwrap();

    let err = RepoRoot::find(&repo).unwrap_err();
    assert_eq!(
        err,
        RootError::NotFound {
            start: repo.clone()
        }
    );

    fs::remove_dir_all(&base).unwrap();
}
