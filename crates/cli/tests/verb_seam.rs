//! nxf's half of the verb-seam omission gate (nexus-flow-6j6v.vtvs).
//!
//! `crates/cli/tests/parity.rs` next door drives the same mutation script through the CLI and the
//! [`Engine`](nexus_flow_facade::engine::Engine) and compares op logs and read results byte for
//! byte. It is blind to a verb that exists on only one side. This gate compares the two verb LISTS
//! instead; both are derived (the `clap` tree, the seam sources), and only the difference is
//! declared below.
//!
//! flow is the module the epic calls exemplary, and the table shows why: nothing here is a gap.
//! Every verb is either the command line's own business or on the seam under a name of its own.
//!
//! See [`nxs_test_support::verb_seam`] for what each waiver kind claims and how the gate holds it
//! to that claim.

use nxs_test_support::verb_seam::{assert_verb_seam, crate_relative, Waiver};

/// nxf's declared difference between the two verb lists. Every entry carries its reason: an
/// exception without one is a finding, not a free pass.
const WAIVERS: &[Waiver] = &[
    // ---- the command line's own business -----------------------------------
    Waiver::CliOnly {
        verb: "init",
        reason: "Sets up the HOST, not the workspace: creates `.nxs/`, picks the plugin \
                 (interactively on a TTY) and delegates the shared agent file + the SessionStart \
                 hook to the `nxs` assembler. An embedding app is installed by its own host; it \
                 does not bootstrap a checkout. Epic-sanctioned as CLI-own (6j6v.fjrc).",
    },
    Waiver::CliOnly {
        verb: "agent-manifest",
        reason: "Emits flow's DECLARED contribution to the shared agent file (spec §6.2) — the \
                 data the `nxs` umbrella assembles from. It is about the files on disk around the \
                 workspace, not about items, and its only consumer is the umbrella running the \
                 binary. Epic-sanctioned as CLI-own (6j6v.fjrc).",
    },
    Waiver::CliOnly {
        verb: "self-update",
        reason: "Replaces the INSTALLED BINARY on the host. A hidden, deprecated alias for \
                 `nxs self-update` (nexus-flow-gel) at that. An app updates through its own \
                 packaging, and a library that could overwrite the executable running it would be \
                 a liability, not a capability. Epic-sanctioned as CLI-own (6j6v.fjrc).",
    },
    Waiver::CliOnly {
        verb: "verify-signature",
        reason:
            "The other half of `self-update`: it checks a minisign signature over a downloaded \
                 release artifact on disk. Same host-not-workspace argument, same trust chain — \
                 waiving the update path and not its verification step would be an odd place to \
                 draw the line.",
    },
    Waiver::CliOnly {
        verb: "setup claude",
        reason: "Mutates the USER'S HOST CONFIG (the Claude Code settings that wire up the \
                 SessionStart hook) — state that lives outside the workspace and outside this \
                 process's lifetime. `cargo xtask docs coverage` exempts it from the golden \
                 corpus for the same reason: it cannot be reproduced in a workspace-scoped run.",
    },
    Waiver::CliOnly {
        verb: "guide",
        reason: "Renders the CLI's own manual — markdown baked into the binary via `include_dir!` \
                 (`crates/cli/docs/guide/en`), written about typing `nxf …`. An embedding app has \
                 no `nxf` to explain and ships its own documentation; there is no workspace \
                 capability behind this verb to lift.",
    },
    // ---- on the seam, under a name of its own -------------------------------
    Waiver::Alias {
        verb: "label list",
        seam: "labels",
        reason: "`read::labels` is the same read, named for the collection rather than the verb \
                 path (`Engine::labels` hands it to an app, plus `labels_bulk` for many items).",
    },
    Waiver::Alias {
        verb: "mention list",
        seam: "mentions",
        reason: "`read::mentions` is what the verb body calls; the CLI only renders it.",
    },
    Waiver::Alias {
        verb: "note list",
        seam: "notes",
        reason: "`read::notes` / `read::notes_with_created` — the verb body calls the latter, and \
                 both are on the seam.",
    },
    Waiver::Alias {
        verb: "thread list",
        seam: "thread_items",
        reason: "`read::thread_items` is what the verb body calls (`thread_links` is the \
                 item→thread direction of the same edge set).",
    },
    Waiver::Alias {
        verb: "thread link",
        seam: "thread_link_add",
        reason: "`write::thread_link_add` — the seam names the edge and the direction, the CLI \
                 names the act. The verb body is a call to it.",
    },
    Waiver::Alias {
        verb: "thread unlink",
        seam: "thread_link_remove",
        reason: "`write::thread_link_remove`, the observed-remove other half of `thread link`.",
    },
    Waiver::Alias {
        verb: "contributes list",
        seam: "show",
        reason: "The WEAKEST alias in this table, and recorded as what it is: there is no \
                 dedicated read, but `read::show`'s `ShowRecord.contributes_to` carries exactly \
                 the edge set this verb prints, so an app reads it without going past the facade. \
                 A dedicated `read::contributes_to` would be tidier; it is not an omission.",
    },
];

#[test]
fn every_nxf_verb_is_reachable_from_the_library_seam() {
    // The seam an embedding app links: the long-lived handle plus the compute layer under it —
    // flow's facade IS `read`/`write`, with `Engine` the concurrency-safe wrapper over them. The
    // core store below is deliberately NOT seam: reaching past the facade is the thing E5 calls a
    // violation, so a capability that exists only there is an omission.
    let sources = [
        "../facade/src/engine.rs",
        "../facade/src/read.rs",
        "../facade/src/write.rs",
    ]
    .map(|rel| crate_relative(env!("CARGO_MANIFEST_DIR"), rel))
    .to_vec();
    assert_verb_seam("nxf", &nexus_flow_cli::command(), &sources, WAIVERS);
}
