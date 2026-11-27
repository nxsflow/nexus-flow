//! A member's ticket tag (RFC 0002, "Ticket ids"): two characters, a lower-case ASCII letter and
//! then a letter or a digit, 936 in all. A tag is picked from those that are free, spread by the
//! new member's id so that two inviters picking offline at the same time rarely pick the same one.

use std::collections::BTreeSet;

use crate::members::tags::{self, Tag};

#[test]
fn a_tag_is_a_letter_then_a_letter_or_a_digit() {
    for good in ["ka", "p7", "a0", "zz"] {
        let tag: Tag = good.parse().unwrap_or_else(|e| panic!("{good}: {e:?}"));
        assert_eq!(tag.as_str(), good);
        assert_eq!(tag.to_string(), good);
    }
    for bad in [
        "", "k", "kab", "7a", "Ka", "kA", "k-", "k ", " k", "é1", "k\u{e9}",
    ] {
        assert!(bad.parse::<Tag>().is_err(), "{bad:?} is no tag");
    }
}

#[test]
fn there_are_936_tags_in_byte_order() {
    let all: Vec<Tag> = tags::all().collect();

    assert_eq!(all.len(), 936);
    assert_eq!(all.first().map(Tag::as_str), Some("a0"));
    assert_eq!(all.last().map(Tag::as_str), Some("zz"));
    let mut sorted = all.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted, all);
}

#[test]
fn a_picked_tag_is_one_that_is_not_taken() {
    let taken: BTreeSet<Tag> = tags::all().filter(|t| t.as_str() != "q3").collect();

    for seed in [[0u8; 32], [7u8; 32], [255u8; 32]] {
        assert_eq!(
            tags::pick(&taken, &seed).map(|t| t.to_string()),
            Some("q3".into())
        );
    }
}

#[test]
fn the_pick_is_spread_by_the_seed_and_the_same_for_the_same_seed() {
    let taken = BTreeSet::new();
    let picks: BTreeSet<Tag> = (0u8..32)
        .map(|n| tags::pick(&taken, &[n; 32]).unwrap())
        .collect();

    assert!(picks.len() > 16, "{picks:?}");
    assert_eq!(tags::pick(&taken, &[9; 32]), tags::pick(&taken, &[9; 32]));
}

#[test]
fn with_every_tag_taken_none_is_picked() {
    let taken: BTreeSet<Tag> = tags::all().collect();

    assert_eq!(tags::pick(&taken, &[1; 32]), None);
}
