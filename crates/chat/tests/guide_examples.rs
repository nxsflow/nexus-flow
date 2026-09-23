//! Anti-drift guard for chat's narrative guides (nxf 6j6v.t6vd), the same loop flow closed for its
//! own guides in `crates/cli/tests/guide_examples.rs`.
//!
//! The project principle is "example = test": every command a guide prints is executed, and the
//! output it shows IS the asserted output. The executed truth lives in the golden trycmd corpus
//! (`tests/golden/**/*.trycmd`, run by `golden.rs`); the guides quote it.
//!
//! This closes the loop from the other end: every ` ```console ` block in
//! `docs/guide/{en,de}/*.md` must appear VERBATIM as a block in some golden file. So a guide can
//! never show a command or an output that is not real and tested — and when a golden's output
//! changes and a guide is not carried along, this is what goes red instead of the documentation
//! quietly becoming a false map.
//!
//! **Both locales are checked**, and that is not symmetry for its own sake: the DE guides quote the
//! SAME blocks (the commands and their bytes are not translated — only the prose around them is),
//! so a block corrected in EN and forgotten in DE is exactly the drift this catches.
//!
//! Illustrative ` ```bash ` and ` ```yaml ` snippets — install, `nxc init`, a declaration file —
//! are deliberately NOT checked, for flow's reason: they are shapes to copy, not hermetic runs.
//!
//! # What this CANNOT see, written down rather than left to be rediscovered (nxf 6j6v.mg5b)
//!
//! This gate pins OUTPUTS. A guide sentence that makes a CLAIM ABOUT BEHAVIOUR with no command
//! beside it is invisible to it, and that is not a hypothetical gap: `personas.md` said "every
//! persona's turn currently starts a fresh session" for as long as `reply --thread` had been
//! resuming them, this suite green throughout. It cost an agent in an external testbed five
//! persona declarations built on a memory ritual against an amnesia that does not exist — and it
//! was found by READING, which is the one mechanism that reaches this class.
//!
//! **No gate was built for it, deliberately.** The three shapes a gate could take were considered
//! and none catches what actually failed:
//!
//! - Pin the guide's inert-field LIST against a list in code. The list was correct here; the
//!   field really is inert. What was wrong was the REASON beside it — free prose, and the half a
//!   derived list cannot carry.
//! - Execute the claim as a ` ```console ` block. That is this suite, and it already covers every
//!   sentence that HAS a command. A sentence about what a session carries across a turn has none
//!   to write that would not be a whole scripted scenario in a guide.
//! - Assert truth of English. Nothing does this.
//!
//! What DOES hold each claim is a named test beside the code it describes, and a reader who wants
//! the rule follows it there: `orchestration_reply.rs`'s
//! `a_reply_resumes_the_targets_own_return_address_session` for the resume, `role.rs`'s
//! `compose_override_drops_claude_md_exactly_as_ignore_does` for the one half-read field in that
//! same section. Building a list-parity gate on suspicion would have reddened on nothing and
//! trained a reader to believe the section is machine-checked, which is worse than knowing it is
//! not. The mechanism for the neighbouring class — a term whose IDENTIFIER is gone while the prose
//! naming it rots — is `xtask/sweep-retired-vocabulary.sh`, and it is a review aid for the same
//! reason: no rule tells a stale claim from an honest historical one.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The normalized text of every ` ```console ` block in `src`. Leading/trailing blank lines of a
/// block are stripped so a cosmetic newline never causes a false mismatch; the command and output
/// lines must still agree exactly.
fn console_blocks(src: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = src.lines();
    while let Some(line) = lines.next() {
        if line.trim_end() != "```console" {
            continue;
        }
        let mut body: Vec<&str> = Vec::new();
        for inner in lines.by_ref() {
            if inner.trim_end() == "```" {
                break;
            }
            body.push(inner);
        }
        while body.first().is_some_and(|l| l.trim().is_empty()) {
            body.remove(0);
        }
        while body.last().is_some_and(|l| l.trim().is_empty()) {
            body.pop();
        }
        blocks.push(body.join("\n"));
    }
    blocks
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// All files matching `<dir>/**/*.<ext>` (recursive), sorted for determinism.
fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_guide_console_block_is_a_golden_example() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    let golden: BTreeSet<String> = files_with_ext(&crate_dir.join("tests/golden"), "trycmd")
        .iter()
        .flat_map(|p| console_blocks(&read(p)))
        .collect();
    assert!(!golden.is_empty(), "no golden console blocks found");

    let mut missing = Vec::new();
    for lang in ["en", "de"] {
        for guide in files_with_ext(&crate_dir.join("docs/guide").join(lang), "md") {
            for block in console_blocks(&read(&guide)) {
                if !golden.contains(&block) {
                    missing.push(format!("{}:\n{block}\n", guide.display()));
                }
            }
        }
    }

    assert!(
        missing.is_empty(),
        "these guide ```console blocks are not verbatim golden examples (run the command, add it \
         to a tests/golden/**/*.trycmd case, or fix the guide):\n\n{}",
        missing.join("\n---\n")
    );
}

/// The other direction, for the ONE corpus file that exists only to be quoted: every block in
/// `messaging.trycmd` must be shown by some guide.
///
/// Not asserted for the corpus as a whole — `help.trycmd` is a map of every `--help` page and
/// `contract.trycmd` covers the onboarding verbs, and neither is written to be quoted. But
/// `messaging.trycmd`'s whole reason to exist is the guides, so a block nobody quotes is either a
/// guide that forgot it or a case that should not be there. Left un-asserted, that file would drift
/// into a second, unread test corpus.
#[test]
fn every_worked_example_is_actually_shown_in_a_guide() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let shown: BTreeSet<String> = ["en", "de"]
        .iter()
        .flat_map(|lang| files_with_ext(&crate_dir.join("docs/guide").join(lang), "md"))
        .flat_map(|p| console_blocks(&read(&p)))
        .collect();

    let unquoted: Vec<String> =
        console_blocks(&read(&crate_dir.join("tests/golden/messaging.trycmd")))
            .into_iter()
            .filter(|b| !shown.contains(b))
            .collect();

    assert!(
        unquoted.is_empty(),
        "these messaging.trycmd blocks are quoted by no guide — quote them or drop the case:\n\n{}",
        unquoted.join("\n---\n")
    );
}
