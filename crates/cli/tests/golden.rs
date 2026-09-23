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

#[test]
fn golden_examples() {
    // FIRST, before trycmd touches a single case: a package-scoped run leaves `nxs` unbuilt, and
    // under TRYCMD=overwrite an old binary would rewrite the goldens — burning stale output into
    // the agent-facing docs (that is PR #241, 6j6v.01dn). Fail before anything is written.
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
        // Pin the clock too (C3 #916.4): `close`/`archive` now stamp `closed_at`/`archived` from
        // `now`, so a command that omits `--now` would otherwise capture the live wall clock and
        // make the golden flaky. A fixed instant keeps those record fields byte-stable; commands
        // that pass an explicit `--now` still override it.
        .env("NXF_NOW", "2026-06-23T00:00:00Z")
        .case("tests/golden/*.trycmd")
        .case("tests/golden/*/*.trycmd");
}
