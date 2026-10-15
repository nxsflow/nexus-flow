//! Golden-example harness (nexus-flow-4oa.2): every doc example block is executed against the
//! built `nxf` binary, and the shown output IS the asserted output (trycmd). The epic principle
//! "Beispiel = Test" — documentation cannot lie without turning CI red.
//!
//! Determinism: cases run with `NXF_DETERMINISTIC_IDS=1`, so a fresh workspace takes a fixed
//! identity (`ab12.…`) and `create` mints sequential ids. The canonical `--json` record carries no
//! *stored* timestamps; the op-log-derived `created_at`/`updated_at` read-layer keys (6j6v.2kjy) are
//! byte-stable because `NXF_NOW` pins the wall clock they fold from (a command's explicit `--now`
//! overrides it, exactly as for `closed_at`/`archived`). The ONE inherently-random token is a note's
//! op-id (a CRDT identity we never seed); those lines use trycmd's `[..]` wildcard.
//!
//! Layout: `tests/golden/<plugin>/*.trycmd` are the per-plugin workflows (each seeds its own
//! sandbox via `init`); `tests/golden/*.trycmd` are plugin-agnostic. `cargo xtask docs coverage`
//! enforces that every non-hidden subcommand appears here under BOTH plugin fixtures.
//!
//! Regenerate expected output after an intentional change with:
//!   TRYCMD=overwrite cargo test -p nexus-flow-cli --test golden
//! That command does NOT rebuild `nxs` (the binary `nxf` symlinks to), so it is gated below —
//! see nexus-flow-0yyw.

/// Every `.trycmd` case under `dir`, recursively.
fn trycmd_cases(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("the golden corpus is readable") {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            trycmd_cases(&path, out);
        } else if path.extension().is_some_and(|e| e == "trycmd") {
            out.push(path);
        }
    }
}

/// Every case runs in a sandbox — a temporary copy of `<name>.in/`, outside the repository — which
/// trycmd makes only when `<name>.out/` exists too. Without both, a case runs in `<name>.in/` or in
/// this crate's directory, inside the repository, and `nxf` walks up from there to the
/// repository's own `.nxs/` board: a case's `nxf init`/`create` then writes to the real board
/// (review of PR #63, Test Quality #3 — it happened once). So a case without its sandbox fails
/// here — and [`golden_examples`] asserts it before trycmd runs anything, so the case never runs.
fn assert_every_case_runs_in_a_sandbox() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut cases = Vec::new();
    trycmd_cases(&root, &mut cases);
    assert!(!cases.is_empty(), "no golden case found under {root:?}");
    let unsandboxed: Vec<String> = cases
        .iter()
        .filter(|case| {
            let dir = |ext: &str| case.with_extension(ext).is_dir();
            !(dir("in") && dir("out"))
        })
        .map(|case| case.display().to_string())
        .collect();
    assert!(
        unsandboxed.is_empty(),
        "every golden case needs a sandbox: add `<name>.in/.keep` and `<name>.out/.keep` beside \
         {unsandboxed:?}"
    );
}

#[test]
fn every_golden_case_runs_in_a_sandbox() {
    assert_every_case_runs_in_a_sandbox();
}

#[test]
fn golden_examples() {
    // FIRST, before trycmd touches a single case: a package-scoped run leaves `nxs` unbuilt, and
    // under TRYCMD=overwrite an old binary would rewrite the goldens — burning stale output into
    // the agent-facing docs (that is PR #241, 6j6v.01dn). Fail before anything is written.
    nxs_test_support::assert_multicall_binary_fresh();
    // And no case may run inside the repository, where `nxf` would find the real board.
    assert_every_case_runs_in_a_sandbox();

    // Never this machine's own `$HOME` (nxf 6j6v.9bjv). A golden corpus asserts stdout AND stderr
    // byte for byte, so the service report every invocation makes when the host's program alias is
    // dangling (nxf 6j6v.dcpk) fails every case — and under `TRYCMD=overwrite` is burned into the
    // committed goldens, developer paths and all. See `pinned_home_env` for why each pair is the
    // value an unset variable resolves to anyway.
    let cases = trycmd::TestCases::new();
    for (key, value) in nxs_test_support::pinned_home_env() {
        cases.env(key, value);
    }
    cases
        .env("NXF_DETERMINISTIC_IDS", "1")
        // Pin the clock too (C3 #916.4): `close`/`archive` now stamp `closed_at`/`archived` from
        // `now`, so a command that omits `--now` would otherwise capture the live wall clock and
        // make the golden flaky. A fixed instant keeps those record fields byte-stable; commands
        // that pass an explicit `--now` still override it.
        .env("NXF_NOW", "2026-06-23T00:00:00Z")
        .case("tests/golden/*.trycmd")
        .case("tests/golden/*/*.trycmd");
}
