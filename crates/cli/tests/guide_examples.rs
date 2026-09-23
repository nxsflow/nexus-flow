//! Anti-drift guard for the narrative guides (nexus-flow-4oa.7).
//!
//! The epic principle is "Beispiel = Test": every command shown in the docs is executed and
//! the shown output IS the asserted output. The executed truth lives in the golden trycmd
//! corpus (`tests/golden/**/*.trycmd`, run by `golden.rs`). The guides quote that output.
//!
//! This test closes the loop: every ` ```console ` block printed in a guide
//! (`docs/guide/{en,de}/*.md`) must appear **verbatim** as a block in some golden file. So a
//! guide can never show a command/output that isn't a real, tested example — if a golden
//! output changes and a guide isn't updated, this fails. Illustrative ` ```bash ` snippets
//! (install, `init`, the non-hermetic `sync` path) are deliberately NOT checked.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Return the normalized text of every ` ```console ` block in `src`. Normalization strips
/// leading/trailing blank lines of each block so cosmetic trailing newlines never cause a
/// false mismatch; the command + output lines must still agree exactly.
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
        // Drop leading/trailing empty lines, keep internal structure verbatim.
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

    // The executed truth: every console block in the golden corpus.
    let golden: BTreeSet<String> = files_with_ext(&crate_dir.join("tests/golden"), "trycmd")
        .iter()
        .flat_map(|p| console_blocks(&read(p)))
        .collect();
    assert!(!golden.is_empty(), "no golden console blocks found");

    // Every console block shown in EN + DE guides must be one of them.
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
        "these guide ```console blocks are not verbatim golden examples (run the command, \
         add it to a tests/golden/**/*.trycmd case, or fix the guide):\n\n{}",
        missing.join("\n---\n")
    );
}

/// The canonical record is plugin-independent — with one sp6.4 exception. `nxf list --json`
/// (id-ordered, not ranked) is identical under both plugins' `compare` fixtures EXCEPT the `type`
/// field, which the type system makes plugin-declared and stored verbatim (issue-tracker `feature`
/// vs personal-todo `todo`). Strip that one field and the records match field for field — the
/// plugins guide states this, and asserting it here keeps the property tamper-evident.
#[test]
fn list_json_is_plugin_independent_except_the_declared_type() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let list_json = |plugin: &str| -> Vec<serde_json::Value> {
        let path = crate_dir.join(format!("tests/golden/{plugin}/compare.trycmd"));
        let block = console_blocks(&read(&path))
            .into_iter()
            .find(|b| b.lines().next() == Some("$ nxf list --json"))
            .unwrap_or_else(|| panic!("{plugin}/compare.trycmd has no `nxf list --json` block"));
        let json_line = block.lines().nth(1).expect("the json output line");
        serde_json::from_str(json_line).expect("a valid json array")
    };
    let it = list_json("issue-tracker");
    let pt = list_json("personal-todo");
    // The type genuinely differs now — plugin-declared vocabulary, not a shared core role.
    assert!(
        !it.is_empty() && it.iter().all(|i| i["type"] == "feature"),
        "issue-tracker items are typed `feature`"
    );
    assert!(
        !pt.is_empty() && pt.iter().all(|i| i["type"] == "todo"),
        "personal-todo items are typed `todo`"
    );
    // Apart from the plugin-declared `type` and the additive presentation labels (ee2h:
    // `priority_label`/`type_label` are the plugin's display vocabulary, deliberately plugin-
    // specific), the canonical record is identical across the two plugins.
    let strip_presentation = |mut items: Vec<serde_json::Value>| {
        for i in &mut items {
            let obj = i.as_object_mut().unwrap();
            obj.remove("type");
            obj.remove("priority_label");
            obj.remove("type_label");
        }
        items
    };
    assert_eq!(
        strip_presentation(it),
        strip_presentation(pt),
        "apart from the plugin-declared `type` and the additive *_label fields, the canonical \
         `nxf list --json` record is plugin-independent"
    );
}
