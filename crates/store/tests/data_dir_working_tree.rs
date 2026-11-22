//! Opening the data directory writes nothing inside the repository's working tree (RFC 0001,
//! "The local data on disk"). The test runs from inside a working tree, so it changes the
//! process's working directory and lives alone in its own test binary.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use nexus_flow_store::data_dir::{DataDir, Platform};

/// Every file and folder under `dir`, with each file's contents.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut seen = BTreeMap::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(next) = todo.pop() {
        for entry in fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                seen.insert(path.clone(), None);
                todo.push(path);
            } else {
                let contents = fs::read(&path).unwrap();
                seen.insert(path, Some(contents));
            }
        }
    }
    seen
}

fn env(vars: Vec<(&'static str, PathBuf)>) -> impl Fn(&str) -> Option<OsString> {
    move |name| {
        vars.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone().into_os_string())
    }
}

#[test]
fn opening_the_data_directory_leaves_the_working_tree_unchanged() {
    let base = std::env::temp_dir().join(format!("nxs-tree-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let repo = base.join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("nexus-flow.toml"), "space = \"s1\"\n").unwrap();
    fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/v2\n").unwrap();
    std::env::set_current_dir(&repo).unwrap();
    let before = snapshot(&repo);

    let data = base.join("data");
    let dir = DataDir::resolve(Platform::current(), env(vec![("NXS_DATA_DIR", data.clone())]))
        .unwrap();
    dir.open().expect("the data directory opens");
    assert!(data.is_dir(), "opening creates the data directory");

    let home = base.join("home");
    let dir = DataDir::resolve(
        Platform::Linux,
        env(vec![("HOME", home.clone()), ("XDG_DATA_HOME", "share".into())]),
    )
    .unwrap();
    dir.open().expect("the data directory opens");
    assert!(home.join(".local/share/nexus-flow").is_dir());

    let relative = DataDir::resolve(
        Platform::current(),
        env(vec![("NXS_DATA_DIR", "data".into())]),
    );
    assert!(relative.is_err(), "a relative NXS_DATA_DIR opens nothing");

    assert_eq!(snapshot(&repo), before, "the working tree is unchanged");
    std::env::set_current_dir(std::env::temp_dir()).unwrap();
    fs::remove_dir_all(&base).unwrap();
}
