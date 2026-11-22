//! NEX-177: what the reader takes from a v1 workspace's files (RFC 0008, "What v1 holds").

mod fixture;

use fixture::Fixture;
use nexus_flow_migrate::v1_reader::V1Workspace;

#[test]
fn it_reads_the_replica_the_plugin_the_name_and_the_stream_id() {
    let f = Fixture::v1();
    let ws = V1Workspace::open(&f.nxs()).unwrap();
    assert_eq!(ws.schema(), 9);
    assert_eq!(ws.replica().site_id, 1);
    assert_eq!(ws.replica().prefix, "6j6v");
    assert_eq!(ws.replica().replica_uuid, "0b5d6c1e-replica");
    assert_eq!(ws.plugin(), "issue-tracker");
    assert_eq!(ws.name(), Some("nexus-flow"));
    assert_eq!(ws.stream_id(), Some("nxsflow-nexus-flow"));
}

#[test]
fn a_workspace_never_bound_to_a_stream_has_no_stream_id() {
    let f = Fixture::v1();
    f.remove("sync.toml");
    let ws = V1Workspace::open(&f.nxs()).unwrap();
    assert_eq!(ws.stream_id(), None);
}

/// v1 reads a missing `config.toml` as the default config, whose plugin is `issue-tracker`, and
/// a config from before `[flow]` as its top-level `plugin` (v1's `flow_plugin`).
#[test]
fn the_plugin_falls_back_as_v1_resolves_it() {
    let f = Fixture::v1();
    f.remove("config.toml");
    let ws = V1Workspace::open(&f.nxs()).unwrap();
    assert_eq!(ws.plugin(), "issue-tracker");
    assert_eq!(ws.name(), None);

    f.write("config.toml", "plugin = \"issue-tracker\"\n");
    assert_eq!(
        V1Workspace::open(&f.nxs()).unwrap().plugin(),
        "issue-tracker"
    );
}

/// `sync.toml`'s endpoint may carry a password: a TOML error would quote the line it failed on,
/// so the error names the file and nothing of its contents.
#[test]
fn a_damaged_sync_toml_is_reported_without_its_contents() {
    let f = Fixture::v1();
    f.write(
        "sync.toml",
        "stream_id = \"s\"\nendpoint = \"https://user:hunter2@relay.example/\"\nbroken =\n",
    );
    let err = V1Workspace::open(&f.nxs()).unwrap_err().to_string();
    assert!(err.contains("sync.toml"), "{err}");
    assert!(!err.contains("hunter2"), "the endpoint leaked: {err}");
}

/// `signing.key` is never read: a workspace whose key cannot be read opens all the same.
#[test]
fn the_signing_key_is_never_read() {
    let f = Fixture::v1();
    f.remove("signing.key");
    std::fs::create_dir(f.nxs().join("signing.key")).unwrap();
    V1Workspace::open(&f.nxs()).unwrap();
}
