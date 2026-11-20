use std::path::Path;
use std::process::Command;

use super::*;

fn rfc_text(number: &str, title: &str, status: &str) -> String {
    format!("# RFC {number}: {title}\n\nStatus: {status}\n\nOne paragraph.\n")
}

const PROSE_ABOVE: &str = "# RFCs\n\nThe RFCs here design the target state.\n\n";
const PROSE_BELOW: &str = "\n\n## Writing one\n\nCopy the template.\n";

fn readme_with(table: &str) -> String {
    format!("{PROSE_ABOVE}{BEGIN}\n{table}{END}{PROSE_BELOW}")
}

/// A `docs/rfc/` folder with the template, a README whose table is empty, and the given RFCs.
fn folder(rfcs: &[(&str, &str, &str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("0000-template.md"),
        "# RFC NNNN: <title>\n\nStatus: Draft | Decided (YYYY-MM-DD)\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("README.md"), readme_with("")).unwrap();
    for (file, number, title, status) in rfcs {
        std::fs::write(dir.path().join(file), rfc_text(number, title, status)).unwrap();
    }
    dir
}

fn readme(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("README.md")).unwrap()
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
    assert!(err.contains("0005-goals.md") && err.contains("Status:"), "{err}");
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
fn writes_the_table_between_the_markers_and_keeps_the_prose() {
    let dir = folder(&[("0010-merge-gate.md", "0010", "The merge gate", "Draft")]);
    run(dir.path(), false).unwrap();
    let text = readme(dir.path());
    assert_eq!(
        text,
        readme_with(
            "| RFC | Title | Status |\n| --- | --- | --- |\n\
             | [0010](0010-merge-gate.md) | The merge gate | Draft |\n"
        )
    );
}

#[test]
fn a_readme_without_the_markers_is_an_error() {
    let err = splice("# RFCs\n\nNo table.\n", "| RFC |\n")
        .unwrap_err()
        .to_string();
    assert!(err.contains("rfc-index"), "{err}");
}

#[test]
fn check_passes_on_a_freshly_written_table() {
    let dir = folder(&[("0006-factory.md", "0006", "Factory storage", "Draft")]);
    run(dir.path(), false).unwrap();
    run(dir.path(), true).unwrap();
}

#[test]
fn check_fails_on_a_hand_edited_table_and_leaves_it_alone() {
    let dir = folder(&[("0006-factory.md", "0006", "Factory storage", "Draft")]);
    run(dir.path(), false).unwrap();
    let edited = readme(dir.path()).replace("| Draft |", "| Decided (2026-11-20) |");
    std::fs::write(dir.path().join("README.md"), &edited).unwrap();
    let err = run(dir.path(), true).unwrap_err().to_string();
    assert!(err.contains("cargo xtask rfc-index"), "{err}");
    assert_eq!(readme(dir.path()), edited);
}

#[test]
fn check_fails_when_an_rfc_has_no_row_yet() {
    let dir = folder(&[("0006-factory.md", "0006", "Factory storage", "Draft")]);
    run(dir.path(), false).unwrap();
    std::fs::write(
        dir.path().join("0011-next.md"),
        rfc_text("0011", "Next", "Draft"),
    )
    .unwrap();
    assert!(run(dir.path(), true).is_err());
}

fn git(repo: &Path, args: &[&str]) -> std::process::Output {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
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

/// The acceptance case: two RFC branches each add an RFC file and leave README.md alone. Merging
/// one into the base does not make the other conflict, and regenerating on the base lists both.
#[test]
fn two_rfc_branches_merge_without_conflict_and_the_base_regenerates_both_rows() {
    let repo = folder(&[("0001-one.md", "0001", "One", "Decided (2026-11-17)")]);
    let path = repo.path();
    run(path, false).unwrap();
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
    // The other branch still merges cleanly: neither branch touched README.md.
    git(path, &["merge-tree", "--write-tree", "v2", "rfc-b"]);
    git(path, &["merge", "-q", "--no-ff", "--no-edit", "rfc-b"]);

    assert!(run(path, true).is_err(), "the base's table is stale until regenerated");
    run(path, false).unwrap();
    run(path, true).unwrap();
    let text = readme(path);
    assert!(text.contains("| [0002](0002-a.md) | A | Draft |"), "{text}");
    assert!(text.contains("| [0003](0003-b.md) | B | Draft |"), "{text}");
}

/// The repository's own README carries the markers, so the generator has a place to write.
#[test]
fn the_repository_readme_carries_the_markers() {
    let readme = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/rfc/README.md");
    let text = std::fs::read_to_string(readme).unwrap();
    assert!(text.contains(BEGIN) && text.contains(END));
}

/// CI runs the check in the `lint` job on a push to `v2` only: an RFC pull request does not
/// regenerate the table, so on a pull request the table is stale by design.
#[test]
fn ci_runs_the_check_in_lint_on_a_push_to_v2_only() {
    let ci = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows/ci.yml");
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(ci).unwrap()).unwrap();
    let steps = yaml["jobs"]["lint"]["steps"].as_sequence().unwrap();
    let step = steps
        .iter()
        .find(|s| s["run"].as_str() == Some("cargo xtask rfc-index --check"))
        .expect("lint runs cargo xtask rfc-index --check");
    assert_eq!(
        step["if"].as_str(),
        Some("github.event_name == 'push' && github.ref_name == 'v2'")
    );
}
