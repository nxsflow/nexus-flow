//! `nxs sync bind` — the explicit create-vs-join stream-binding step (E4 T-bind, §4.2). The verb
//! moved from `nxf` to the umbrella in aye.2.1; the workspace is still created via `nxf init`.

use assert_cmd::Command;
use nxs_test_support::PinHome;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

fn nxf() -> Command {
    nxs_test_support::cargo_bin("nxf")
}

/// The umbrella binary. `bind` now upserts the workspace into `~/.nexusflow/workspaces.toml`
/// (kgn5) on every success — route `HOME` at a dir-local sandbox, through [`PinHome`], so these
/// tests never touch the developer's real registry. That helper carries the variables that would
/// out-vote a pinned home (the `XDG_*` overrides that redirect the same lookup on Linux, and the
/// service instance this repo's `.envrc` names) rather than leaving each of them to a line here.
fn nxs(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxs");
    c.current_dir(dir).pin_home(dir.join(".fake-home"));
    c
}

fn init(dir: &Path) {
    nxf()
        .args(["init", "--plugin", "issue-tracker"])
        .current_dir(dir)
        .assert()
        .success();
}

/// `git <args>` in `dir`, asserting success — used to give a fixture a real `origin` remote for
/// the derived-bind path.
fn git(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn json(out: Vec<u8>) -> serde_json::Value {
    serde_json::from_slice(&out).expect("json output")
}

#[test]
fn create_emits_a_fresh_stream_id() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxs(tmp.path())
        .args(["sync", "bind", "--no-daemon", "--create", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["mode"], "create");
    assert!(v["stream_id"].as_str().unwrap().starts_with("stream-"));
}

#[test]
fn join_binds_the_given_stream_id() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-shared-42",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["mode"], "join");
    assert_eq!(v["stream_id"], "stream-shared-42");
}

/// The real CLI/arg-parser/`--json` seam for the new default path (kgn5): a workspace with a
/// usable `origin` remote binds with no flags at all and derives its `stream_id` from it. This
/// complements (does not replace) the `bind_derived` unit tests in
/// `crates/nxs/src/sync/mod.rs`, which call the private helper directly and so never exercise
/// clap parsing or the `--json` envelope.
#[test]
fn bind_without_flags_derives_from_the_origin_remote() {
    let tmp = TempDir::new().unwrap();
    git(tmp.path(), &["init", "-q"]);
    git(
        tmp.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/nxsflow/manufakt-io",
        ],
    );
    init(tmp.path());
    let out = nxs(tmp.path())
        .args(["sync", "bind", "--no-daemon", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["mode"], "derived");
    assert!(v["stream_id"].as_str().unwrap().starts_with("stream-"));
}

/// Review finding Integrity #2 (PR #263): the default derived path makes `stream_id` a pure
/// function of the repo remote against a relay with NO authentication (`crates/server/src/app.rs`)
/// — the project owner decided to keep the derivation (that convergence is the whole point of the
/// default path) but make the exposure impossible to miss. A derived bind must warn on stderr.
#[test]
fn bind_without_flags_warns_that_the_derived_id_is_guessable() {
    let tmp = TempDir::new().unwrap();
    git(tmp.path(), &["init", "-q"]);
    git(
        tmp.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/nxsflow/manufakt-io",
        ],
    );
    init(tmp.path());
    nxs(tmp.path())
        .args(["sync", "bind", "--no-daemon", "--json"])
        .assert()
        .success()
        .stderr(predicates::str::contains("no authentication"))
        .stderr(predicates::str::contains("--create"));
}

/// The counterpart to the warning above: `--create` mints a random, undiscoverable ULID — not
/// derived from anything public — so it must NOT print the derived-id warning.
#[test]
fn bind_with_create_does_not_warn_about_a_derived_id() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxs(tmp.path())
        .args(["sync", "bind", "--no-daemon", "--create", "--json"])
        .assert()
        .success()
        .stderr(predicates::str::contains("no authentication").not());
}

/// Renamed from `bind_without_create_or_join_is_rejected` (kgn5): "no flags" is no longer itself
/// the rejected condition — it is now the DEFAULT derivation path (see
/// `bind_without_flags_derives_from_the_origin_remote`, above). This fixture has no `.git` at
/// all, so there is nothing to derive from; THAT — the missing origin, not the missing flags —
/// is what is rejected here.
#[test]
fn bind_without_a_git_origin_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxs(tmp.path())
        .args(["sync", "bind", "--no-daemon", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["error"]["kind"], "validation");
    let msg = v["error"]["msg"].as_str().unwrap();
    assert!(msg.contains("--create"), "{msg}");
    assert!(msg.contains("--join"), "{msg}");
}

#[test]
fn create_and_join_together_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--create",
            "--join",
            "x",
            "--json",
        ])
        .assert()
        .failure();
}

/// Was `rebinding_an_already_bound_workspace_is_rejected`, asserting that ANY second bind
/// failed. Post-kgn5 that is only true for a *differing* id (see
/// `rebinding_to_the_same_id_is_a_successful_no_op`, below, for the now-legal same-id case) —
/// updated to bind two different explicit ids so it also pins the "names both ids, mentions
/// --rebind" shape of the conflict, matching the unit tests in `crates/nxs/src/sync/mod.rs`.
#[test]
fn rebinding_to_a_different_id_without_rebind_is_rejected_and_names_both() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--json",
        ])
        .assert()
        .success();
    let out = nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-two",
            "--json",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["error"]["kind"], "conflict");
    let msg = v["error"]["msg"].as_str().unwrap();
    assert!(
        msg.contains("stream-one") && msg.contains("stream-two"),
        "{msg}"
    );
    assert!(msg.contains("--rebind"), "{msg}");
}

/// The idempotent case (kgn5 hwr2), at the CLI seam and not only as the unit test in
/// `crates/nxs/src/sync/mod.rs`: re-running `bind` with the SAME id succeeds and is a no-op,
/// which is what makes `bind` safe to call from scripts and from `nxs init`.
#[test]
fn rebinding_to_the_same_id_is_a_successful_no_op() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--json",
        ])
        .assert()
        .success();
    let out = nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["mode"], "unchanged");
    assert_eq!(v["stream_id"], "stream-one");
}

/// Review finding Code Quality #1 (PR #263): `--rebind` on an id that's ALREADY the bound one
/// must still be the no-op `rebinding_to_the_same_id_is_a_successful_no_op` above proves for the
/// no-flag case — `bind_with` used to check `!opts.rebind` before the id-equality test, so
/// `--rebind` on an unchanged id fell through to the switch path: deleted `sync.toml` and reset
/// both watermarks for a bind that changed nothing. Pins both halves of the fix: `mode` reports
/// `"unchanged"` (not `"rebind"`), and the watermarks a prior sync pass established survive.
#[test]
fn rebind_flag_on_an_unchanged_id_is_a_no_op_and_preserves_watermarks() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--json",
        ])
        .assert()
        .success();

    // Simulate a prior sync pass having advanced both watermarks — `bind`'s own JSON output
    // carries no watermark fields, so this is written directly rather than driven through a real
    // relay, exactly what a genuine switch (`rebind_flag_switches_the_stream_at_the_cli_seam`)
    // would reset to (0, 0) were this wrongly treated as one.
    let meta_path = tmp.path().join(".nxs").join("sync.toml");
    std::fs::write(
        &meta_path,
        "stream_id = \"stream-one\"\npushed_through = 42\npulled_through = 17\n",
    )
    .unwrap();

    let out = nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--rebind",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["mode"], "unchanged", "same id: --rebind has no teeth");
    assert_eq!(v["stream_id"], "stream-one");

    let raw = std::fs::read_to_string(&meta_path).unwrap();
    assert!(
        raw.contains("pushed_through = 42") && raw.contains("pulled_through = 17"),
        "watermarks must survive a --rebind that changed nothing: {raw}"
    );
}

/// `--rebind` end-to-end through the real arg parser (the unit tests call `bind`/the test-only
/// `rebind` wrapper directly and so never exercise clap for this flag): switches the stream and
/// reports `mode: "rebind"`.
#[test]
fn rebind_flag_switches_the_stream_at_the_cli_seam() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--json",
        ])
        .assert()
        .success();
    let out = nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-two",
            "--rebind",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["mode"], "rebind");
    assert_eq!(v["stream_id"], "stream-two");
}

// Review findings (post-task-8), both closing a coverage gap around an already-correct
// implementation rather than a behaviour change:
//
// - `--create` on an already-bound workspace was left untested at every seam once the CLI test
//   that used to bind twice via `--create` was rewritten to use `--join` for the differing-id
//   case above. `--create` always mints a fresh id (never idempotent — see
//   `intended_stream_id`'s `(true, _)` branch in `crates/nxs/src/sync/mod.rs`), so on a bound
//   workspace it is unconditionally a switch: refused without `--rebind`, performed with it.
// - `--rebind` on a workspace that was NEVER bound has nothing to switch from and must be
//   reported as an ordinary first bind, not `"rebind"` — a real bug caught in self-review during
//   task 8 (see the `switched` flag in `bind()`) that shipped with no regression test.

/// `--create` on an already-bound workspace, sub-case 1: refused without `--rebind`, naming the
/// existing id (mirrors `create_on_an_already_bound_workspace_without_rebind_is_a_conflict_naming_the_existing_id`
/// in `crates/nxs/src/sync/mod.rs`, at the CLI seam).
#[test]
fn create_on_an_already_bound_workspace_without_rebind_is_rejected() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--json",
        ])
        .assert()
        .success();
    let out = nxs(tmp.path())
        .args(["sync", "bind", "--no-daemon", "--create", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["error"]["kind"], "conflict");
    let msg = v["error"]["msg"].as_str().unwrap();
    assert!(msg.contains("stream-one"), "{msg}");
    assert!(msg.contains("--rebind"), "{msg}");
}

/// `--create` on an already-bound workspace, sub-case 2: `--rebind` performs the switch, minting
/// a genuinely fresh id (mirrors
/// `create_with_rebind_on_an_already_bound_workspace_mints_a_fresh_id_and_resets_watermarks` in
/// `crates/nxs/src/sync/mod.rs`, at the CLI seam).
#[test]
fn create_with_rebind_on_an_already_bound_workspace_mints_a_fresh_id() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--json",
        ])
        .assert()
        .success();
    let out = nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--create",
            "--rebind",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["mode"], "rebind");
    let new_id = v["stream_id"].as_str().unwrap();
    assert_ne!(new_id, "stream-one", "a fresh id was minted");
    assert!(new_id.starts_with("stream-"));
}

/// Regression guard for the self-review bug fixed during task 8 (the `switched` flag in
/// `bind()`): on a workspace that was NEVER bound, `--rebind` has nothing to switch from and is
/// just an ordinary first bind — the reported mode must be the normal `"join"`
/// (`"derived"`/`"create"` for the other flag combinations), never `"rebind"`.
#[test]
fn rebind_flag_on_a_never_bound_workspace_is_reported_as_an_ordinary_bind() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let out = nxs(tmp.path())
        .args([
            "sync",
            "bind",
            "--no-daemon",
            "--join",
            "stream-one",
            "--rebind",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v = json(out);
    assert_eq!(v["mode"], "join", "nothing to switch from — not a rebind");
    assert_eq!(v["stream_id"], "stream-one");
}
