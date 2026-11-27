//! RFC 0003, "Handles": a decision's handle is `YYYY-MM-DD-<ticket>-<slug>`. `<ticket>` is the
//! triggering ticket's label, or `no-ticket`; `<slug>` is the title in lowercase words joined by
//! `-`, cut at 60 characters on a word boundary, unless the writer gives one with `--slug`.

use nexus_flow_store::decisions::handle::{handle, slug};

#[test]
fn the_slug_is_the_title_in_lowercase_words_joined_by_a_dash() {
    assert_eq!(
        slug("One NoCentralDB space per repository, holds its tickets!"),
        Some("one-nocentraldb-space-per-repository-holds-its-tickets".to_owned())
    );
}

#[test]
fn the_slug_is_cut_at_60_characters_on_a_word_boundary() {
    let title = "A write-once object takes all its fields in its create, and never a later set";
    let cut = slug(title).unwrap();
    assert_eq!(
        cut,
        "a-write-once-object-takes-all-its-fields-in-its-create-and"
    );
    assert!(cut.chars().count() <= 60);
    let exactly_60 = format!("{} {}", "a".repeat(29), "b".repeat(30));
    assert_eq!(slug(&exactly_60).unwrap().len(), 60, "60 characters fit");
}

#[test]
fn a_first_word_longer_than_60_characters_is_cut_inside_it() {
    assert_eq!(slug(&"x".repeat(70)), Some("x".repeat(60)));
}

#[test]
fn a_title_without_a_letter_or_digit_has_no_slug() {
    assert_eq!(slug("— !? —"), None);
}

#[test]
fn the_handle_is_the_date_the_triggering_ticket_and_the_slug() {
    assert_eq!(
        handle("2026-11-15", Some("NEX-15"), "one-space-per-repository"),
        "2026-11-15-NEX-15-one-space-per-repository"
    );
    assert_eq!(
        handle("2026-11-15", None, "english-everywhere"),
        "2026-11-15-no-ticket-english-everywhere"
    );
}
