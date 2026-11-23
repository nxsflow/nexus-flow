//! Opening a space by id (RFC 0001, "The library seam", `Engine::open_space`; "Derived state"):
//! for a caller without a checkout, such as manufakt.io, the store opens the device member's
//! database and the one space, and holds no checkout, so every "on main" value is `null`. A
//! caller that names a checkout gets its root and base branch from its `nexus-flow.toml`.

mod common;

use std::fs;

use common::{block_on, data_dir, keys, scratch};
use nexus_flow_store::config::Config;
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::opened;
use nexus_flow_store::repo_root::RootError;
use nexus_flow_store::space::{init, InitError};

#[test]
fn a_space_opened_by_id_without_a_checkout_has_no_on_main() {
    let base = scratch("space-no-git");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let space: SpaceId = report.space.parse().unwrap();

    let opened = block_on(opened::space(space, None, &data, keys(&data))).unwrap();

    assert_eq!(opened.held(&space), Some(true));
    assert_eq!(opened.checkout(), None, "no checkout: on main is null");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_the_device_does_not_hold_yet_is_opened_as_not_held() {
    let base = scratch("space-unheld");
    let data = data_dir(&base.join("data"));
    let space: SpaceId = "00112233445566778899aabbccddeeff".parse().unwrap();

    let opened = block_on(opened::space(space, None, &data, keys(&data))).unwrap();

    assert_eq!(opened.held(&space), Some(false));
    assert!(base.join("data").join("device").is_dir());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_named_checkout_gives_its_root_and_base_branch() {
    let base = scratch("space-git");
    let data = data_dir(&base.join("data"));
    let report = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap();
    let mut config =
        Config::read(&nexus_flow_store::repo_root::RepoRoot::find(&base.join("repo")).unwrap())
            .unwrap();
    config.base = "develop".to_owned();
    fs::remove_file(base.join("repo").join("nexus-flow.toml")).unwrap();
    config.write_new(&base.join("repo")).unwrap();
    let nested = base.join("repo").join("src");
    fs::create_dir_all(&nested).unwrap();

    let space = report.space.parse().unwrap();
    let opened = block_on(opened::space(space, Some(&nested), &data, keys(&data))).unwrap();

    let checkout = opened.checkout().expect("a named checkout");
    assert_eq!(checkout.path, base.join("repo"));
    assert_eq!(checkout.base, "develop");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_named_checkout_without_nexus_flow_toml_is_refused() {
    let base = scratch("space-git-none");
    let data = data_dir(&base.join("data"));
    let space = "00112233445566778899aabbccddeeff".parse().unwrap();

    let err = block_on(opened::space(
        space,
        Some(&base.join("repo")),
        &data,
        keys(&data),
    ))
    .unwrap_err();

    assert!(
        matches!(err, InitError::Root(RootError::NotFound { .. })),
        "{err:?}"
    );
    assert!(!base.join("data").exists());
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_space_id_reads_back_as_it_shows() {
    let text = "00112233445566778899aabbccddeeff";
    let space: SpaceId = text.parse().unwrap();
    assert_eq!(space.to_string(), text);

    let err = "not-a-space-id".parse::<SpaceId>().unwrap_err();
    assert!(err.to_string().contains("not-a-space-id"), "{err}");
}
