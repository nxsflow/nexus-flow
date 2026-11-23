//! `Engine::as_session` and `NXS_SESSION` (RFC 0001, "Reading and writing", "The library
//! seam"): an engine runs as a running session's member when `NXS_SESSION` names one, and
//! `as_session` reopens a device engine on a session's database. A session that is not running
//! fails with a clear error, and nothing is written. The store's tests open a running session
//! (`crates/store/tests/member_select.rs`): only `nxs session start` creates one, and the seam
//! does not have it yet.

mod common;

use std::fs;

use common::{block_on, options, scratch};
use nexus_flow_seam::{Engine, SessionId};
use nexus_flow_store::space::init;

#[test]
fn a_session_id_is_a_plain_folder_name() {
    assert_eq!(
        "coder-nex-98".parse::<SessionId>().unwrap().to_string(),
        "coder-nex-98"
    );
    assert!("../device".parse::<SessionId>().is_err());
}

#[test]
fn as_session_on_a_session_that_is_not_running_fails_and_creates_nothing() {
    let base = scratch("as-session-gone");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let engine = block_on(Engine::open_with(
        &base.join("repo"),
        options(&base.join("data")),
    ))
    .unwrap();

    let err = block_on(engine.as_session("gone".parse().unwrap())).unwrap_err();

    let text = err.to_string();
    assert!(
        text.contains("gone") && text.contains("not running"),
        "{text}"
    );
    assert!(!base.join("data").join("sessions").exists());
    fs::remove_dir_all(&base).unwrap();
}

/// The one test of this file that sets the process's environment, so no other test reads it
/// while it changes.
#[test]
fn open_runs_as_the_session_nxs_session_names() {
    let base = scratch("nxs-session");
    let data = base.join("data");
    std::env::set_var("NXS_DATA_DIR", &data);

    std::env::set_var("NXS_SESSION", "gone");
    let err = block_on(Engine::open_space(
        "00112233445566778899aabbccddeeff".parse().unwrap(),
        None,
    ))
    .unwrap_err();
    let text = err.to_string();
    assert!(
        text.contains("gone") && text.contains("NXS_SESSION"),
        "{text}"
    );
    assert!(!data.exists(), "nothing was written");

    std::env::set_var("NXS_SESSION", "../device");
    let err = block_on(Engine::open(&base.join("repo"))).unwrap_err();
    assert!(err.to_string().contains("NXS_SESSION"), "{err}");
    assert!(!data.exists(), "nothing was written");

    std::env::remove_var("NXS_SESSION");
    std::env::remove_var("NXS_DATA_DIR");
    fs::remove_dir_all(&base).unwrap();
}
