//! NEX-170: `factory/merge-gate/gate.toml` format 1 is read deterministically, and every way it
//! cannot be read has its error code (RFC 0010, "The merge rules").

use nexus_flow_merge_gate::gate_toml::{
    read_gate_toml, Effect, GateRule, GateToml, GateTomlError, RuleRef,
};

const HEAD: &str = "format = 1\ndefault = \"owner\"\nowners = [\"cabcookie\"]\nverdict_authors = [\"nexusflow-ug\"]\n";

fn rule_toml(id: &str, effect: &str, text: &str) -> String {
    format!("\n[[rule]]\nid = \"{id}\"\neffect = \"{effect}\"\ntext = \"{text}\"\n")
}

fn errors(source: &str) -> Vec<GateTomlError> {
    read_gate_toml(Some(source)).expect_err("gate.toml should not be readable")
}

fn codes(source: &str) -> Vec<&'static str> {
    errors(source).iter().map(GateTomlError::code).collect()
}

fn id(id: &str) -> RuleRef {
    RuleRef::Id(id.to_string())
}

#[test]
fn the_rfcs_example_reads_with_its_settings_and_rules() {
    let source = r#"format = 1

# What the gate decides when no rule says otherwise: "owner" or "pm".
default = "owner"

# The forge logins whose approving review counts as the owner's.
owners = ["cabcookie"]

# The forge logins whose comments may carry the verdict.
verdict_authors = ["nexusflow-ug"]

[[rule]]
id = "decided-design"
effect = "pm"
text = """
The pull request only carries out decisions the owner has already made,
such as a decided RFC, and departs from none of them.
"""

[[rule]]
id = "stored-format"
effect = "owner"
text = """
The pull request changes a stored format.
"""
"#;
    assert_eq!(
        read_gate_toml(Some(source)),
        Ok(GateToml {
            default: Effect::Owner,
            owners: vec!["cabcookie".to_string()],
            verdict_authors: vec!["nexusflow-ug".to_string()],
            rules: vec![
                GateRule {
                    id: "decided-design".to_string(),
                    effect: Effect::Pm,
                    text: "The pull request only carries out decisions the owner has already made,\nsuch as a decided RFC, and departs from none of them.\n".to_string(),
                },
                GateRule {
                    id: "stored-format".to_string(),
                    effect: Effect::Owner,
                    text: "The pull request changes a stored format.\n".to_string(),
                },
            ],
        })
    );
}

#[test]
fn rules_keep_their_order_in_the_file() {
    let ids = ["zeta", "alpha", "mid-1", "beta"];
    let source = ids.iter().fold(HEAD.to_string(), |s, i| {
        s + &rule_toml(i, "owner", "A rule.")
    });
    let read = read_gate_toml(Some(&source)).expect("readable");
    let read_ids: Vec<&str> = read.rules.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(read_ids, ids);
}

#[test]
fn a_file_without_rules_is_readable_and_pm_is_a_default() {
    let source = HEAD.replace("\"owner\"", "\"pm\"");
    let read = read_gate_toml(Some(&source)).expect("readable");
    assert_eq!(read.default, Effect::Pm);
    assert!(read.rules.is_empty());
}

#[test]
fn unknown_keys_are_ignored() {
    let source = format!(
        "notes = \"kept beside the rules\"\n{HEAD}[extra]\nwhy = 1\n{}see = \"NEX-170\"\n",
        rule_toml("a-rule", "pm", "A rule.")
    );
    let read = read_gate_toml(Some(&source)).expect("readable");
    assert_eq!(read.rules.len(), 1);
}

#[test]
fn a_missing_file_is_file_missing() {
    assert_eq!(read_gate_toml(None), Err(vec![GateTomlError::FileMissing]));
    assert_eq!(GateTomlError::FileMissing.code(), "file_missing");
}

#[test]
fn a_file_that_is_not_toml_is_toml_invalid() {
    assert_eq!(codes("format = 1\nowners = [\"a\"\n"), ["toml_invalid"]);
    assert_eq!(codes(&format!("{HEAD}format = 2\n")), ["toml_invalid"]);
}

#[test]
fn a_format_other_than_1_is_format_unknown() {
    for format in ["format = 2", "format = \"1\"", "format = 1.0", ""] {
        let source = HEAD.replace("format = 1", format);
        assert_eq!(codes(&source), ["format_unknown"], "{format:?}");
    }
}

#[test]
fn a_default_other_than_owner_or_pm_is_default_invalid() {
    for default in ["default = \"Owner\"", "default = \"\"", "default = 1", ""] {
        let source = HEAD.replace("default = \"owner\"", default);
        assert_eq!(codes(&source), ["default_invalid"], "{default:?}");
    }
}

#[test]
fn empty_or_missing_owners_are_owners_empty() {
    for owners in [
        "owners = []",
        "",
        "owners = \"cabcookie\"",
        "owners = [\"\"]",
        "owners = [1]",
    ] {
        let source = HEAD.replace("owners = [\"cabcookie\"]", owners);
        assert_eq!(codes(&source), ["owners_empty"], "{owners:?}");
    }
}

#[test]
fn empty_or_missing_verdict_authors_are_verdict_authors_empty() {
    for authors in ["verdict_authors = []", "", "verdict_authors = [\"\"]"] {
        let source = HEAD.replace("verdict_authors = [\"nexusflow-ug\"]", authors);
        assert_eq!(codes(&source), ["verdict_authors_empty"], "{authors:?}");
    }
}

#[test]
fn an_id_off_the_pattern_is_rule_id_invalid() {
    let too_long = format!("a{}", "b".repeat(63));
    for bad in [
        "Upper",
        "1st",
        "-dash",
        "has.dot",
        "has_underscore",
        "",
        too_long.as_str(),
    ] {
        let source = HEAD.to_string() + &rule_toml(bad, "owner", "A rule.");
        assert_eq!(
            errors(&source),
            [GateTomlError::RuleIdInvalid(id(bad))],
            "{bad:?}"
        );
    }
    let longest = format!("a{}", "b".repeat(62));
    let source = HEAD.to_string() + &rule_toml(&longest, "owner", "A rule.");
    assert!(read_gate_toml(Some(&source)).is_ok(), "64 characters fit");
    assert_eq!(
        GateTomlError::RuleIdInvalid(id("x")).code(),
        "rule_id_invalid"
    );
}

#[test]
fn a_rule_without_a_string_id_is_named_by_its_position() {
    let source = format!(
        "{HEAD}{}\n[[rule]]\neffect = \"pm\"\ntext = \"No id.\"\n\n[[rule]]\nid = 7\neffect = \"pm\"\ntext = \"A number.\"\n",
        rule_toml("first", "owner", "A rule.")
    );
    assert_eq!(
        errors(&source),
        [
            GateTomlError::RuleIdInvalid(RuleRef::Position(2)),
            GateTomlError::RuleIdInvalid(RuleRef::Position(3)),
        ]
    );
}

#[test]
fn an_id_that_repeats_is_rule_id_duplicate() {
    let source = HEAD.to_string()
        + &rule_toml("same", "owner", "One.")
        + &rule_toml("other", "pm", "Two.")
        + &rule_toml("same", "pm", "Three.");
    assert_eq!(
        errors(&source),
        [GateTomlError::RuleIdDuplicate(id("same"))]
    );
    assert_eq!(
        GateTomlError::RuleIdDuplicate(id("x")).code(),
        "rule_id_duplicate"
    );
}

#[test]
fn an_effect_other_than_owner_or_pm_is_effect_invalid() {
    for effect in ["Owner", "pm ", "", "maybe"] {
        let source = HEAD.to_string() + &rule_toml("a-rule", effect, "A rule.");
        assert_eq!(
            errors(&source),
            [GateTomlError::EffectInvalid(id("a-rule"))],
            "{effect:?}"
        );
    }
    let missing = format!("{HEAD}\n[[rule]]\nid = \"a-rule\"\ntext = \"A rule.\"\n");
    assert_eq!(codes(&missing), ["effect_invalid"]);
}

#[test]
fn an_empty_or_missing_text_is_text_empty() {
    for text in ["", "  \\n\\t "] {
        let source = HEAD.to_string() + &rule_toml("a-rule", "owner", text);
        assert_eq!(
            errors(&source),
            [GateTomlError::TextEmpty(id("a-rule"))],
            "{text:?}"
        );
    }
    let missing = format!("{HEAD}\n[[rule]]\nid = \"a-rule\"\neffect = \"pm\"\n");
    assert_eq!(codes(&missing), ["text_empty"]);
    let number = format!("{HEAD}\n[[rule]]\nid = \"a-rule\"\neffect = \"pm\"\ntext = 3\n");
    assert_eq!(codes(&number), ["text_empty"]);
}

#[test]
fn every_problem_is_named_in_the_order_of_the_file() {
    let source = "format = 2\ndefault = \"nobody\"\nowners = []\nverdict_authors = []\n"
        .to_string()
        + &rule_toml("Bad", "owner", "One.")
        + &rule_toml("ok", "boss", "")
        + &rule_toml("ok", "pm", "Three.");
    assert_eq!(
        codes(&source),
        [
            "format_unknown",
            "default_invalid",
            "owners_empty",
            "verdict_authors_empty",
            "rule_id_invalid",
            "effect_invalid",
            "text_empty",
            "rule_id_duplicate",
        ]
    );
}

#[test]
fn a_rule_error_names_its_rule() {
    assert_eq!(
        GateTomlError::TextEmpty(id("a-rule")).rule(),
        Some(&id("a-rule"))
    );
    assert_eq!(
        GateTomlError::EffectInvalid(RuleRef::Position(4)).rule(),
        Some(&RuleRef::Position(4))
    );
    assert_eq!(GateTomlError::OwnersEmpty.rule(), None);
}

#[test]
fn each_settings_error_has_its_code() {
    let all = [
        (GateTomlError::FileMissing, "file_missing"),
        (GateTomlError::TomlInvalid, "toml_invalid"),
        (GateTomlError::FormatUnknown, "format_unknown"),
        (GateTomlError::DefaultInvalid, "default_invalid"),
        (GateTomlError::OwnersEmpty, "owners_empty"),
        (GateTomlError::VerdictAuthorsEmpty, "verdict_authors_empty"),
        (GateTomlError::EffectInvalid(id("x")), "effect_invalid"),
        (GateTomlError::TextEmpty(id("x")), "text_empty"),
    ];
    for (error, code) in all {
        assert_eq!(error.code(), code);
    }
}

#[test]
fn rule_that_is_not_a_list_of_tables_is_toml_invalid() {
    assert_eq!(codes(&format!("{HEAD}rule = 3\n")), ["toml_invalid"]);
    assert_eq!(codes(&format!("{HEAD}rule = [1, 2]\n")), ["toml_invalid"]);
}
