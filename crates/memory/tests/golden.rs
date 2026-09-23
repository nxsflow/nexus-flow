//! Golden-example harness for `nxm` (mirrors flow's `nexus-flow-cli` golden): every doc example is
//! executed against the built `nxm` binary and the shown output IS the asserted output (trycmd) —
//! documentation cannot drift without turning CI red.
//!
//! Determinism: cases run with `NXF_DETERMINISTIC_IDS=1` (fixed replica `ab12`, site 1),
//! `NXM_ACTOR=alice`, and a pinned `NXM_NOW`, so init prefixes, authors, and `updated` timestamps
//! are byte-stable. Auto-keys are content hashes (already deterministic).
//!
//! Regenerate expected output after an intentional change with:
//!   TRYCMD=overwrite cargo test -p nexus-memory --test golden
//! That command does NOT rebuild `nxs` (the binary `nxm` symlinks to), so it is gated below —
//! see nexus-flow-0yyw.

#[test]
fn golden_examples() {
    // Before trycmd writes anything: under TRYCMD=overwrite a stale `nxs` would burn old output
    // into the goldens. See the flow golden harness for the full account.
    nxs_test_support::assert_multicall_binary_fresh();

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
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", "2026-06-20T10:00:00Z")
        // aye.24: point Claude-host import detection at the relative `claude-memory/` fixture. It
        // resolves per-case against the sandbox cwd, so it only fires for `import.trycmd` (whose
        // `import.in/` ships the fixture) and is inert for every other case.
        .env("NXM_CLAUDE_MEMORY_DIR", "claude-memory")
        .case("tests/golden/*.trycmd");
}
