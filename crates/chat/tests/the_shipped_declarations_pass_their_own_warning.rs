//! The declarations this repository SHIPS are held to the warning it ships (nxf 6j6v.nzxr).
//!
//! v0.93.0 (nxf 6j6v.9w08) shipped `nxc guide writing-declarations` and the first of its three
//! checkable rules: a declaration that spells out `nxc reply` / `nxc send` / `nxc list` is copying
//! text the engine writes into the session anyway. Run over this repository's OWN examples it
//! reported twelve declarations — every persona in `role-runtime-v2` and `role-runtime-v3`, the
//! golden corpus's `coder`, and the worked example in `nxc guide personas`, which stands under a
//! sentence inviting the reader to copy it.
//!
//! **Nothing had held them to it, which is why it could happen at all.** A warning nobody runs over
//! the examples is a warning about somebody else's files; the examples are the one place a reader
//! copies from, so the rule has to bite hardest exactly there. This file is that gate, and it reads
//! the SHIPPED files rather than a fixture — the same discipline as `example_v3_valid.rs` beside
//! it: a copy reintroduced into a shipped persona fails here, loudly, naming the file.
//!
//! The guide's worked example is NOT checked here. It is Markdown, not a declaration directory, and
//! a gate that parsed prose for YAML blocks would be a second, weaker implementation of what
//! `guide_examples.rs` already reasons about. It was measured by hand for the ticket (extracted
//! into a workspace of its own: zero warnings) and it is covered by the rule the paragraph under it
//! now states.
//!
//! Pure load + check, exactly like `example_v3_valid.rs`: no subprocess, no worker, no database.

use nexus_chat::channel::load_all_channels;
use nexus_chat::declaration_quality::warn_declarations;
use nexus_chat::role::{load_all_roles, RoleDecl};
use std::path::{Path, PathBuf};

/// The declaration directories this repository ships, by the name a reader would call them. One
/// added later belongs in this list; that is the whole maintenance burden — the complete team of
/// nxf 6j6v.xjh3 was the first to be added this way, by the final review of that branch.
const SHIPPED: &[(&str, &str)] = &[
    (
        "examples/role-runtime-v2",
        "examples/role-runtime-v2/.nxs-personas",
    ),
    (
        "examples/role-runtime-v3",
        "examples/role-runtime-v3/.nxs-personas",
    ),
    (
        "tests/golden/messaging.in",
        "tests/golden/messaging.in/.nxs-personas",
    ),
    (
        "examples/a-complete-team",
        "examples/a-complete-team/.nxs-personas",
    ),
];

fn crate_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

#[test]
fn every_shipped_declaration_set_warns_about_nothing() {
    for (name, relative) in SHIPPED {
        let dir = crate_path(relative);
        let roles = load_all_roles(&dir).unwrap_or_else(|e| panic!("loading {name}'s roles: {e}"));
        assert!(!roles.is_empty(), "{name} declares no personas at all");
        // `load_all_channels` answers with an empty list where there is no `channels.yaml`, which
        // is a real shape rather than a missing file — `role-runtime-v2` is the set in this list
        // that has none. Every other set declares channels of its own, so both halves of
        // `warn_declarations` are exercised across the list.
        let channels =
            load_all_channels(&dir).unwrap_or_else(|e| panic!("loading {name}'s channels: {e}"));
        let warnings = warn_declarations(&roles, &channels);
        assert!(
            warnings.is_empty(),
            "{name} trips the warning it ships — {} finding(s):\n{}",
            warnings.len(),
            warnings
                .iter()
                .map(|w| format!("  {}: {}\n", w.file, w.what))
                .collect::<String>()
        );
    }
}

/// Proof the assertion above is a real gate and not a tautology: the detector it calls is the same
/// one that reported these very files, so a declaration written the way they were written must
/// still trip it. Shaped like what was actually removed — a "How you finish" section rebuilding the
/// engine's own answering rules, with a placeholder where the engine puts the real thread id.
#[test]
fn a_declaration_written_the_old_way_still_trips_it() {
    let before: RoleDecl = serde_yaml::from_str(
        "handle: coder\n\
         job_description: Implements a work order on a branch and merges it.\n\
         system_prompt: |\n  \
         You are the coder. Do what the trigger message asks.\n\n  \
         ## How you finish\n\n  \
         nxc reply --thread <the thread id in your trigger message> - <<'EOF'\n  \
         <what you did>\n  \
         EOF\n",
    )
    .expect("the fixture parses as a role");
    let warnings = warn_declarations(std::slice::from_ref(&before), &[]);
    assert_eq!(
        warnings.len(),
        1,
        "the pre-6j6v.nzxr shape must still be reported: {warnings:?}"
    );
    assert!(
        warnings[0].what.contains("`nxc reply`"),
        "and it must name the invocation it copied: {:?}",
        warnings[0].what
    );
}
