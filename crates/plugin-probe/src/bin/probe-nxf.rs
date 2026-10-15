//! `nxf` with this crate's test-only plugins linked (review of PR #63, Test Quality #5): the real
//! `nxf` command tree, so a test can run a verb as a SUBPROCESS over a workspace whose plugin is
//! the `file-fixture` and assert what it prints. The shipped `nxf` cannot see a test-only plugin,
//! and the in-process `run_from` prints to the test's own stdout. Not shipped: this crate is never
//! a dependency of a shipped binary, and no release step builds it.

// Link the library so its plugins are in the facade registry.
extern crate plugin_probe as _;

fn main() -> std::process::ExitCode {
    // The command tree is `nxf`'s whatever this binary is called.
    nexus_flow_cli::run_from(std::iter::once("nxf".into()).chain(std::env::args_os().skip(1)))
}
