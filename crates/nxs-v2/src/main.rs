//! v2's `nxs` binary on the library seam (RFC 0001), built as `nxs-v2` until the cut-over (NEX-194) names it `nxs`.

use std::io::Write;
use std::path::PathBuf;

use nexus_flow_seam::Options;

fn main() {
    let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let outcome = nxs_v2::run(std::env::args_os(), &dir, Options::from_env);
    let _ = std::io::stdout().write_all(&outcome.stdout);
    let _ = std::io::stderr().write_all(&outcome.stderr);
    std::process::exit(outcome.code);
}
