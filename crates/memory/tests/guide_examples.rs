//! Anti-drift guard for memory's narrative guides (nxf 6j6v.h4k0), the same loop flow and chat
//! closed for theirs (`crates/cli/tests/guide_examples.rs`, `crates/chat/tests/guide_examples.rs`).
//!
//! The project principle is "example = test": every command a guide prints is executed, and the
//! output it shows IS the asserted output. The executed truth lives in the golden trycmd corpus
//! (`tests/golden/**/*.trycmd`, run by `golden.rs`); the guides quote it.
//!
//! This closes the loop from the other end: every ` ```console ` block in
//! `docs/guide/{en,de}/*.md` must appear VERBATIM as a block in some golden file. So a guide can
//! never show a command or an output that is not real — and when a golden's output changes and a
//! guide is not carried along, this is what goes red instead of the documentation quietly becoming
//! a false map.
//!
//! **Both locales are checked**, and that is not symmetry for its own sake: the DE guides quote the
//! SAME blocks (the commands and their bytes are not translated — only the prose around them is),
//! so a block corrected in EN and forgotten in DE is exactly the drift this catches. Checking each
//! locale against the corpus is not enough on its own, though — a DE guide that DROPS a block EN
//! still carries breaks nothing, because what remains still matches. That is what
//! `every_topic_shows_the_same_console_blocks_in_both_locales` closes (PR #356 review, Test
//! Quality #2).
//!
//! Illustrative ` ```bash ` snippets — the install, `nxs init --module memory`, a judged
//! `migrate plan` piped to a file — are deliberately NOT checked, for flow's reason: they are shapes
//! to copy, not hermetic runs.
//!
//! # What this CANNOT see
//!
//! This gate pins OUTPUTS, and only outputs. Two blind spots follow from that, and both are worth
//! knowing before trusting a green run (PR #356 review, Test Quality #4):
//!
//! - **A CLAIM ABOUT BEHAVIOUR with no command beside it is invisible to it.** The retrieval-rule
//!   table in `core-concepts.md` is the clearest case here, because its `nxf show` / `nxf next`
//!   columns are flow's surface and no `nxm` golden can execute them. What holds those claims is a
//!   named test beside the code: `nexus_memory::model::replayed_at_session_start` and `reaches_item`
//!   carry the rule, and `facade.rs`'s `the retrieval rule (6j6v.srpg)` block exercises both halves.
//! - **The PROSE around an unchanged block can rot while the block still matches.** A paragraph
//!   that describes the output wrongly is a false map with a green suite behind it. Nothing here
//!   sees that; a reader does.
//!
//! chat's copy of this header records the full argument for why no gate was built for that class —
//! the three shapes one could take were considered and none catches what actually failed.

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
        let guides = files_with_ext(&crate_dir.join("docs/guide").join(lang), "md");
        // `files_with_ext` skips an unreadable directory rather than panicking, so without this the
        // whole check passes VACUOUSLY when a locale tree goes missing: zero guides, zero blocks,
        // zero mismatches (PR #356 review, Integrity & Robustness #3). The golden side has carried
        // the same guard from the start; this is its twin.
        assert!(
            !guides.is_empty(),
            "no {lang} guides found — the tree is missing or unreadable, and this suite would \
             otherwise pass on an empty set"
        );
        for guide in guides {
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

/// The locales must show the SAME worked examples, topic by topic (PR #356 review, Test Quality
/// #2).
///
/// `every_guide_console_block_is_a_golden_example` holds each locale against the corpus
/// INDEPENDENTLY, which is one assertion short of the rule it is there for: a DE guide that drops a
/// block its EN twin still shows, or replaces it with a different-but-also-real one, satisfies that
/// check completely. The commands and their bytes are not translated — only the prose around them
/// is — so equality of the block SEQUENCE is the actual invariant, and it is asserted here rather
/// than left to hold by coincidence.
#[test]
fn every_topic_shows_the_same_console_blocks_in_both_locales() {
    let guide_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/guide");
    let topics: Vec<String> = files_with_ext(&guide_dir.join("en"), "md")
        .iter()
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect();
    assert!(!topics.is_empty(), "no en guides found");

    for topic in topics {
        let en = console_blocks(&read(&guide_dir.join("en").join(format!("{topic}.md"))));
        let de = console_blocks(&read(&guide_dir.join("de").join(format!("{topic}.md"))));
        assert_eq!(
            en, de,
            "{topic}: the en and de guides must show the same ```console blocks, in the same order \
             — a command and its output are not translated, only the prose around them is"
        );
    }
}

/// The other direction, for the ONE corpus file that exists only to be quoted: every block in
/// `guides.trycmd` must be shown by some guide.
///
/// Not asserted for the corpus as a whole — `contract.trycmd` covers the onboarding verbs,
/// `memory.trycmd` is the product tour and `import.trycmd` the import one, and none of the three is
/// written to be quoted (though a guide may quote them, and does). But `guides.trycmd`'s whole
/// reason to exist is the guides, so a block nobody quotes is either a guide that forgot it or a
/// case that should not be there. Left un-asserted, that file would drift into a second, unread
/// test corpus.
#[test]
fn every_worked_example_is_actually_shown_in_a_guide() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let shown: BTreeSet<String> = ["en", "de"]
        .iter()
        .flat_map(|lang| files_with_ext(&crate_dir.join("docs/guide").join(lang), "md"))
        .flat_map(|p| console_blocks(&read(&p)))
        .collect();

    let unquoted: Vec<String> =
        console_blocks(&read(&crate_dir.join("tests/golden/guides.trycmd")))
            .into_iter()
            .filter(|b| !shown.contains(b))
            .collect();

    assert!(
        unquoted.is_empty(),
        "these guides.trycmd blocks are quoted by no guide — quote them or drop the case:\n\n{}",
        unquoted.join("\n---\n")
    );
}
