//! `Engine::open` (RFC 0001, "The library seam"): opens the device member's database and the
//! spaces `nexus-flow.toml` names, from any folder of the repository. Outside a repository it
//! fails with an error that says to run `nxs init`.

mod common;

use std::fs;

use common::{block_on, options, scratch};
use nexus_flow_seam::{DeviceKeys, Engine, Options};
use nexus_flow_store::space::init;

#[test]
fn a_repository_with_nexus_flow_toml_opens() {
    let base = scratch("open");
    let setup = options(&base.join("data"));
    block_on(init(&base.join("repo"), &setup.data, setup.keys.clone())).unwrap();
    let nested = base.join("repo").join("src");
    fs::create_dir_all(&nested).unwrap();

    block_on(Engine::open_with(
        &base.join("repo"),
        options(&base.join("data")),
    ))
    .unwrap();
    block_on(Engine::open_with(&nested, options(&base.join("data")))).unwrap();
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn outside_a_repository_open_names_nxs_init() {
    let base = scratch("outside");

    let err = block_on(Engine::open_with(
        &base.join("repo"),
        options(&base.join("data")),
    ))
    .unwrap_err();
    assert!(err.to_string().contains("nxs init"), "{err}");
    assert!(!base.join("data").exists(), "nothing was created");

    let err = block_on(Engine::open(&base.join("repo"))).unwrap_err();
    assert!(err.to_string().contains("nxs init"), "{err}");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_default_keeps_the_device_keys_in_the_key_store() {
    if let Ok(options) = Options::from_env() {
        assert_eq!(options.keys, DeviceKeys::KeyStore);
    }
}
