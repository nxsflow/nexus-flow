use super::*;

const HASH: &str = "950df51e800596505944bf58dbbec8fa26f05e41";
const GIT: &str = "https://github.com/nocentraldb/nocentraldb";

fn manifest(entry: &str) -> String {
    format!("[workspace]\nmembers = []\n\n[workspace.dependencies]\nnocentraldb = {entry}\n")
}

#[test]
fn accepts_a_git_dependency_at_a_full_commit_hash() {
    let text = manifest(&format!(r#"{{ git = "{GIT}", rev = "{HASH}" }}"#));
    assert_eq!(check(&text).unwrap(), HASH);
}

#[test]
fn refuses_a_branch() {
    let text = manifest(&format!(r#"{{ git = "{GIT}", branch = "main" }}"#));
    assert!(check(&text).is_err());
}

#[test]
fn refuses_a_tag() {
    let text = manifest(&format!(r#"{{ git = "{GIT}", tag = "v0.1.0-beta.1" }}"#));
    assert!(check(&text).is_err());
}

#[test]
fn refuses_a_rev_next_to_a_branch_or_tag() {
    for extra in [r#"branch = "main""#, r#"tag = "v0.1.0-beta.1""#] {
        let text = manifest(&format!(r#"{{ git = "{GIT}", rev = "{HASH}", {extra} }}"#));
        assert!(check(&text).is_err(), "{extra}");
    }
}

#[test]
fn refuses_a_short_or_non_hex_rev() {
    for rev in [
        "950df51",
        "v0.1.0-beta.1",
        "950DF51E800596505944BF58DBBEC8FA26F05E41",
        &format!("{HASH}0"),
    ] {
        let text = manifest(&format!(r#"{{ git = "{GIT}", rev = "{rev}" }}"#));
        assert!(check(&text).is_err(), "{rev}");
    }
}

#[test]
fn refuses_a_git_dependency_without_a_rev() {
    let text = manifest(&format!(r#"{{ git = "{GIT}" }}"#));
    assert!(check(&text).is_err());
}

#[test]
fn refuses_a_registry_version_or_a_path() {
    for entry in [
        r#""0.1.0-beta.1""#,
        r#"{ version = "0.1.0-beta.1" }"#,
        r#"{ path = "../nocentraldb" }"#,
    ] {
        assert!(check(&manifest(entry)).is_err(), "{entry}");
    }
}

#[test]
fn refuses_a_manifest_without_the_sdk() {
    assert!(check("[workspace]\nmembers = []\n").is_err());
}

#[test]
fn the_workspace_pins_the_sdk_at_a_full_commit_hash() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    check(&text).unwrap();
}
