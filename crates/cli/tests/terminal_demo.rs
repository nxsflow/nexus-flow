//! Generator + drift guard for the landing terminal demo (p2dw; content provider 6j6v.0a6p).
//!
//! The federated `/open-source` terminal-demo section (content.json, assembled by the content
//! provider) claims to show *"real output captured from `nxf`"*. Before this it was a
//! hand-maintained transcript that silently drifted from the CLI on every change (bare-id
//! rendering, the issue-tracker type vocabulary, the `next` framing), while the caption kept
//! promising it was real.
//!
//! This test makes the promise enforceable, in the same spirit as the golden-example harness
//! ("Beispiel = Test — documentation cannot lie without turning CI red"): it drives the SAME
//! commands the demo shows against the built `nxf`, captures their real stdout, and holds the
//! committed fixture the provider assembles into content.json
//! (`content/data/terminal-demo.json`) byte-for-byte against that capture. If the CLI's output
//! drifts, this test goes red until the fixture is regenerated — which, because the provider
//! reads the very same file, also refreshes the demo. The transcript can no longer be
//! stale-but-green.
//!
//! Determinism mirrors the golden harness: `NXF_DETERMINISTIC_IDS=1` fixes the workspace identity
//! (`ab12.…`) and makes `create` mint sequential ids; the canonical `--json` record carries no
//! timestamps, and the human `next`/`blocked` views are byte-stable plain (the Theme is TTY-gated).
//! A pinned `NXF_NOW` keeps any close/stamp deterministic.
//!
//! Regenerate after an intentional CLI-output change:
//!   UPDATE_DEMO=1 cargo test -p nexus-flow-cli --test terminal_demo

use assert_cmd::Command;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A shown command and the exact stdout `nxf` produced for it. The landing splits `out` into lines
/// for rendering, so the framing (the blank lines `next`/`blocked` print around their body) is
/// preserved verbatim — that framing is part of the "real output" the caption promises.
#[derive(Serialize)]
struct Block {
    cmd: String,
    out: String,
}

/// The committed fixture the provider assembles into content.json. `_note` self-documents the file
/// (JSON has no comments); `session` is the ordered transcript.
#[derive(Serialize)]
struct Demo {
    #[serde(rename = "_note")]
    note: &'static str,
    session: Vec<Block>,
}

const NOTE: &str =
    "Generated and drift-guarded by crates/cli/tests/terminal_demo.rs — do not edit \
by hand. Regenerate with `UPDATE_DEMO=1 cargo test -p nexus-flow-cli --test terminal_demo`.";

fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXF_NOW", "2026-06-23T00:00:00Z")
        .current_dir(dir);
    c
}

/// Run `nxf <args>` in `dir`, require success, return stdout as a String.
fn stdout(dir: &Path, args: &[&str]) -> String {
    let out = nxf(dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("nxf stdout is utf-8")
}

fn fixture_path() -> PathBuf {
    // tests/terminal_demo.rs -> crates/cli -> repo root -> content/data/…
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/data/terminal-demo.json")
}

/// Seed a small, realistic board, then capture the read commands the landing page shows.
///
/// Story (deterministic ids): an epic with two children, one of which depends on the other, so the
/// board exercises the whole pitch in one screen — `next` ranks the ready work, `blocked` shows the
/// dependent held back by the graph, and `--json` is the same query as the machine contract.
#[test]
fn terminal_demo_matches_real_nxf_output() {
    let tmp = TempDir::new().expect("tempdir");
    let dir = tmp.path();

    // --- setup (not shown in the demo) ---
    nxf(dir)
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
    // ab12.0001 — the epic
    create(
        dir,
        &[
            "create",
            "--type",
            "epic",
            "--title",
            "Payments revamp",
            "--description",
            "Rework the payments stack for v2.",
            "--priority",
            "P1",
        ],
    );
    // ab12.0002 — a feature under the epic
    create(
        dir,
        &[
            "create",
            "--type",
            "feature",
            "--title",
            "Design new checkout flow",
            "--description",
            "One-page checkout with saved cards.",
            "--priority",
            "P1",
            "--parent",
            "ab12.0001",
        ],
    );
    // ab12.0003 — a chore under the epic, blocked until the checkout flow is designed
    create(
        dir,
        &[
            "create",
            "--type",
            "chore",
            "--title",
            "Migrate billing webhooks",
            "--description",
            "Move webhooks to the new endpoint.",
            "--priority",
            "P2",
            "--parent",
            "ab12.0001",
            "--depends-on",
            "ab12.0002",
        ],
    );

    // --- shown in the demo: real reads over the derived graph ---
    let session = vec![
        block(dir, &["next"]),
        block(dir, &["blocked"]),
        block(dir, &["next", "--json"]),
    ];

    let demo = Demo {
        note: NOTE,
        session,
    };
    let mut rendered = serde_json::to_string_pretty(&demo).expect("serialize demo");
    rendered.push('\n');

    let path = fixture_path();
    if std::env::var_os("UPDATE_DEMO").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).expect("mkdir generated/");
        std::fs::write(&path, &rendered).expect("write fixture");
        eprintln!("wrote {}", path.display());
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}). Generate it with `UPDATE_DEMO=1 cargo test -p nexus-flow-cli \
             --test terminal_demo`",
            path.display()
        )
    });
    assert_eq!(
        committed, rendered,
        "terminal-demo.json is stale vs real `nxf` output — the /open-source demo drifted. \
         Regenerate with `UPDATE_DEMO=1 cargo test -p nexus-flow-cli --test terminal_demo`."
    );
}

/// Create an item via `--json` and assert success (id is deterministic, so callers reference it by
/// its known `ab12.000N`). Kept minimal — the demo asserts the *reads*, not the setup writes.
fn create(dir: &Path, base_args: &[&str]) {
    let mut args = base_args.to_vec();
    args.push("--json");
    nxf(dir).args(&args).assert().success();
}

/// Capture one shown command as a `Block`.
fn block(dir: &Path, args: &[&str]) -> Block {
    Block {
        cmd: format!("nxf {}", args.join(" ")),
        out: stdout(dir, args),
    }
}
