use std::ffi::OsStr;
use std::path::Path;

use super::runner::{command, CONFIG, REMOVED_ENV, SET_ENV};
use super::{normalise_origin_url, scrub, GitBounds};

#[test]
fn one_repository_however_its_url_is_written_is_one_url() {
    let same = [
        "https://github.com/nxsflow/nexus-flow.git",
        "https://github.com/nxsflow/nexus-flow",
        "https://github.com/nxsflow/nexus-flow/",
        "https://github.com/nxsflow/nexus-flow.git/",
        "HTTPS://GitHub.com/nxsflow/nexus-flow",
        "https://x-access-token:secret@github.com/nxsflow/nexus-flow.git",
        "ssh://git@github.com/nxsflow/nexus-flow.git",
        "git@github.com:nxsflow/nexus-flow.git",
    ];
    for url in same {
        assert_eq!(
            normalise_origin_url(url),
            "github.com/nxsflow/nexus-flow",
            "{url}"
        );
    }
}

#[test]
fn another_repository_stays_another_url() {
    assert_ne!(
        normalise_origin_url("https://github.com/nxsflow/nexus-flow"),
        normalise_origin_url("https://github.com/nxsflow/nexus-flow-v2")
    );
    assert_ne!(
        normalise_origin_url("https://github.com/nxsflow/nexus-flow"),
        normalise_origin_url("https://github.com/NXSFLOW/nexus-flow"),
        "a path's case is the server's to judge"
    );
}

#[test]
fn a_file_path_and_its_file_url_are_one_repository() {
    assert_eq!(normalise_origin_url("/tmp/origin/"), "/tmp/origin");
    assert_eq!(
        normalise_origin_url("file:///tmp/origin.git"),
        "/tmp/origin"
    );
    assert_eq!(normalise_origin_url("/tmp/a:b/origin"), "/tmp/a:b/origin");
}

#[test]
fn a_normalised_url_names_no_credential() {
    let normal = normalise_origin_url("https://me:ghp_secret@github.com/a/b.git");
    assert!(
        !normal.contains("secret") && !normal.contains("me"),
        "{normal}"
    );
}

#[test]
fn the_scrub_removes_a_token_from_a_url_in_stderr_and_keeps_the_rest() {
    let text = "fatal: unable to access 'https://x-access-token:ghp_canary@github.com/a/b.git/': 403\nnext line";
    let clean = scrub(text);
    assert!(!clean.contains("ghp_canary"), "{clean}");
    assert!(clean.contains("github.com/a/b.git"), "{clean}");
    assert!(clean.contains("next line"), "{clean}");
}

#[test]
fn the_scrub_caps_the_text() {
    assert_eq!(scrub(&"x".repeat(100_000)).chars().count(), 2_000);
}

fn env_of<'a>(cmd: &'a std::process::Command, name: &str) -> Option<Option<&'a OsStr>> {
    cmd.get_envs()
        .find(|(key, _)| *key == OsStr::new(name))
        .map(|(_, value)| value)
}

#[test]
fn every_call_carries_the_environment_and_the_config_list_of_section_5() {
    let cmd = command(Path::new("git"), Path::new("/some/where"), &["status"]);
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
    ] {
        assert!(REMOVED_ENV.contains(&name));
        assert_eq!(env_of(&cmd, name), Some(None), "{name} is removed");
    }
    for (name, value) in [
        ("GIT_TERMINAL_PROMPT", "0"),
        ("GIT_OPTIONAL_LOCKS", "0"),
        ("GIT_ASKPASS", ""),
        ("SSH_ASKPASS", ""),
        ("GIT_PAGER", "cat"),
        ("GIT_ALLOW_PROTOCOL", "https:ssh:file"),
    ] {
        assert!(SET_ENV.contains(&(name, value)));
        assert_eq!(env_of(&cmd, name), Some(Some(OsStr::new(value))), "{name}");
    }
    let args: Vec<_> = cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    for config in [
        "core.hooksPath=",
        "gc.auto=0",
        "core.fsmonitor=false",
        "core.sshCommand=",
    ] {
        assert!(CONFIG.contains(&config));
        let at = args
            .iter()
            .position(|arg| arg == config)
            .unwrap_or_else(|| panic!("{config} in {args:?}"));
        assert_eq!(args[at - 1], "-c", "{config}");
    }
    assert_eq!(
        args.last().map(String::as_str),
        Some("status"),
        "the subcommand comes after the list"
    );
}

/// Discovery ends at the directory: an inherited ceiling is replaced, not kept, so `not_a_repository` is a fact.
#[test]
fn a_call_never_looks_above_its_directory() {
    let cmd = command(Path::new("git"), Path::new("/some/where"), &["status"]);
    assert_eq!(
        env_of(&cmd, "GIT_CEILING_DIRECTORIES"),
        Some(Some(OsStr::new("/some")))
    );
}

#[test]
fn the_bounds_are_those_of_section_5() {
    let bounds = GitBounds::default();
    assert_eq!(bounds.clone.as_secs(), 600);
    assert_eq!(bounds.fetch.as_secs(), 120);
    assert_eq!(bounds.other.as_secs(), 60);
}
