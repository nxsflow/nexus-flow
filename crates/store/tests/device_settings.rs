//! The device member's own settings (RFC 0001, "Sync", **One relay per member database.**): the
//! relay it syncs through lives in the data directory, never in `nexus-flow.toml`, since which
//! relay a machine syncs through is the machine's choice. Only a `ws://` or `wss://` URL is a
//! relay's, as NoCentralDB opens no other.

mod common;

use std::fs;

use common::{data_dir, scratch};
use nexus_flow_store::device::{DeviceSettings, RelayUrl};

#[test]
fn a_fresh_data_directory_has_no_relay_and_never_synced() {
    let base = scratch("settings-fresh");
    let data = data_dir(&base.join("data"));

    let settings = DeviceSettings::read(&data).unwrap();

    assert_eq!(settings, DeviceSettings::default());
    assert_eq!(settings.relay, None);
    assert_eq!(settings.synced_at, None);
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_relay_written_is_read_back_from_the_data_directory() {
    let base = scratch("settings-relay");
    let data = data_dir(&base.join("data"));
    let relay: RelayUrl = "wss://relay.example.org/team".parse().unwrap();

    let settings = DeviceSettings {
        relay: Some(relay.clone()),
        synced_at: Some(1_795_000_000),
    };
    settings.write(&data).unwrap();

    assert!(data.settings().starts_with(data.root()));
    assert_eq!(DeviceSettings::read(&data).unwrap(), settings);
    assert_eq!(relay.as_str(), "wss://relay.example.org/team");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn only_a_ws_or_wss_url_is_a_relay() {
    assert!("ws://127.0.0.1:4000".parse::<RelayUrl>().is_ok());
    assert!("wss://relay.example.org".parse::<RelayUrl>().is_ok());
    for text in ["", "https://relay.example.org", "relay.example.org", "wss://", "ws://"] {
        let refused = text.parse::<RelayUrl>().unwrap_err();
        assert!(
            refused.to_string().contains("ws://"),
            "{text:?}: {refused}"
        );
    }
}

#[test]
fn a_settings_file_with_an_unknown_key_is_refused() {
    let base = scratch("settings-unknown");
    let data = data_dir(&base.join("data"));
    data.open().unwrap();
    fs::write(data.settings(), "relay = \"wss://r.example.org\"\ncolour = \"red\"\n").unwrap();

    let refused = DeviceSettings::read(&data).unwrap_err();

    assert!(refused.to_string().contains("colour"), "{refused}");
    fs::remove_dir_all(&base).unwrap();
}
