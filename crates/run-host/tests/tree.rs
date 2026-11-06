//! The tree tests of §12.11, over the real `GitTree` against temporary repositories with a file-path origin.

use std::path::Path;
use std::process::Command;

/// The repository's tracked file, as it is in this commit.
const TRACKED_GITIGNORE: &str = include_str!("../../../.gitignore");

fn in_repo(dir: &Path, args: &[&str]) -> String {
    let empty = std::env::temp_dir().join("zjyv-empty-gitconfig");
    std::fs::write(&empty, b"").expect("an empty config");
    let output = Command::new("git")
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", &empty)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        // The default `core.excludesFile` is under here, and this machine ignores `.claude/settings.local.json` there.
        .env("XDG_CONFIG_HOME", dir.parent().expect("a parent"))
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The finisher's clean-tree check (§6.6): a clone has no self-ignoring `.nxf/.gitignore`, so the verifier's gate log
/// and a session's local settings have to be ignored by the file the clone carries, not by this machine's excludes.
#[test]
fn finisher_clean_check_passes_in_a_fresh_clone_with_a_gate_log() {
    let temp = tempfile::tempdir().expect("a temp directory");
    let origin = temp.path().join("origin");
    std::fs::create_dir_all(&origin).expect("the origin");
    std::fs::write(origin.join(".gitignore"), TRACKED_GITIGNORE).expect("the tracked file");
    in_repo(&origin, &["-c", "init.defaultBranch=main", "init", "-q"]);
    in_repo(&origin, &["add", "-A"]);
    in_repo(&origin, &["commit", "-q", "-m", "first"]);
    in_repo(temp.path(), &["clone", "-q", "origin", "clone"]);
    let clone = temp.path().join("clone");
    std::fs::create_dir_all(clone.join(".nxf/review")).expect("the gate log's directory");
    std::fs::write(clone.join(".nxf/review/gates-x.txt"), b"green").expect("a gate log");
    std::fs::create_dir_all(clone.join(".claude")).expect("the settings' directory");
    std::fs::write(clone.join(".claude/settings.local.json"), b"{}").expect("local settings");

    assert_eq!(in_repo(&clone, &["status", "--porcelain"]), "", "the tree is clean");
    for ignored in [".nxf/review/gates-x.txt", ".claude/settings.local.json"] {
        let answer = in_repo(&clone, &["check-ignore", "-v", ignored]);
        assert!(answer.starts_with(".gitignore:"), "{ignored} by: {answer:?}");
    }
}
