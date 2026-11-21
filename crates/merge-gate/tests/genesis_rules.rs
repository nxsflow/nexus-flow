//! NEX-169: the rules on what comes back to the owner are the list after the mark in GENESIS.md
//! (RFC 0010, "The rules from GENESIS"; RFC 0004, "The rules on what comes back to the owner").

use nexus_flow_merge_gate::genesis_rules::{read_genesis_rules, GenesisError, GenesisRule};

const MARK: &str = "<!-- nexus-flow: owner-rules -->";

fn rule(n: usize, text: &str) -> GenesisRule {
    GenesisRule {
        id: format!("genesis.{n}"),
        text: text.to_string(),
    }
}

#[test]
fn each_top_level_item_of_the_list_after_the_mark_is_a_rule_in_order() {
    let genesis = format!(
        "# Product\n\n## What has to come back through me\n\n{MARK}\n- Anything a user sees first.\n- A change to a stored format.\n\nMore text.\n"
    );
    assert_eq!(
        read_genesis_rules(Some(&genesis)),
        Ok(vec![
            rule(1, "Anything a user sees first."),
            rule(2, "A change to a stored format."),
        ])
    );
}

#[test]
fn blank_lines_may_stand_between_the_mark_and_the_list() {
    let genesis = format!("{MARK}\n\n  \n* The only rule.\n");
    assert_eq!(
        read_genesis_rules(Some(&genesis)),
        Ok(vec![rule(1, "The only rule.")])
    );
}

#[test]
fn a_missing_file_has_no_readable_rules() {
    assert_eq!(read_genesis_rules(None), Err(GenesisError::FileMissing));
    assert_eq!(GenesisError::FileMissing.code(), "file_missing");
}

#[test]
fn a_file_without_the_mark_has_no_readable_rules() {
    let genesis = "## What has to come back through me\n\n- A rule nobody marked.\n";
    assert_eq!(
        read_genesis_rules(Some(genesis)),
        Err(GenesisError::MarkMissing)
    );
    assert_eq!(GenesisError::MarkMissing.code(), "mark_missing");
}

#[test]
fn a_mark_that_must_read_exactly_is_missing_when_it_differs() {
    let genesis = "<!-- nexus-flow: owner-rules (old) -->\n- A rule.\n";
    assert_eq!(
        read_genesis_rules(Some(genesis)),
        Err(GenesisError::MarkMissing)
    );
}

#[test]
fn a_doubled_mark_has_no_readable_rules() {
    let genesis = format!("{MARK}\n- First list.\n\n   {MARK}  \n- Second list.\n");
    assert_eq!(
        read_genesis_rules(Some(&genesis)),
        Err(GenesisError::MarkDoubled)
    );
    assert_eq!(GenesisError::MarkDoubled.code(), "mark_doubled");
}

#[test]
fn a_paragraph_between_the_mark_and_the_list_gives_list_missing() {
    let genesis = format!("{MARK}\nThese are the rules:\n\n- A rule further down.\n");
    assert_eq!(
        read_genesis_rules(Some(&genesis)),
        Err(GenesisError::ListMissing)
    );
    assert_eq!(GenesisError::ListMissing.code(), "list_missing");
}

#[test]
fn a_heading_after_the_mark_gives_list_missing() {
    let genesis = format!("{MARK}\n\n## Next section\n\n- Not a rule.\n");
    assert_eq!(
        read_genesis_rules(Some(&genesis)),
        Err(GenesisError::ListMissing)
    );
}

#[test]
fn a_mark_at_the_end_of_the_file_gives_list_missing() {
    let genesis = format!("- Before the mark.\n\n{MARK}\n\n");
    assert_eq!(
        read_genesis_rules(Some(&genesis)),
        Err(GenesisError::ListMissing)
    );
}

#[test]
fn a_mark_inside_a_code_fence_is_ignored() {
    let genesis = format!(
        "The guide shows the mark:\n\n```markdown\n{MARK}\n- An example rule.\n```\n\n~~~\n{MARK}\n~~~\n\n{MARK}\n- The real rule.\n"
    );
    assert_eq!(
        read_genesis_rules(Some(&genesis)),
        Ok(vec![rule(1, "The real rule.")])
    );
}

#[test]
fn a_file_whose_only_mark_is_in_a_code_fence_has_no_mark() {
    let genesis = format!("```\n{MARK}\n```\n\n- Not a rule.\n");
    assert_eq!(
        read_genesis_rules(Some(&genesis)),
        Err(GenesisError::MarkMissing)
    );
}
