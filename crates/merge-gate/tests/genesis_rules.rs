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
