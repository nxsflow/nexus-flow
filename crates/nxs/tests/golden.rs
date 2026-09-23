//! Golden-example harness for the `nxs` umbrella verbs (nexus-flow-aye.33). Each block is executed
//! against the built `nxs` binary (which shells out to the sibling `nxf`/`nxm`), and the shown
//! output IS the asserted output (trycmd) — the docs cannot lie without turning CI red.
//!
//! Determinism: `NXF_DETERMINISTIC_IDS=1` propagates to the driven `nxf`/`nxm` children, so the
//! workspace takes a fixed identity (`ab12` / site 1); `NXS_NOW` pins the prime fan-out's shared
//! clock. The `--json` records carry a declared field order and no wall-clock, so they are
//! byte-stable; workspace paths use trycmd's `[CWD]` redaction.
//!
//! Regenerate expected output after an intentional change with:
//!   TRYCMD=overwrite cargo test -p nxs --test golden

#[test]
fn golden_examples() {
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
        .env("NXS_NOW", "2026-06-22T12:00:00Z")
        .case("tests/golden/*.trycmd");
}
