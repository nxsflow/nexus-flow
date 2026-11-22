//! The layout inside the data directory (RFC 0001, "The local data on disk"):
//! `device/`, `sessions/<session id>/`, `derived/<member id>/` and `migration/<run>/`.

use std::ffi::OsString;
use std::path::PathBuf;

use nexus_flow_store::data_dir::{DataDir, DataDirError, Platform};

fn data_dir() -> (DataDir, PathBuf) {
    let root = std::env::temp_dir().join("nxs-layout");
    let value = root.clone();
    let dir = DataDir::resolve(Platform::current(), move |name| {
        (name == "NXS_DATA_DIR").then(|| OsString::from(value.clone()))
    })
    .unwrap();
    (dir, root)
}

#[test]
fn the_device_members_database_is_device() {
    let (dir, root) = data_dir();
    assert_eq!(dir.device(), root.join("device"));
}

#[test]
fn a_session_members_database_is_under_sessions() {
    let (dir, root) = data_dir();
    assert_eq!(
        dir.session("s-42").unwrap(),
        root.join("sessions").join("s-42")
    );
}

#[test]
fn a_members_derived_cache_is_under_derived() {
    let (dir, root) = data_dir();
    assert_eq!(dir.derived("m1").unwrap(), root.join("derived").join("m1"));
}

#[test]
fn a_migration_runs_database_is_under_migration() {
    let (dir, root) = data_dir();
    assert_eq!(
        dir.migration("run-7").unwrap(),
        root.join("migration").join("run-7")
    );
}

#[test]
fn an_id_that_is_not_one_plain_name_is_refused_so_no_path_leaves_its_folder() {
    let (dir, _) = data_dir();
    for id in ["", ".", "..", "../repo", "a/b", "/abs", r"a\b"] {
        assert!(
            matches!(dir.session(id), Err(DataDirError::NotAName { .. })),
            "session {id:?}"
        );
        assert!(dir.derived(id).is_err(), "derived {id:?}");
        assert!(dir.migration(id).is_err(), "migration {id:?}");
    }
}
