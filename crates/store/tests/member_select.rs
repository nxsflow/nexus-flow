//! The member a command runs as (RFC 0001, "Reading and writing"): the session's member when
//! `NXS_SESSION` names a running session, otherwise the device's member. A session runs while
//! its folder under `sessions/` in the data directory holds its database and keys; a
//! `NXS_SESSION` that names no running session fails before anything is created.

mod common;

use std::ffi::OsString;
use std::fs;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::data_dir::DataDir;
use nexus_flow_store::id::{SessionId, SpaceId};
use nexus_flow_store::member_select::{RunAs, SessionFiles};
use nexus_flow_store::model;
use nexus_flow_store::opened;
use nexus_flow_store::space::{init, InitError};
use nocentraldb::{Db, OpenOptions};

/// The member `NXS_SESSION` selects, with only that variable set to `value`.
fn selected(value: Option<&str>) -> Result<Option<SessionId>, String> {
    let value = value.map(OsString::from);
    RunAs::session_from(|name| (name == "NXS_SESSION").then(|| value.clone()).flatten())
        .map_err(|e| e.to_string())
}

/// Starts the session `id` as `nxs session start` leaves it: a sub-agent's database and its keys
/// in the session's folder.
fn start_session(data: &DataDir, id: &SessionId) {
    let files = SessionFiles::of(data, id);
    let options = OpenOptions::new(model::LOCK)
        .sub_agent()
        .path(&files.database)
        .key_file(&files.keys);
    drop(block_on(Db::open(options)).unwrap());
}

#[test]
fn nxs_session_unset_or_empty_selects_the_device() {
    assert_eq!(selected(None), Ok(None));
    assert_eq!(selected(Some("")), Ok(None));
}

#[test]
fn nxs_session_names_the_session() {
    assert_eq!(
        selected(Some("coder-nex-98")),
        Ok(Some("coder-nex-98".parse().unwrap()))
    );
}

#[test]
fn a_session_id_that_is_no_plain_name_is_refused() {
    for id in ["..", "a/b", "a\\b", "."] {
        assert!(id.parse::<SessionId>().is_err(), "{id:?}");
        let err = selected(Some(id)).unwrap_err();
        assert!(err.contains("NXS_SESSION"), "{err}");
    }
}

#[test]
fn without_a_session_the_device_member_opens_the_repository() {
    let base = scratch("runs-as-device");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();

    let opened = block_on(opened::repository_as(
        &base.join("repo"),
        &data,
        RunAs::Device(keys(&data)),
    ))
    .unwrap();

    assert_eq!(opened.session(), None);
    assert_eq!(opened.held(&space), Some(true), "the device's database");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_running_session_opens_its_own_database() {
    let base = scratch("runs-as-session");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();
    let session: SessionId = "coder-1".parse().unwrap();
    start_session(&data, &session);
    let device_keys = fs::read(data.root().join("device.keys")).unwrap();

    let opened = block_on(opened::repository_as(
        &base.join("repo"),
        &data,
        RunAs::Session(session.clone()),
    ))
    .unwrap();

    assert_eq!(opened.session(), Some(&session));
    assert_eq!(
        opened.held(&space),
        Some(false),
        "the session's database, which nobody has added to the device's space"
    );
    assert_eq!(
        fs::read(data.root().join("device.keys")).unwrap(),
        device_keys
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_session_opens_a_space_by_id_without_the_device() {
    let base = scratch("runs-as-session-space");
    let data = data_dir(&base.join("data"));
    let session: SessionId = "coder-2".parse().unwrap();
    start_session(&data, &session);
    let space: SpaceId = "00112233445566778899aabbccddeeff".parse().unwrap();

    let opened = block_on(opened::space_as(
        space,
        None,
        &data,
        RunAs::Session(session.clone()),
    ))
    .unwrap();

    assert_eq!(opened.session(), Some(&session));
    assert!(
        !data.device().exists(),
        "the device's database is not created"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_device_engine_reopens_as_a_session() {
    let base = scratch("as-session");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();
    let session: SessionId = "coder-3".parse().unwrap();
    start_session(&data, &session);
    let device = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();

    let opened = block_on(device.as_session(&data, session.clone())).unwrap();

    assert_eq!(opened.session(), Some(&session));
    assert_eq!(opened.held(&space), Some(false));
    let checkout = opened.checkout().expect("the checkout is kept");
    assert_eq!(checkout.path, base.join("repo"));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_session_that_is_not_running_fails_and_creates_nothing() {
    let base = scratch("no-session");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let before = fs::read_dir(data.root()).unwrap().count();
    let session: SessionId = "gone".parse().unwrap();

    let err = block_on(opened::repository_as(
        &base.join("repo"),
        &data,
        RunAs::Session(session.clone()),
    ))
    .unwrap_err();

    assert!(matches!(err, InitError::NoSession { .. }), "{err:?}");
    let text = err.to_string();
    assert!(
        text.contains("gone") && text.contains("not running"),
        "{text}"
    );
    assert!(
        !data.root().join("sessions").exists(),
        "nothing was created"
    );
    assert_eq!(fs::read_dir(data.root()).unwrap().count(), before);

    let fresh = data_dir(&base.join("fresh"));
    let space: SpaceId = report.space.parse().unwrap();
    let err = block_on(opened::space_as(
        space,
        None,
        &fresh,
        RunAs::Session(session),
    ))
    .unwrap_err();
    assert!(matches!(err, InitError::NoSession { .. }), "{err:?}");
    assert!(!fresh.root().exists(), "not even the data directory");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_session_folder_without_its_database_is_not_running() {
    let base = scratch("half-session");
    let data = data_dir(&base.join("data"));
    let session: SessionId = "half".parse().unwrap();
    let files = SessionFiles::of(&data, &session);
    fs::create_dir_all(files.database.parent().unwrap()).unwrap();
    let space: SpaceId = "00112233445566778899aabbccddeeff".parse().unwrap();

    let err = block_on(opened::space_as(
        space,
        None,
        &data,
        RunAs::Session(session),
    ))
    .unwrap_err();

    assert!(matches!(err, InitError::NoSession { .. }), "{err:?}");
    assert!(!files.database.exists() && !files.keys.exists());
    fs::remove_dir_all(&base).unwrap();
}
