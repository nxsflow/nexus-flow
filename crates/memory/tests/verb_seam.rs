//! nxm's half of the verb-seam omission gate (nexus-flow-6j6v.vtvs).
//!
//! `crates/memory/tests/parity.rs` next door drives the same script through the CLI and the
//! [`Engine`](nexus_memory::engine::Engine) and compares op logs and read results byte for byte. It
//! is blind to a verb that exists on only one side — which is how `nxm prime` assembled its
//! session-start prose inside `cli.rs` for a year with that suite green. This gate compares the two
//! verb LISTS instead; both are derived (the `clap` tree, the seam sources), and only the
//! difference is declared below.
//!
//! See [`nxs_test_support::verb_seam`] for what each waiver kind claims and how the gate holds it
//! to that claim.

use nxs_test_support::verb_seam::{assert_verb_seam, crate_relative, Waiver};

/// nxm's declared difference between the two verb lists. Every entry carries its reason: an
/// exception without one is a finding, not a free pass.
const WAIVERS: &[Waiver] = &[
    Waiver::CliOnly {
        verb: "init",
        reason:
            "Sets up the HOST, not the workspace: ensures `.nxs/`, registers the memory module \
                 and delegates the shared agent file + the SessionStart hook to the `nxs` \
                 assembler. An embedding app is installed by its own host; it does not bootstrap \
                 a checkout. Epic-sanctioned as CLI-own (6j6v.fjrc).",
    },
    Waiver::CliOnly {
        verb: "agent-manifest",
        reason: "Emits nxm's DECLARED contribution to the shared agent file (spec §6.2) — the \
                 data the `nxs` umbrella assembles from. It is about the files on disk around the \
                 workspace, not about memories, and its only consumer is the umbrella running the \
                 binary. Epic-sanctioned as CLI-own (6j6v.fjrc).",
    },
    Waiver::CliOnly {
        verb: "import",
        reason: "A host-filesystem on-ramp (aye.24): it auto-DETECTS the Claude memory directory \
                 from the host's project/home layout and parses markdown files off it. Both halves \
                 are already library-reachable on their own — `nexus_memory::import::{detect, \
                 scan, parse_fact}` are public and the write goes through `remember`'s own core — \
                 so what stays CLI-own is the host discovery, which is what makes it an import \
                 path. Epic-sanctioned as CLI-own (6j6v.fjrc).",
    },
    Waiver::CliOnly {
        verb: "guide",
        reason: "Renders the CLI's own manual — markdown baked into the binary via `include_dir!` \
                 (`crates/memory/docs/guide/en`), written about typing `nxm …`. An embedding app has \
                 no `nxm` to explain and ships its own documentation; there is no workspace \
                 capability behind this verb to lift. Same shape as flow's, and the same shared \
                 mechanism (`nxs-guide`, 6j6v.9e3r).",
    },
];

#[test]
fn every_nxm_verb_is_reachable_from_the_library_seam() {
    // The seam an embedding app links: the long-lived handle plus the compute layer under it. The
    // store below them is deliberately NOT seam — reaching past the facade into the store is the
    // very thing E5c calls a violation, so a capability that exists only there is an omission.
    let sources = ["src/engine.rs", "src/facade.rs"]
        .map(|rel| crate_relative(env!("CARGO_MANIFEST_DIR"), rel))
        .to_vec();
    assert_verb_seam("nxm", &nexus_memory::cli::command(), &sources, WAIVERS);
}
