//! The output contract (RFC 0001, "Reading and writing"): JSON by default and Markdown with
//! `--md`, and the same local copy gives the same bytes. The golden files hold the bytes of a
//! repository whose `nexus-flow.toml` already names a space, and which has no factory and no
//! CODEOWNERS file yet: `nxs init` writes the factory and GENESIS, so the second run is in a
//! second repository of the same state.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{nxs, scratch, stdout};

const FILE: &str = "format = 1\n\n[space]\nrepository = \"0123456789abcdef0123456789abcdef\"\n";

fn golden(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// A repository whose `nexus-flow.toml` names the fixed space above.
fn repository(test: &str) -> (PathBuf, PathBuf) {
    let base = scratch(test);
    let repo = base.join("repo");
    fs::write(repo.join("nexus-flow.toml"), FILE).unwrap();
    (base, repo)
}

#[test]
fn json_by_default_matches_the_golden_file_every_time() {
    let (base, repo) = repository("golden-json");
    let (other, again) = repository("golden-json-again");

    let first = nxs(&base, &repo, &["init"]);
    let second = nxs(&other, &again, &["init"]);

    assert_eq!(first.code, 0, "{}", stdout(&first));
    assert_eq!(stdout(&first), golden("init.json"));
    assert_eq!(second.stdout, first.stdout);
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(&other).unwrap();
}

#[test]
fn markdown_with_md_matches_the_golden_file_every_time() {
    let (base, repo) = repository("golden-md");
    let (other, again) = repository("golden-md-again");

    let first = nxs(&base, &repo, &["init", "--md"]);
    let second = nxs(&other, &again, &["--md", "init"]);

    assert_eq!(first.code, 0, "{}", stdout(&first));
    assert_eq!(stdout(&first), golden("init.md"));
    assert_eq!(second.stdout, first.stdout);
    fs::remove_dir_all(&base).unwrap();
    fs::remove_dir_all(&other).unwrap();
}
