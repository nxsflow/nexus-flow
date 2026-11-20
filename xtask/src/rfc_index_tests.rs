use std::path::Path;
use std::process::Command;

use super::*;

fn rfc_text(number: &str, title: &str, status: &str) -> String {
    format!("# RFC {number}: {title}\n\nStatus: {status}\n\nOne paragraph.\n")
}

/// A `docs/rfc/` folder with the template, a README, and the given RFCs.
fn folder(rfcs: &[(&str, &str, &str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("0000-template.md"),
        "# RFC NNNN: <title>\n\nStatus: Draft | Decided (YYYY-MM-DD)\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), "# RFCs\n\nHow they work.\n").unwrap();
    for (file, number, title, status) in rfcs {
        std::fs::write(dir.path().join(file), rfc_text(number, title, status)).unwrap();
    }
    dir
}

#[test]
fn reads_number_title_and_status_and_skips_the_template_and_readme() {
    let dir = folder(&[
        ("0002-tickets.md", "0002", "Tickets in NoCentralDB", "Draft"),
        (
            "0001-nexus-flow-on-nocentraldb.md",
            "0001",
            "nexus-flow on NoCentralDB",
            "Decided (2026-11-17)",
        ),
    ]);
    let rfcs = read_rfcs(dir.path()).unwrap();
    assert_eq!(
        rfcs,
        vec![
            Rfc {
                number: "0001".into(),
                file: "0001-nexus-flow-on-nocentraldb.md".into(),
                title: "nexus-flow on NoCentralDB".into(),
                status: "Decided (2026-11-17)".into(),
            },
            Rfc {
                number: "0002".into(),
                file: "0002-tickets.md".into(),
                title: "Tickets in NoCentralDB".into(),
                status: "Draft".into(),
            },
        ]
    );
}

#[test]
fn a_heading_whose_number_differs_from_the_file_name_is_an_error() {
    let dir = folder(&[("0003-decisions.md", "0004", "decisions", "Draft")]);
    let err = read_rfcs(dir.path()).unwrap_err().to_string();
    assert!(err.contains("0003-decisions.md"), "{err}");
}

#[test]
fn an_rfc_without_a_status_line_is_an_error() {
    let dir = folder(&[]);
    std::fs::write(
        dir.path().join("0005-goals.md"),
        "# RFC 0005: Goals\n\nNo status here.\n",
    )
    .unwrap();
    let err = read_rfcs(dir.path()).unwrap_err().to_string();
    assert!(
        err.contains("0005-goals.md") && err.contains("Status:"),
        "{err}"
    );
}

#[test]
fn renders_one_row_per_rfc_with_a_link_and_escapes_pipes() {
    let rfcs = vec![Rfc {
        number: "0007".into(),
        file: "0007-status.md".into(),
        title: "status | notes".into(),
        status: "Draft".into(),
    }];
    assert_eq!(
        render(&rfcs),
        "| RFC | Title | Status |\n\
         | --- | --- | --- |\n\
         | [0007](0007-status.md) | status \\| notes | Draft |\n"
    );
}

#[test]
fn the_index_prints_one_line_per_rfc() {
    let dir = folder(&[
        ("0002-tickets.md", "0002", "Tickets in NoCentralDB", "Draft"),
        ("0001-one.md", "0001", "One", "Decided (2026-11-17)"),
    ]);
    assert_eq!(
        index(dir.path(), false).unwrap(),
        "0001 · One · Decided (2026-11-17)\n0002 · Tickets in NoCentralDB · Draft\n"
    );
}

#[test]
fn md_prints_the_index_as_a_markdown_table() {
    let dir = folder(&[("0010-merge-gate.md", "0010", "The merge gate", "Draft")]);
    assert_eq!(
        index(dir.path(), true).unwrap(),
        "| RFC | Title | Status |\n| --- | --- | --- |\n\
         | [0010](0010-merge-gate.md) | The merge gate | Draft |\n"
    );
}

fn git(repo: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

/// The acceptance case: two RFC branches each add an RFC file and touch no shared file, so merging
/// one does not make the other conflict, and the index on the base lists both.
#[test]
fn two_rfc_branches_merge_without_conflict_and_the_index_lists_both() {
    let repo = folder(&[("0001-one.md", "0001", "One", "Decided (2026-11-17)")]);
    let path = repo.path();
    git(path, &["init", "-q", "-b", "v2"]);
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "base"]);

    for (branch, file, number, title) in [
        ("rfc-a", "0002-a.md", "0002", "A"),
        ("rfc-b", "0003-b.md", "0003", "B"),
    ] {
        git(path, &["checkout", "-q", "-b", branch, "v2"]);
        std::fs::write(path.join(file), rfc_text(number, title, "Draft")).unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", branch]);
    }

    git(path, &["checkout", "-q", "v2"]);
    git(path, &["merge", "-q", "--no-ff", "--no-edit", "rfc-a"]);
    git(path, &["merge-tree", "--write-tree", "v2", "rfc-b"]);
    git(path, &["merge", "-q", "--no-ff", "--no-edit", "rfc-b"]);

    assert_eq!(
        index(path, false).unwrap(),
        "0001 · One · Decided (2026-11-17)\n0002 · A · Draft\n0003 · B · Draft\n"
    );
}

/// The repository's README keeps no table of RFCs, so no RFC pull request has a shared file to edit.
#[test]
fn the_repository_readme_keeps_no_table_of_rfcs() {
    let readme = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rfc/README.md");
    let text = std::fs::read_to_string(readme).unwrap();
    assert!(!text.contains("| [0001]"), "{text}");
    assert!(text.contains("cargo xtask rfc-index"), "{text}");
}

#[test]
fn check_passes_on_well_formed_rfcs() {
    let dir = folder(&[
        ("0001-one.md", "0001", "One", "Draft"),
        ("0002-two.md", "0002", "Two", "Draft"),
    ]);
    check(dir.path()).unwrap();
}

#[test]
fn check_fails_when_two_rfc_files_share_a_number() {
    let dir = folder(&[
        ("0011-a.md", "0011", "A", "Draft"),
        ("0011-b.md", "0011", "B", "Draft"),
    ]);
    let err = check(dir.path()).unwrap_err().to_string();
    assert!(
        err.contains("0011-a.md") && err.contains("0011-b.md"),
        "{err}"
    );
}

#[test]
fn check_fails_when_a_heading_number_differs_from_the_file_name() {
    let dir = folder(&[("0012-a.md", "0013", "A", "Draft")]);
    let err = check(dir.path()).unwrap_err().to_string();
    assert!(err.contains("0012-a.md"), "{err}");
}

/// CI runs the check in `lint` on every event: it can only fail on a mistake in the RFC files.
#[test]
fn ci_runs_the_check_in_lint_on_every_event() {
    let ci = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows/ci.yml");
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(ci).unwrap()).unwrap();
    let steps = yaml["jobs"]["lint"]["steps"].as_sequence().unwrap();
    let step = steps
        .iter()
        .find(|s| s["run"].as_str() == Some("cargo xtask rfc-index --check"))
        .expect("lint runs cargo xtask rfc-index --check");
    assert!(step.get("if").is_none(), "the check runs unconditionally");
}
