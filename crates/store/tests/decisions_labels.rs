//! RFC 0003, "Writing a decision" and "decisionsSubject": each ticket label is a ticket id, such
//! as `NEX-22`, or a range of them, such as `NEX-6 to NEX-13`, which names every ticket in it.

use nexus_flow_store::decisions::labels::expand;
use nexus_flow_store::decisions::DecisionError;

#[test]
fn a_ticket_id_names_itself() {
    assert_eq!(expand("NEX-22").unwrap(), ["NEX-22"]);
}

#[test]
fn a_range_names_every_ticket_in_it() {
    assert_eq!(
        expand("NEX-6 to NEX-13").unwrap(),
        ["NEX-6", "NEX-7", "NEX-8", "NEX-9", "NEX-10", "NEX-11", "NEX-12", "NEX-13"]
    );
}

#[test]
fn a_label_that_is_neither_is_refused_by_name() {
    for text in [
        "nex-22",
        "NEX",
        "NEX-",
        "22",
        "NEX-9, NEX-10",
        "NEX-13 to NEX-6",
        "NEX-6 to ABC-13",
        "no ticket",
        "",
    ] {
        assert_eq!(
            expand(text),
            Err(DecisionError::NotATicket {
                text: text.to_owned()
            }),
            "{text:?}"
        );
    }
}
