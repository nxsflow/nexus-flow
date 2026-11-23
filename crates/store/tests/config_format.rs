//! `nexus-flow.toml`, format 1 (RFC 0001, "Creating and finding a space"): what it holds, how it
//! is read and written, and what is refused.

use std::path::Path;

use nexus_flow_store::config::{Config, ConfigError, Problem};

const FILE: &str = "/work/repo/nexus-flow.toml";

/// RFC 0001's example, with space ids in place of its placeholders.
const DOCUMENTED: &str = r#"# nexus-flow.toml, committed. Holds no secret: a space id grants nothing.
format = 1

[space]
repository = "sp_repository_0001"
coordination = "sp_coordination_0002"   # optional

[git]
base = "main"                             # the team's base branch, which "on main" is checked against
"#;

fn parse(text: &str) -> Result<Config, ConfigError> {
    Config::parse(text, Path::new(FILE))
}

fn problem(text: &str) -> Problem {
    let err = parse(text).unwrap_err();
    assert_eq!(err.file, Path::new(FILE), "the error names the file");
    assert!(err.to_string().contains(FILE), "{err}");
    err.problem
}

#[test]
fn the_documented_example_round_trips() {
    let config = parse(DOCUMENTED).expect("the documented example parses");
    assert_eq!(
        config,
        Config {
            repository: "sp_repository_0001".into(),
            coordination: Some("sp_coordination_0002".into()),
            base: "main".into(),
        }
    );
    assert_eq!(parse(&config.to_toml()), Ok(config));
}

#[test]
fn every_config_round_trips_through_its_text() {
    for config in [
        Config::new("sp_only"),
        Config {
            repository: "sp \"quoted\" \\ id".into(),
            coordination: Some("sp_coord".into()),
            base: "v2".into(),
        },
    ] {
        let text = config.to_toml();
        assert!(text.starts_with("# nexus-flow.toml"), "{text}");
        assert_eq!(parse(&text), Ok(config), "{text}");
    }
}

#[test]
fn the_text_written_leaves_out_an_unset_coordination_space() {
    let text = Config::new("sp_only").to_toml();
    assert!(!text.contains("coordination"), "{text}");
    assert!(text.contains("base = \"main\""), "{text}");
}

#[test]
fn base_defaults_to_main() {
    let no_git = "format = 1\n[space]\nrepository = \"sp\"\n";
    assert_eq!(parse(no_git).unwrap().base, "main");

    let no_base = "format = 1\n[space]\nrepository = \"sp\"\n[git]\n";
    assert_eq!(parse(no_base).unwrap().base, "main");

    assert_eq!(Config::new("sp").base, "main");
}

#[test]
fn a_format_other_than_1_is_refused_naming_the_file() {
    let future = "format = 2\n[space]\nrepository = \"sp\"\n";
    assert_eq!(
        problem(future),
        Problem::UnsupportedFormat { found: "2".into() }
    );
    let message = parse(future).unwrap_err().to_string();
    assert!(message.contains("format 2"), "{message}");

    assert_eq!(
        problem("format = \"1\"\n[space]\nrepository = \"sp\"\n"),
        Problem::UnsupportedFormat {
            found: "\"1\"".into()
        }
    );
    assert_eq!(
        problem("[space]\nrepository = \"sp\"\n"),
        Problem::Missing { key: "format" }
    );
}

#[test]
fn a_missing_or_empty_repository_space_is_refused() {
    assert_eq!(
        problem("format = 1\n"),
        Problem::Missing {
            key: "space.repository"
        }
    );
    assert_eq!(
        problem("format = 1\n[space]\nrepository = \"\"\n"),
        Problem::Empty {
            key: "space.repository"
        }
    );
    assert_eq!(
        problem("format = 1\n[space]\nrepository = 7\n"),
        Problem::NotAString {
            key: "space.repository"
        }
    );
    assert_eq!(
        problem("format = 1\n[space]\nrepository = \"sp\"\ncoordination = \"\"\n"),
        Problem::Empty {
            key: "space.coordination"
        }
    );
}

#[test]
fn an_unknown_key_is_refused_not_ignored() {
    assert_eq!(
        problem("format = 1\n[space]\nrepository = \"sp\"\n[git]\nbsae = \"v2\"\n"),
        Problem::UnknownKey {
            key: "git.bsae".into()
        }
    );
    assert_eq!(
        problem("format = 1\nspace = \"sp\"\n"),
        Problem::NotATable { key: "space" }
    );
    assert_eq!(
        problem("format = 1\n[space]\nrepository = \"sp\"\n[sync]\n"),
        Problem::UnknownKey { key: "sync".into() }
    );
}

#[test]
fn text_that_is_not_toml_is_refused_naming_the_file() {
    assert!(matches!(
        problem("format = 1\n[space\n"),
        Problem::Syntax { .. }
    ));
}
