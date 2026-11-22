//! The data directory lies outside the repository (RFC 0001, "The local data on disk"): once the
//! root is found, a data directory inside it is refused, however its path is spelled.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use nexus_flow_store::data_dir::{DataDir, Platform};
use nexus_flow_store::repo_root::{RepoRoot, RootError};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-inside-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    // The temporary directory may itself be reached through a symlink (macOS's /var).
    dir.canonicalize().unwrap()
}

fn repo_in(base: &Path) -> RepoRoot {
    let repo = base.join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("nexus-flow.toml"), "format = 1\n").unwrap();
    RepoRoot::find(&repo).unwrap()
}

fn data_dir(path: &Path) -> DataDir {
    let path = path.to_path_buf();
    DataDir::resolve(Platform::Linux, move |name| {
        (name == "NXS_DATA_DIR").then(|| OsString::from(path.clone()))
    })
    .unwrap()
}

fn assert_inside(root: &RepoRoot, data: &Path) {
    let err = root.check_data_dir(&data_dir(data)).unwrap_err();
    assert!(
        matches!(err, RootError::DataDirInside { .. }),
        "{} is inside the root: {err:?}",
        data.display()
    );
    let message = err.to_string();
    assert!(message.contains(&data.display().to_string()), "{message}");
    assert!(
        message.contains(&root.path().display().to_string()),
        "{message}"
    );
}

#[test]
fn a_data_directory_inside_the_root_is_refused() {
    let base = scratch("under");
    let root = repo_in(&base);

    assert_inside(&root, &root.path().join(".nxs-data"));
    assert_inside(&root, &root.path().join("target").join("data"));
    assert_inside(&root, root.path());

    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_data_directory_outside_the_root_is_accepted() {
    let base = scratch("outside");
    let root = repo_in(&base);

    for outside in [
        base.join("data"),
        base.join("repo-data"),
        base.join("repo").join("..").join("data"),
    ] {
        assert_eq!(
            root.check_data_dir(&data_dir(&outside)),
            Ok(()),
            "{} is outside the root",
            outside.display()
        );
    }

    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_dotted_path_back_into_the_root_is_refused() {
    let base = scratch("dotted");
    let root = repo_in(&base);

    assert_inside(&root, &base.join("missing").join("..").join("repo").join("data"));
    assert_inside(&root, &base.join(".").join("repo").join("data"));

    fs::remove_dir_all(&base).unwrap();
}

#[cfg(unix)]
#[test]
fn a_symlink_into_the_root_is_refused() {
    let base = scratch("symlink");
    let root = repo_in(&base);
    let link = base.join("link");
    std::os::unix::fs::symlink(root.path(), &link).unwrap();

    assert_inside(&root, &link.join("data"));

    fs::remove_dir_all(&base).unwrap();
}
