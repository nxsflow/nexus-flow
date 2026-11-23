//! Opening a repository's spaces (RFC 0001, "Reading and writing", "The library seam"): the
//! store finds the root from a folder in the repository, reads `nexus-flow.toml`, opens the
//! device member's database and the spaces the file names. A named space the device does not
//! hold yet is opened as not held, so a read answers from what the database holds.

mod common;

use std::fs;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::config::Config;
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened;
use nexus_flow_store::repo_root::RootError;
use nexus_flow_store::space::{init, InitError};

/// A space id no database of these tests holds.
const UNHELD: &str = "00112233445566778899aabbccddeeff";

#[test]
fn the_repository_space_the_file_names_is_opened_and_held() {
    let base = scratch("open-held");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();

    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();

    assert_eq!(opened.held(&space), Some(true));
    let checkout = opened.checkout().expect("a repository has a checkout");
    assert_eq!(checkout.path, base.join("repo"));
    assert_eq!(checkout.base, "main");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_folder_inside_the_repository_opens_the_same_space() {
    let base = scratch("open-nested");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let nested = base.join("repo").join("src");
    fs::create_dir_all(&nested).unwrap();

    let opened = block_on(opened::repository(&nested, &data, keys(&data))).unwrap();

    assert_eq!(opened.held(&report.space.parse().unwrap()), Some(true));
    assert_eq!(opened.checkout().unwrap().path, base.join("repo"));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_named_space_the_device_does_not_hold_yet_is_opened_as_not_held() {
    let base = scratch("open-unheld");
    let data = data_dir(&base.join("data"));
    let mut config = Config::new(UNHELD);
    config.coordination = Some("ffeeddccbbaa99887766554433221100".to_owned());
    config.base = "develop".to_owned();
    config.write_new(&base.join("repo")).unwrap();

    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();

    assert_eq!(opened.held(&UNHELD.parse().unwrap()), Some(false));
    assert_eq!(
        opened.held(&"ffeeddccbbaa99887766554433221100".parse().unwrap()),
        Some(false)
    );
    assert_eq!(opened.checkout().unwrap().base, "develop");
    assert!(
        base.join("data").join("device").is_dir(),
        "the device member's database is opened under device/"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_the_file_does_not_name_is_not_open() {
    let base = scratch("open-unnamed");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();

    let opened = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap();

    assert_eq!(opened.held(&UNHELD.parse().unwrap()), None);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn outside_a_repository_the_error_names_nxs_init_and_nothing_is_created() {
    let base = scratch("open-outside");
    let data = data_dir(&base.join("data"));

    let err = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap_err();

    assert!(
        matches!(err, InitError::Root(RootError::NotFound { .. })),
        "{err:?}"
    );
    assert!(err.to_string().contains("nxs init"), "{err}");
    assert!(!base.join("data").exists(), "no database was created");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_file_whose_space_id_is_not_one_is_refused_naming_the_file() {
    let base = scratch("open-bad-id");
    let data = data_dir(&base.join("data"));
    Config::new("not-a-space-id")
        .write_new(&base.join("repo"))
        .unwrap();

    let err = block_on(opened::repository(&base.join("repo"), &data, keys(&data))).unwrap_err();

    assert!(matches!(err, InitError::SpaceId { .. }), "{err:?}");
    let message = err.to_string();
    assert!(
        message.contains("nexus-flow.toml") && message.contains("not-a-space-id"),
        "{message}"
    );
    assert!(!base.join("data").exists(), "no database was created");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_data_directory_inside_the_repository_is_refused() {
    let base = scratch("open-inside");
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let inside = data_dir(&base.join("repo").join(".data"));

    let err = block_on(opened::repository(
        &base.join("repo"),
        &inside,
        keys(&inside),
    ))
    .unwrap_err();

    assert!(
        matches!(err, InitError::Root(RootError::DataDirInside { .. })),
        "{err:?}"
    );
    assert!(!base.join("repo").join(".data").exists());
    fs::remove_dir_all(&base).unwrap();
}
