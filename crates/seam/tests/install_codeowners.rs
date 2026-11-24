//! CODEOWNERS is reported, never written (RFC 0006, "Installing the shipped defaults"; RFC 0004):
//! the file is found where GitHub looks for it, in GitHub's order, and a path is covered when the
//! last rule matching it, with GitHub's patterns, names an owner.

use std::fs;
use std::path::{Path, PathBuf};

use nexus_flow_seam::install::codeowners::{find, find_in, report, Codeowners, Report};

// --- the matcher against GitHub's documented patterns ------------------------------------------

/// Pattern, path, and whether GitHub's CODEOWNERS matches the path with the pattern. The rows
/// follow GitHub's documentation, "About code owners", "Example of a CODEOWNERS file".
const PATTERNS: &[(&str, &str, bool)] = &[
    // `*` owns every file.
    ("*", "README.md", true),
    ("*", "factory/roles/pm.md", true),
    ("*", ".github/workflows/ci.yml", true),
    // `*.js` owns every .js file, at any depth.
    ("*.js", "app.js", true),
    ("*.js", "src/web/app.js", true),
    ("*.js", "app.jsx", false),
    ("*.js", "app.ts", false),
    // `*.go` with an email owner, at any depth.
    ("*.go", "cmd/main.go", true),
    // `/build/logs/` owns the directory at the root and everything beneath it.
    ("/build/logs/", "build/logs/out.txt", true),
    ("/build/logs/", "build/logs/2026/11/out.txt", true),
    ("/build/logs/", "src/build/logs/out.txt", false),
    ("/build/logs/", "build/logs", false),
    // `docs/*` owns files directly in docs/, not further nested ones.
    ("docs/*", "docs/getting-started.md", true),
    ("docs/*", "docs/build-app/troubleshooting.md", false),
    ("docs/*", "src/docs/getting-started.md", false),
    // `apps/` owns any file in an apps directory anywhere.
    ("apps/", "apps/web/index.js", true),
    ("apps/", "src/apps/web/index.js", true),
    ("apps/", "apps", false),
    ("apps/", "myapps/index.js", false),
    // `/docs/` owns the root docs directory and its subdirectories.
    ("/docs/", "docs/a.md", true),
    ("/docs/", "docs/build-app/troubleshooting.md", true),
    ("/docs/", "src/docs/a.md", false),
    // `/scripts/` owns the root scripts directory.
    ("/scripts/", "scripts/release.sh", true),
    ("/scripts/", "tools/scripts/release.sh", false),
    // `**/logs` owns files in any logs directory.
    ("**/logs", "build/logs/out.txt", true),
    ("**/logs", "scripts/logs/out.txt", true),
    ("**/logs", "deeply/nested/logs/out.txt", true),
    ("**/logs", "logs/out.txt", true),
    ("**/logs", "logsfile.txt", false),
    ("**/logs", "build/catalogs/out.txt", false),
    // `/apps/github` owns that subdirectory, and the file of that name.
    ("/apps/github", "apps/github/index.js", true),
    ("/apps/github", "apps/github", true),
    ("/apps/github", "apps/githubber/index.js", false),
    ("/apps/github", "src/apps/github/index.js", false),
    // A plain name without a slash matches at any depth, file or directory.
    ("Cargo.toml", "Cargo.toml", true),
    ("Cargo.toml", "crates/seam/Cargo.toml", true),
    ("roles", "factory/roles/pm.md", true),
    // A leading slash anchors a file to the root.
    ("/Cargo.toml", "Cargo.toml", true),
    ("/Cargo.toml", "crates/seam/Cargo.toml", false),
    // A slash in the middle anchors the pattern to the root too.
    ("factory/roles/", "factory/roles/pm.md", true),
    ("factory/roles/", "x/factory/roles/pm.md", false),
    // `/**/` matches zero or more directories; a trailing `/**` everything inside.
    ("/factory/**/pm.md", "factory/pm.md", true),
    ("/factory/**/pm.md", "factory/roles/pm.md", true),
    ("/factory/**/pm.md", "factory/a/b/pm.md", true),
    ("/factory/**", "factory/roles/pm.md", true),
    ("/factory/**", "factory/FACTORY.md", true),
    ("/factory/**", "other/FACTORY.md", false),
    // `?` matches one character, never a slash.
    ("/factory/roles/p?.md", "factory/roles/pm.md", true),
    ("/factory/roles/p?.md", "factory/roles/pmm.md", false),
    ("/factory/roles?pm.md", "factory/roles/pm.md", false),
    // `*` never crosses a slash.
    ("/factory/*.md", "factory/FACTORY.md", true),
    ("/factory/*.md", "factory/roles/pm.md", false),
    ("/factory/*/pm.md", "factory/roles/pm.md", true),
    // Paths are case sensitive.
    ("/factory/roles/", "Factory/roles/pm.md", false),
    ("*.MD", "factory/FACTORY.md", false),
];

#[test]
fn the_matcher_agrees_with_githubs_documented_patterns() {
    let mut wrong = Vec::new();
    for &(pattern, path, expected) in PATTERNS {
        let owners = Codeowners::parse(&format!("{pattern} @octocat\n"));
        if owners.covers(path) != expected {
            wrong.push(format!("{pattern} on {path}: expected {expected}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_last_matching_rule_wins() {
    let owners = Codeowners::parse("/apps/ @octocat\n/apps/github @doctocat\n");
    assert_eq!(
        owners.owners("apps/github/x.js"),
        Some(&["@doctocat".to_string()][..])
    );
    assert_eq!(
        owners.owners("apps/web/x.js"),
        Some(&["@octocat".to_string()][..])
    );
    assert_eq!(owners.owners("lib/x.js"), None);
}

#[test]
fn a_last_matching_rule_without_owners_leaves_the_path_uncovered() {
    let owners = Codeowners::parse("/apps/ @octocat\n/apps/github\n");
    assert!(!owners.covers("apps/github/index.js"));
    assert!(owners.covers("apps/web/index.js"));
}

#[test]
fn comments_and_blank_lines_are_no_rules() {
    let owners = Codeowners::parse(
        "# The owner approves these.\n\n   \n  # indented comment\n/factory/ @cabcookie # trailing\n",
    );
    assert_eq!(
        owners.owners("factory/CONFIG.md"),
        Some(&["@cabcookie".to_string()][..])
    );
    assert!(!owners.covers("#"));
}

#[test]
fn several_owners_and_emails_are_owners() {
    let owners = Codeowners::parse("*.go docs@example.com @org/team\t@octocat\r\n");
    assert_eq!(
        owners.owners("cmd/main.go").unwrap(),
        ["docs@example.com", "@org/team", "@octocat"]
    );
}

#[test]
fn lines_with_syntax_github_does_not_support_are_skipped() {
    // GitHub skips a line it cannot read; negation and character ranges are such lines.
    let owners = Codeowners::parse("* @octocat\n!factory/ @nobody\n/factory/[a-z]* @nobody\n");
    assert_eq!(
        owners.owners("factory/CONFIG.md"),
        Some(&["@octocat".to_string()][..])
    );
    assert_eq!(
        owners.owners("!factory/x"),
        Some(&["@octocat".to_string()][..])
    );
}

#[test]
fn this_repositorys_codeowners_covers_the_factory_rules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(root.join(".github/CODEOWNERS")).unwrap();
    let owners = Codeowners::parse(&text);
    for path in [
        "factory/FACTORY.md",
        "factory/CONFIG.md",
        "factory/GENESIS.md",
        "factory/roles/pm.md",
        "factory/roles/coder.md",
        "Cargo.lock",
    ] {
        assert!(owners.covers(path), "{path}");
    }
    assert!(!owners.covers("factory/decisions/x.md"));
    assert!(!owners.covers("crates/seam/src/lib.rs"));
}

// --- finding the file ---------------------------------------------------------------------------

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nex132-codeowners-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn github_dir_comes_before_root_and_docs() {
    let root = scratch("order1");
    put(&root, ".github/CODEOWNERS", "a");
    put(&root, "CODEOWNERS", "b");
    put(&root, "docs/CODEOWNERS", "c");
    assert_eq!(
        find_in(&root).unwrap(),
        Some((".github/CODEOWNERS", "a".to_string()))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn root_comes_before_docs() {
    let root = scratch("order2");
    put(&root, "CODEOWNERS", "b");
    put(&root, "docs/CODEOWNERS", "c");
    assert_eq!(
        find_in(&root).unwrap(),
        Some(("CODEOWNERS", "b".to_string()))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn docs_is_found_last() {
    let root = scratch("order3");
    put(&root, "docs/CODEOWNERS", "c");
    assert_eq!(
        find_in(&root).unwrap(),
        Some(("docs/CODEOWNERS", "c".to_string()))
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn no_file_is_none() {
    let root = scratch("none");
    put(&root, "README.md", "x");
    put(&root, "src/CODEOWNERS", "not a place GitHub looks");
    put(&root, ".github/codeowners", "the name is case sensitive");
    fs::create_dir_all(root.join("CODEOWNERS")).unwrap();
    assert_eq!(find_in(&root).unwrap(), None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn an_empty_file_is_found() {
    let root = scratch("empty");
    put(&root, "CODEOWNERS", "");
    assert_eq!(find_in(&root).unwrap(), Some(("CODEOWNERS", String::new())));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn find_reads_through_any_source_in_githubs_order() {
    // The merge gate reads the base commit, not the working tree (RFC 0010).
    let mut asked = Vec::new();
    let found = find(|path| {
        asked.push(path.to_string());
        Ok((path == "docs/CODEOWNERS").then(|| "* @octocat".to_string()))
    })
    .unwrap();
    assert_eq!(
        asked,
        [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"]
    );
    assert_eq!(found, Some(("docs/CODEOWNERS", "* @octocat".to_string())));
}

// --- the report ---------------------------------------------------------------------------------

#[test]
fn the_report_names_the_file_and_the_uncovered_paths_in_the_given_order() {
    let root = scratch("report");
    put(
        &root,
        ".github/CODEOWNERS",
        "/factory/FACTORY.md @o\n/factory/roles/coder.md @o\n",
    );
    let paths = [
        "factory/CONFIG.md",
        "factory/FACTORY.md",
        "factory/roles/coder.md",
        "factory/roles/pm.md",
    ];
    assert_eq!(
        report(&root, &paths).unwrap(),
        Some(Report {
            file: ".github/CODEOWNERS".to_string(),
            uncovered: vec![
                "factory/CONFIG.md".to_string(),
                "factory/roles/pm.md".to_string()
            ],
        })
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_report_is_none_without_a_codeowners_file() {
    let root = scratch("noreport");
    assert_eq!(report(&root, &["factory/CONFIG.md"]).unwrap(), None);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_report_never_writes_codeowners() {
    let root = scratch("readonly");
    put(&root, "CODEOWNERS", "/factory/FACTORY.md @o\n");
    report(&root, &["factory/CONFIG.md"]).unwrap();
    let mut names: Vec<String> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["CODEOWNERS"]);
    assert_eq!(
        fs::read_to_string(root.join("CODEOWNERS")).unwrap(),
        "/factory/FACTORY.md @o\n"
    );
    fs::remove_dir_all(&root).unwrap();
}
