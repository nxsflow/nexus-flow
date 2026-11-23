//! The shipped defaults (RFC 0006, "Installing the shipped defaults"; RFC 0004, "The guide"):
//! `shipped.toml` lists the SHA-256 of every version nexus-flow shipped for a path, oldest first,
//! and a repository's file at that path is compared with them after CRLF is turned into LF.

use nexus_flow_seam::install::shipped::{sha256_lf, Shipped, ShippedError, State};

const V1: &str = "# Coder\n\nVersion one.\n";
const V2: &str = "# Coder\n\nVersion two.\n";
const V3: &str = "# Coder\n\nVersion three.\n";

fn crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

/// A shipped list with `roles/coder.md` at three versions and `roles/old.md` retired.
fn shipped() -> Shipped {
    let toml = format!(
        r#"
[[file]]
path = "roles/coder.md"
sha256 = ["{}", "{}", "{}"]

[[file]]
path = "roles/old.md"
sha256 = ["{}"]
retired = true
"#,
        sha256_lf(V1.as_bytes()),
        sha256_lf(V2.as_bytes()),
        sha256_lf(V3.as_bytes()),
        sha256_lf(V1.as_bytes()),
    );
    Shipped::parse(&toml).unwrap()
}

fn state(path: &str, file: Option<&str>, update: bool) -> Option<State> {
    shipped().state(path, file.map(str::as_bytes), update)
}

// --- the hash -----------------------------------------------------------------------------------

#[test]
fn the_hash_is_lowercase_hex_sha256() {
    assert_eq!(
        sha256_lf(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn the_hash_turns_crlf_into_lf_first() {
    assert_eq!(sha256_lf(crlf(V2).as_bytes()), sha256_lf(V2.as_bytes()));
}

#[test]
fn the_hash_keeps_a_lone_carriage_return() {
    assert_ne!(sha256_lf(b"a\rb"), sha256_lf(b"a\nb"));
    assert_ne!(sha256_lf(b"a\rb"), sha256_lf(b"ab"));
}

// --- the state table ----------------------------------------------------------------------------

#[test]
fn a_missing_file_is_written() {
    assert_eq!(state("roles/coder.md", None, false), Some(State::Written));
    assert_eq!(state("roles/coder.md", None, true), Some(State::Written));
}

#[test]
fn the_current_default_is_current() {
    assert_eq!(state("roles/coder.md", Some(V3), false), Some(State::Current));
    assert_eq!(state("roles/coder.md", Some(V3), true), Some(State::Current));
}

#[test]
fn the_current_default_with_crlf_is_current() {
    assert_eq!(
        state("roles/coder.md", Some(&crlf(V3)), false),
        Some(State::Current)
    );
}

#[test]
fn an_earlier_default_is_older_default() {
    assert_eq!(
        state("roles/coder.md", Some(V1), false),
        Some(State::OlderDefault)
    );
    assert_eq!(
        state("roles/coder.md", Some(V2), false),
        Some(State::OlderDefault)
    );
}

#[test]
fn an_earlier_default_with_crlf_is_older_default() {
    assert_eq!(
        state("roles/coder.md", Some(&crlf(V1)), false),
        Some(State::OlderDefault)
    );
}

#[test]
fn an_earlier_default_with_update_is_updated() {
    assert_eq!(
        state("roles/coder.md", Some(V1), true),
        Some(State::Updated)
    );
    assert_eq!(
        state("roles/coder.md", Some(&crlf(V2)), true),
        Some(State::Updated)
    );
}

#[test]
fn anything_else_is_team() {
    let changed = format!("{V3}A line the team added.\n");
    assert_eq!(
        state("roles/coder.md", Some(&changed), false),
        Some(State::Team)
    );
    assert_eq!(
        state("roles/coder.md", Some(&changed), true),
        Some(State::Team)
    );
    assert_eq!(state("roles/coder.md", Some(""), true), Some(State::Team));
}

#[test]
fn a_lone_carriage_return_is_a_change() {
    let changed = V3.replace('\n', "\r");
    assert_eq!(
        state("roles/coder.md", Some(&changed), false),
        Some(State::Team)
    );
}

#[test]
fn a_present_file_at_a_retired_path_is_retired_whatever_it_holds() {
    assert_eq!(state("roles/old.md", Some(V1), false), Some(State::Retired));
    assert_eq!(state("roles/old.md", Some(V1), true), Some(State::Retired));
    assert_eq!(
        state("roles/old.md", Some("the team's text"), true),
        Some(State::Retired)
    );
}

#[test]
fn a_missing_file_at_a_retired_path_has_no_state() {
    assert_eq!(state("roles/old.md", None, false), None);
    assert_eq!(state("roles/old.md", None, true), None);
}

#[test]
fn a_path_the_list_does_not_name_has_no_state() {
    assert_eq!(state("roles/team-role.md", Some(V1), true), None);
    assert_eq!(state("roles/team-role.md", None, true), None);
}

#[test]
fn a_default_shipped_again_after_a_change_is_current() {
    let toml = format!(
        "[[file]]\npath = \"FACTORY.md\"\nsha256 = [\"{}\", \"{}\", \"{}\"]\n",
        sha256_lf(V1.as_bytes()),
        sha256_lf(V2.as_bytes()),
        sha256_lf(V1.as_bytes()),
    );
    let shipped = Shipped::parse(&toml).unwrap();
    assert_eq!(
        shipped.state("FACTORY.md", Some(V1.as_bytes()), true),
        Some(State::Current)
    );
}

#[test]
fn the_states_carry_rfc_0006_names() {
    let names: Vec<&str> = [
        State::Written,
        State::Current,
        State::OlderDefault,
        State::Updated,
        State::Team,
        State::Retired,
    ]
    .iter()
    .map(|s| s.as_str())
    .collect();
    assert_eq!(
        names,
        [
            "written",
            "current",
            "older_default",
            "updated",
            "team",
            "retired"
        ]
    );
}

// --- reading shipped.toml -----------------------------------------------------------------------

#[test]
fn the_paths_come_in_bytewise_order() {
    let h = sha256_lf(b"x");
    let toml = format!(
        "[[file]]\npath = \"roles/pm.md\"\nsha256 = [\"{h}\"]\n\
         [[file]]\npath = \"FACTORY.md\"\nsha256 = [\"{h}\"]\n\
         [[file]]\npath = \"CONFIG.md\"\nsha256 = [\"{h}\"]\n\
         [[file]]\npath = \"roles/coder.md\"\nsha256 = [\"{h}\"]\n"
    );
    let shipped = Shipped::parse(&toml).unwrap();
    let paths: Vec<&str> = shipped.paths().collect();
    assert_eq!(
        paths,
        ["CONFIG.md", "FACTORY.md", "roles/coder.md", "roles/pm.md"]
    );
}

#[test]
fn a_path_reports_whether_it_is_retired() {
    let shipped = shipped();
    assert!(!shipped.is_retired("roles/coder.md"));
    assert!(shipped.is_retired("roles/old.md"));
}

#[test]
fn the_current_hash_is_the_last_one() {
    assert_eq!(
        shipped().current("roles/coder.md"),
        Some(sha256_lf(V3.as_bytes()).as_str())
    );
    assert_eq!(shipped().current("roles/team-role.md"), None);
}

#[test]
fn an_empty_list_reads_as_no_files() {
    assert_eq!(Shipped::parse("").unwrap().paths().count(), 0);
}

fn refused(toml: &str) -> ShippedError {
    Shipped::parse(toml).unwrap_err()
}

const H: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn invalid_toml_is_refused() {
    assert!(matches!(refused("[[file]"), ShippedError::Toml(_)));
}

#[test]
fn an_unknown_key_is_refused() {
    assert!(matches!(
        refused(&format!(
            "[[file]]\npath = \"a.md\"\nsha256 = [\"{H}\"]\nnote = \"x\"\n"
        )),
        ShippedError::Toml(_)
    ));
    assert!(matches!(refused("format = 1\n"), ShippedError::Toml(_)));
}

#[test]
fn a_file_without_hashes_is_refused() {
    assert_eq!(
        refused("[[file]]\npath = \"a.md\"\nsha256 = []\n"),
        ShippedError::NoHash("a.md".into())
    );
}

#[test]
fn a_hash_that_is_not_64_lowercase_hex_digits_is_refused() {
    for bad in [&H[..63], &H.to_uppercase(), &format!("{}g", &H[..63])] {
        assert_eq!(
            refused(&format!(
                "[[file]]\npath = \"a.md\"\nsha256 = [\"{bad}\"]\n"
            )),
            ShippedError::HashInvalid("a.md".into()),
            "{bad}"
        );
    }
}

#[test]
fn a_hash_listed_twice_for_one_path_is_refused() {
    assert_eq!(
        refused(&format!(
            "[[file]]\npath = \"a.md\"\nsha256 = [\"{H}\", \"{H}\"]\n"
        )),
        ShippedError::HashDuplicate("a.md".into())
    );
}

#[test]
fn a_path_listed_twice_is_refused() {
    assert_eq!(
        refused(&format!(
            "[[file]]\npath = \"a.md\"\nsha256 = [\"{H}\"]\n[[file]]\npath = \"a.md\"\nsha256 = [\"{H}\"]\n"
        )),
        ShippedError::PathDuplicate("a.md".into())
    );
}

#[test]
fn a_path_that_is_not_relative_and_plain_is_refused() {
    for bad in [
        "", "/a.md", "a.md/", "a//b.md", "./a.md", "roles/../a.md", "..", "roles\\a.md",
    ] {
        assert_eq!(
            refused(&format!(
                "[[file]]\npath = \"{}\"\nsha256 = [\"{H}\"]\n",
                bad.replace('\\', "\\\\")
            )),
            ShippedError::PathInvalid(bad.into()),
            "{bad:?}"
        );
    }
}
