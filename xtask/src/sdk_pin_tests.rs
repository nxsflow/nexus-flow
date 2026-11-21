use super::*;

const VERSION: &str = "=0.1.0-beta.1";

fn manifest(entries: &str) -> String {
    format!("[workspace]\nmembers = []\n\n[workspace.dependencies]\n{entries}\n")
}

fn sdk(entry: &str) -> String {
    manifest(&format!("nocentraldb = {entry}"))
}

#[test]
fn accepts_an_exact_version() {
    assert_eq!(check(&sdk(&format!(r#""{VERSION}""#))).unwrap(), VERSION);
    let table = sdk(&format!(
        r#"{{ version = "{VERSION}", default-features = false }}"#
    ));
    assert_eq!(check(&table).unwrap(), VERSION);
    assert_eq!(check(&sdk(r#""=1.2.3""#)).unwrap(), "=1.2.3");
}

#[test]
fn refuses_a_version_that_is_not_exact() {
    for req in [
        "0.1.0-beta.1",
        "^0.1.0-beta.1",
        "~0.1",
        "=0.1",
        "=0",
        ">=0.1.0",
        "*",
        "=0.1.0, <0.2",
        "= 0.1.0",
    ] {
        assert!(check(&sdk(&format!(r#""{req}""#))).is_err(), "{req}");
    }
}

#[test]
fn refuses_git_path_and_another_registry() {
    for entry in [
        r#"{ git = "https://github.com/nocentraldb/nocentraldb", rev = "950df51e800596505944bf58dbbec8fa26f05e41" }"#,
        r#"{ git = "https://github.com/nocentraldb/nocentraldb", tag = "v0.1.0-beta.1" }"#,
        r#"{ version = "=0.1.0-beta.1", git = "https://github.com/nocentraldb/nocentraldb" }"#,
        r#"{ version = "=0.1.0-beta.1", path = "../nocentraldb/crates/nocentraldb" }"#,
        r#"{ version = "=0.1.0-beta.1", registry = "elsewhere" }"#,
        r#"{ path = "../nocentraldb/crates/nocentraldb" }"#,
    ] {
        assert!(check(&sdk(entry)).is_err(), "{entry}");
    }
}

#[test]
fn checks_every_ncdb_crate_the_workspace_names() {
    let pinned = format!(r#"nocentraldb = "{VERSION}""#);
    let ok = manifest(&format!(
        "{pinned}\nncdb-core = \"{VERSION}\"\nncdb-plugin = \"{VERSION}\""
    ));
    assert!(check(&ok).is_ok());
    let loose = manifest(&format!("{pinned}\nncdb-core = \"0.1.0-beta.1\""));
    assert!(check(&loose).is_err());
}

#[test]
fn refuses_a_manifest_without_the_sdk() {
    assert!(check("[workspace]\nmembers = []\n").is_err());
    assert!(check(&manifest(r#"ncdb-core = "=0.1.0-beta.1""#)).is_err());
}

#[test]
fn the_workspace_pins_the_sdk_at_an_exact_version() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../Cargo.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    check(&text).unwrap();
}
