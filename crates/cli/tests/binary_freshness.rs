//! The stale-multicall-binary gate for the flow suite (nexus-flow-0yyw).
//!
//! Every black-box test here shells out to `nxf` — an `argv[0]` symlink to `nxs`, a binary that
//! `cargo test -p nexus-flow-cli` does NOT build. Without this gate the whole package can go green
//! against an arbitrarily old `nxs`: a meaningless run that looks exactly like a meaningful one.
//!
//! This is a test target of its own so that ANY package-scoped run turns red. The `--test golden`
//! path — the one that regenerates goldens, where a stale binary does lasting damage — carries its
//! own copy of the check, since a single-target run never reaches this file.

#[test]
fn nxs_binary_is_not_stale() {
    nxs_test_support::assert_multicall_binary_fresh();
}
