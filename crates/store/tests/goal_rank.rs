//! The rank of a goal in focus (RFC 0005, "The order of the goals in focus"): a string of `0-9a-z`,
//! compared by its bytes, strictly between its neighbours, never ending in `0`, and only as long as
//! it must be.
//!
//! The property tests draw their cases from a seeded generator in this file, so a failure
//! reproduces from its seed and the crate needs no property-testing dependency.

use nexus_flow_store::goals::rank::{rank_between, Place, RankError};

const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// SplitMix64: a small, seeded generator, enough to draw test cases.
struct Cases(u64);

impl Cases {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A valid rank of 1 to 6 characters. Low and high digits are drawn more often, so neighbours
    /// that share a prefix, sit next to each other, or end in `z` come up often.
    fn rank(&mut self) -> String {
        let len = 1 + self.below(6) as usize;
        let mut out: Vec<u8> = (0..len)
            .map(|_| match self.below(4) {
                0 => DIGITS[self.below(2) as usize],
                1 => DIGITS[34 + self.below(2) as usize],
                _ => DIGITS[self.below(36) as usize],
            })
            .collect();
        if out[len - 1] == b'0' {
            out[len - 1] = DIGITS[1 + self.below(35) as usize];
        }
        String::from_utf8(out).unwrap()
    }
}

fn digit(c: u8) -> u8 {
    DIGITS.iter().position(|&d| d == c).unwrap() as u8
}

/// The lowest valid rank of at most `len` characters strictly above `a` (the empty string when
/// there is no lower neighbour), or `None` when there is none. Written independently of the code
/// under test, to tell how short a rank between two neighbours can be.
fn lowest_above(a: &str, len: usize) -> Option<String> {
    let a: Vec<u8> = a.bytes().map(digit).collect();
    let out: Vec<u8> = if a.len() < len {
        // Every extension of `a` lies above it; the lowest valid one of at most `len` characters
        // pads with zeros and ends in `1`.
        let mut out = a.clone();
        out.resize(len - 1, 0);
        out.push(1);
        out
    } else {
        // No prefix of `a` lies above it: raise the last digit below `z` within the first `len`.
        let p = a[..len].iter().rposition(|&d| d < 35)?;
        let mut out = a[..p].to_vec();
        out.push(a[p] + 1);
        out
    };
    Some(
        out.into_iter()
            .map(|d| DIGITS[d as usize] as char)
            .collect(),
    )
}

/// The length of the shortest valid rank strictly between `before` and `after`.
fn shortest_len(before: Option<&str>, after: Option<&str>) -> usize {
    (1..)
        .find(|&len| match lowest_above(before.unwrap_or(""), len) {
            Some(r) => after.is_none_or(|b| r.as_str() < b),
            None => false,
        })
        .unwrap()
}

fn assert_rank_between(before: Option<&str>, after: Option<&str>, context: &str) -> String {
    let r = rank_between(before, after).unwrap_or_else(|e| panic!("{context}: {e:?}"));
    assert!(!r.is_empty(), "{context}: empty rank");
    assert!(
        r.bytes().all(|c| DIGITS.contains(&c)),
        "{context}: {r:?} leaves 0-9a-z"
    );
    assert!(!r.ends_with('0'), "{context}: {r:?} ends in 0");
    if let Some(a) = before {
        assert!(
            a.as_bytes() < r.as_bytes(),
            "{context}: {r:?} is not above {a:?}"
        );
    }
    if let Some(b) = after {
        assert!(
            r.as_bytes() < b.as_bytes(),
            "{context}: {r:?} is not below {b:?}"
        );
    }
    let shortest = shortest_len(before, after);
    assert_eq!(
        r.len(),
        shortest,
        "{context}: {r:?} is longer than it must be"
    );
    r
}

#[test]
fn a_rank_lies_strictly_between_any_two_ranks_and_never_ends_in_0() {
    for seed in 0..20_000u64 {
        let mut cases = Cases(seed);
        let (x, y) = (cases.rank(), cases.rank());
        if x == y {
            continue;
        }
        let (a, b) = if x < y { (x, y) } else { (y, x) };
        assert_rank_between(
            Some(&a),
            Some(&b),
            &format!("seed {seed}: between {a:?} and {b:?}"),
        );
    }
}

#[test]
fn a_rank_lies_beyond_a_single_neighbour() {
    for seed in 0..10_000u64 {
        let mut cases = Cases(seed);
        let a = cases.rank();
        assert_rank_between(Some(&a), None, &format!("seed {seed}: after {a:?}"));
        assert_rank_between(None, Some(&a), &format!("seed {seed}: before {a:?}"));
    }
}

#[test]
fn the_first_goal_in_focus_gets_a_one_character_rank() {
    let r = assert_rank_between(None, None, "an empty order");
    assert_eq!(r.len(), 1);
}

#[test]
fn neighbours_that_leave_no_room_at_their_length_give_a_longer_rank() {
    for (a, b) in [
        ("1", "2"),
        ("1", "101"),
        ("1z", "2"),
        ("zz", "zzz1"),
        ("y", "z"),
    ] {
        assert_rank_between(Some(a), Some(b), &format!("between {a:?} and {b:?}"));
    }
    assert_rank_between(Some("z"), None, "after \"z\"");
    assert_rank_between(None, Some("1"), "before \"1\"");
    assert_rank_between(None, Some("01"), "before \"01\"");
}

#[test]
fn inserting_at_the_front_again_and_again_grows_the_rank_only_when_no_shorter_one_fits() {
    let mut first = assert_rank_between(None, None, "an empty order");
    let mut lengths = vec![first.len()];
    for step in 1..=500 {
        let r = assert_rank_between(
            None,
            Some(&first),
            &format!("step {step}: before {first:?}"),
        );
        assert!(
            r.len() <= first.len() + 1,
            "step {step}: {r:?} grew by more than one"
        );
        lengths.push(r.len());
        first = r;
    }
    // The first five moves to the front still find a one-character rank below the last.
    assert_eq!(lengths[..6], [1; 6]);
    assert_eq!(lengths[6], 2);
}

#[test]
fn inserting_at_the_back_again_and_again_grows_the_rank_only_when_no_shorter_one_fits() {
    let mut last = assert_rank_between(None, None, "an empty order");
    for step in 1..=500 {
        let r = assert_rank_between(Some(&last), None, &format!("step {step}: after {last:?}"));
        assert!(
            r.len() <= last.len() + 1,
            "step {step}: {r:?} grew by more than one"
        );
        last = r;
    }
}

#[test]
fn a_malformed_neighbour_is_refused() {
    for bad in ["", "1A", "a-b", "10", "é"] {
        assert_eq!(
            rank_between(Some(bad), None),
            Err(RankError::Malformed(bad.to_owned())),
            "after {bad:?}"
        );
        assert_eq!(
            rank_between(None, Some(bad)),
            Err(RankError::Malformed(bad.to_owned())),
            "before {bad:?}"
        );
    }
}

#[test]
fn neighbours_out_of_order_are_refused() {
    for (a, b) in [("2", "1"), ("1", "1"), ("1a", "1")] {
        assert_eq!(
            rank_between(Some(a), Some(b)),
            Err(RankError::NotOrdered {
                before: a.to_owned(),
                after: b.to_owned()
            }),
            "between {a:?} and {b:?}"
        );
    }
}

#[test]
fn goals_in_focus_order_by_rank_then_key_then_object_id() {
    let place = |rank, key, object_id| Place {
        rank,
        key,
        object_id,
    };
    let mut order = vec![
        place("i", "cb.1", "0001"),
        place("9", "cb.9", "0009"),
        place("9", "ab.4", "0400"),
        place("9", "ab.4", "0300"),
        place("1", "zz.7", "0700"),
    ];
    order.sort();
    let keys: Vec<_> = order.iter().map(|p| (p.key, p.object_id)).collect();
    assert_eq!(
        keys,
        [
            ("zz.7", "0700"),
            ("ab.4", "0300"),
            ("ab.4", "0400"),
            ("cb.9", "0009"),
            ("cb.1", "0001")
        ]
    );
}
