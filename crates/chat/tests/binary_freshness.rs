//! The stale-multicall-binary gate for the chat suite (nexus-flow-0yyw).
//!
//! `nxc` is an `argv[0]` symlink to `nxs`, which `cargo test -p nexus-chat` does not build. See
//! `crates/cli/tests/binary_freshness.rs` for the full account.

#[test]
fn nxs_binary_is_not_stale() {
    nxs_test_support::assert_multicall_binary_fresh();
}
