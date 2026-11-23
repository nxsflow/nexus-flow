//! `init` (RFC 0001, "Creating and finding a space"): the seam's counterpart of `nxs init`. It
//! creates the repository space and `nexus-flow.toml` in a repository that has none, and run again
//! it changes nothing and names the same space.

mod common;

use std::fs;

use common::{block_on, options, scratch};
use nexus_flow_seam::{init_with, Engine, SyncMode};

#[test]
fn init_creates_the_space_and_nexus_flow_toml() {
    let base = scratch("init");
    let repo = base.join("repo");

    let report = block_on(init_with(&repo, options(&base.join("data")))).unwrap();

    assert!(report.created);
    assert_eq!(report.sync, SyncMode::Island);
    assert!(!report.v1_workspace);
    let file = fs::read_to_string(repo.join("nexus-flow.toml")).unwrap();
    assert!(file.contains(&report.space), "{file}");
    block_on(Engine::open_with(&repo, options(&base.join("data")))).unwrap();
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn init_run_twice_changes_nothing() {
    let base = scratch("init-twice");
    let repo = base.join("repo");
    let first = block_on(init_with(&repo, options(&base.join("data")))).unwrap();
    let file = fs::read(repo.join("nexus-flow.toml")).unwrap();

    let second = block_on(init_with(&repo, options(&base.join("data")))).unwrap();

    assert!(!second.created);
    assert_eq!(second.space, first.space);
    assert_eq!(fs::read(repo.join("nexus-flow.toml")).unwrap(), file);
    fs::remove_dir_all(&base).unwrap();
}
