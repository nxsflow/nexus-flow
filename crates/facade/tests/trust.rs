//! Whom a workspace believes, at the embedding seam (nxf 6j6v.pzkb): the `Engine` methods an app
//! (app-foundations → manufakt.io / nexflow.it) reads the verdict on an op and edits the trust list
//! through — the twins of `nxs sync key|trust|verify`. Tested HERE because the products speak
//! `Engine::*`, not the CLI.

use nexus_flow_core::store::Store;
use nexus_flow_facade::engine::Engine;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::workspace::{self, WorkspaceExt};
use nxs_foundation::signing::Provenance;
use tempfile::TempDir;

const NOW: &str = "2026-09-22T10:00:00Z";

/// A workspace, and the ops of a second replica — another machine — delivered into it the way a
/// sync pull delivers them.
fn workspace_with_a_peers_ops() -> (TempDir, Store, Vec<String>) {
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let mut peer = Store::open_in_memory(2);
    peer.create_item("zz99.0001", "task", "from the other machine", "bob");
    let theirs = peer.export();
    ws.open_store().unwrap().apply(&theirs);
    let ids = theirs.into_iter().map(|o| o.op_id).collect();
    (tmp, peer, ids)
}

#[test]
fn an_app_reads_the_verdict_and_edits_the_trust_list_through_the_engine() {
    let (tmp, peer, theirs) = workspace_with_a_peers_ops();
    let engine = Engine::open(None, tmp.path()).unwrap();
    let op = &theirs[0];

    let seen = engine.op_provenance(op).unwrap();
    assert_eq!(seen.provenance, Provenance::Verified);
    assert_eq!(seen.key_id.as_deref(), Some(peer.key_id()));
    assert_eq!(seen.author, "bob");
    assert!(!seen.trusted && !seen.acts);
    assert!(!engine.acts_on(op), "verified, but not trusted: no action");

    let signers = engine.untrusted_signers();
    assert_eq!(signers.len(), 1);
    assert_eq!(signers[0].key_id, peer.key_id());
    assert_eq!(signers[0].authors, ["bob"]);

    assert!(engine
        .trust_key(NOW, peer.key_id(), "bob's laptop")
        .unwrap());
    assert!(engine.acts_on(op), "trusted: its ops may act");
    let listed = engine.trusted_keys();
    assert_eq!(listed.len(), 2);
    assert!(listed[0].own && listed[0].key_id == engine.key_id());
    assert_eq!(
        (listed[1].name.as_str(), listed[1].added.as_str()),
        ("bob's laptop", NOW)
    );
    assert_eq!(
        engine.op_provenance(op).unwrap().trusted_as.as_deref(),
        Some("bob's laptop")
    );
    assert!(engine.untrusted_signers().is_empty());

    assert!(engine.distrust_key(peer.key_id()).unwrap());
    assert!(!engine.acts_on(op), "revoked at once");
}

#[test]
fn the_engine_signs_with_the_workspaces_key_and_its_own_writes_act() {
    let tmp = TempDir::new().unwrap();
    let ws = workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(
        engine.key_id(),
        ws.open_store().unwrap().key_id(),
        "one key per workspace, whoever opens it"
    );
    let mut store = ws.open_store().unwrap();
    store.create_item("ab12.0001", "task", "mine", "alice");
    let op = store.export().pop().unwrap();
    assert_eq!(op.key_id.as_deref(), Some(engine.key_id().as_str()));
    let seen = engine.op_provenance(&op.op_id).unwrap();
    assert_eq!(seen.provenance, Provenance::Own);
    assert!(engine.acts_on(&op.op_id));
}

#[test]
fn the_engine_refuses_what_it_cannot_honour() {
    let tmp = TempDir::new().unwrap();
    workspace::init_flow(tmp.path(), "issue-tracker").unwrap();
    let engine = Engine::open(None, tmp.path()).unwrap();
    assert_eq!(
        engine.op_provenance("no-such-op").unwrap_err().kind,
        ErrorKind::NotFound
    );
    assert!(!engine.acts_on("no-such-op"));
    assert_eq!(
        engine
            .trust_key(NOW, "ed25519:short", "x")
            .unwrap_err()
            .kind,
        ErrorKind::Validation
    );
    assert_eq!(
        engine.distrust_key(&engine.key_id()).unwrap_err().kind,
        ErrorKind::Validation,
        "the own key stays"
    );
}
