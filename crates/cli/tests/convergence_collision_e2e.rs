//! E4 Slice-2 acceptance — the proof that resolves the T5 caveat.
//!
//! T5 (convergence_e2e) proved convergence only with FIXTURE-COORDINATED distinct prefixes.
//! This test repeats it with two replicas that INDEPENDENTLY minted the SAME prefix — the
//! real, uncoordinated collision — and shows the full bab + T-remap chain holds end to end
//! over the running relay: register → Reassigned → local remap (structure AND free-text
//! references via dqj) → converge, WITHOUT aliasing the other replica's items.
//!
//! It deliberately exercises bab and T-remap TOGETHER (registration + the prefix swap),
//! not the parts in isolation.

mod common;

use assert_cmd::Command;
use common::spawn_relay;
use nxs_test_support::PinHome;
use std::path::Path;
use tempfile::TempDir;

fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.current_dir(dir);
    c
}

/// The umbrella binary — the sync verb moved here in aye.2.1 (`nxs sync bind/run`). `bind` now
/// upserts the workspace into `~/.nexusflow/workspaces.toml` (kgn5) on every success — route
/// `HOME` at a dir-local sandbox, through [`PinHome`], so this test never touches the developer's
/// real registry. That helper carries the variables that would out-vote a pinned home (the `XDG_*`
/// overrides, and the service instance this repo's `.envrc` names) rather than leaving each of
/// them to a line here.
fn nxs(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxs");
    c.current_dir(dir).pin_home(dir.join(".fake-home"));
    c
}

/// Pin a replica to a known `(site, prefix, replica_uuid)`. Both replicas here are pinned
/// to the SAME prefix — the collision — but DISTINCT durable uuids (the identity anchor).
fn init_replica(dir: &Path, site: i64, prefix: &str, uuid: &str) {
    nxf(dir)
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    std::fs::write(
        dir.join(".nxs").join("replica.toml"),
        format!("site_id = {site}\nprefix = \"{prefix}\"\nreplica_uuid = \"{uuid}\"\n"),
    )
    .unwrap();
}

fn run_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    // The sync verb moved to the umbrella (aye.2.1): route `sync …` to `nxs`, the rest to `nxf`.
    let mut cmd = if args.first() == Some(&"sync") {
        nxs(dir)
    } else {
        nxf(dir)
    };
    let out = cmd
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json output")
}

fn create_task(dir: &Path, title: &str) -> String {
    run_json(
        dir,
        &[
            "create",
            "--type",
            "bug",
            "--title",
            title,
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ],
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn list_ids(dir: &Path) -> Vec<String> {
    run_json(dir, &["list", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn sync(dir: &Path, relay: &str) -> serde_json::Value {
    run_json(dir, &["sync", "run", "--remote", relay, "--json"])
}

fn suffix(id: &str) -> &str {
    id.split_once('.').unwrap().1
}

#[test]
fn colliding_prefixes_converge_end_to_end_without_aliasing() {
    let relay = spawn_relay();
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    // SAME prefix "aaaa", independently minted — distinct uuids tell them apart.
    init_replica(a.path(), 111, "aaaa", "uuid-aaaa-A");
    init_replica(b.path(), 222, "aaaa", "uuid-aaaa-B");

    // Each works OFFLINE under the colliding prefix.
    let a_id = create_task(a.path(), "A-one");
    let b_one = create_task(b.path(), "B-one");
    let b_two = create_task(b.path(), "B-two");
    // B-two cites B-one in its FREE TEXT and records the reference edge (the bm0 rule),
    // so the remap must fix BOTH the structure and the description text (dqj).
    run_json(
        b.path(),
        &[
            "update",
            &b_two,
            "--set",
            &format!("description=blocked by {b_one}"),
            "--json",
        ],
    );
    run_json(b.path(), &["mention", "add", &b_two, &b_one, "--json"]);
    assert!(a_id.starts_with("aaaa.") && b_one.starts_with("aaaa.") && b_two.starts_with("aaaa."));

    // Bind one shared stream (A creates, B joins out-of-band).
    let stream = run_json(
        a.path(),
        &["sync", "bind", "--no-daemon", "--create", "--json"],
    )["stream_id"]
        .as_str()
        .unwrap()
        .to_string();
    run_json(
        b.path(),
        &["sync", "bind", "--no-daemon", "--join", &stream, "--json"],
    );

    // A syncs first → it claims "aaaa" (Registered, no remap).
    let a_sync = sync(a.path(), &relay);
    assert!(a_sync["reassigned_prefix"].is_null(), "A keeps its prefix");

    // B syncs → the relay reassigns it (collision); B remaps its local store BEFORE merging.
    let b_sync = sync(b.path(), &relay);
    let b_new = b_sync["reassigned_prefix"]
        .as_str()
        .expect("B was reassigned a new prefix");
    assert_ne!(b_new, "aaaa", "B got a genuinely different prefix");

    // B's own ids were remapped locally: prefix swapped, suffixes preserved. The OLD
    // colliding ids are gone from B (so they can never alias A's on merge).
    let b_one_new = format!("{b_new}.{}", suffix(&b_one));
    let b_two_new = format!("{b_new}.{}", suffix(&b_two));
    let b_ids = list_ids(b.path());
    assert!(
        b_ids.contains(&b_one_new) && b_ids.contains(&b_two_new),
        "B's items were remapped to the new prefix: {b_ids:?}"
    );
    assert!(
        !b_ids.contains(&b_one) && !b_ids.contains(&b_two),
        "B's original colliding ids are gone: {b_ids:?}"
    );
    // dqj: the free-text citation in B-two's description now points at the remapped id.
    let b_two_item = run_json(b.path(), &["show", &b_two_new, "--json"])["item"].clone();
    assert_eq!(b_two_item["description"], format!("blocked by {b_one_new}"));
    // ...and the structural mention edge moved with it.
    let mentions = run_json(b.path(), &["mention", "list", &b_two_new, "--json"]);
    assert_eq!(mentions, serde_json::json!([b_one_new]));

    // Drive both to a fixed point.
    for _ in 0..2 {
        sync(a.path(), &relay);
        sync(b.path(), &relay);
    }

    // Identical materialized state, no manual merge — and crucially THREE distinct items:
    // A's create did NOT alias either of B's, even though all three were minted "aaaa.*".
    let ids_a = list_ids(a.path());
    assert_eq!(ids_a, list_ids(b.path()), "replicas converge");
    let expected = {
        let mut v = vec![a_id.clone(), b_one_new.clone(), b_two_new.clone()];
        v.sort();
        v
    };
    assert_eq!(ids_a, expected, "three distinct items, no aliasing");

    // The reassigned replica's references are resolvable on A too (it pulled the remapped
    // ops, never the colliding originals) — both the free-text description AND the mention edge.
    let b_two_on_a = run_json(a.path(), &["show", &b_two_new, "--json"])["item"].clone();
    assert_eq!(b_two_on_a["description"], format!("blocked by {b_one_new}"));
    let mentions_on_a = run_json(a.path(), &["mention", "list", &b_two_new, "--json"]);
    assert_eq!(mentions_on_a, serde_json::json!([b_one_new]));
}
