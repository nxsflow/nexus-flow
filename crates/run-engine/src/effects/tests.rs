//! The stop table of §6.5, one test per row.

use nexus_run_contracts::StopKind::{Kill, Term};

use super::stop_signal;
use crate::state::GraceFired::{First, None as Not, Second};

#[test]
fn unforced_nothing_sent_owes_the_first_sigterm() {
    assert_eq!(stop_signal(false, 0, Not), Some(Term));
}

#[test]
fn unforced_one_sent_waits_for_the_first_grace() {
    assert_eq!(stop_signal(false, 1, Not), None);
}

#[test]
fn unforced_one_sent_after_the_first_grace_owes_the_second_sigterm() {
    assert_eq!(stop_signal(false, 1, First), Some(Term));
}

#[test]
fn unforced_two_sent_owes_nothing_in_the_second_grace_and_nothing_after_it() {
    assert_eq!(stop_signal(false, 2, First), None);
    assert_eq!(stop_signal(false, 2, Second), None);
}

#[test]
fn unforced_never_owes_a_kill_whatever_the_counts_say() {
    for sent in 0..=u8::MAX {
        for fired in [Not, First, Second] {
            assert_ne!(
                stop_signal(false, sent, fired),
                Some(Kill),
                "{sent} {fired:?}"
            );
        }
    }
}

#[test]
fn unforced_three_sent_owes_nothing() {
    for fired in [Not, First, Second] {
        assert_eq!(stop_signal(false, 3, fired), None);
    }
}

#[test]
fn forced_nothing_sent_owes_a_sigterm_and_never_skips_to_a_kill() {
    assert_eq!(stop_signal(true, 0, Not), Some(Term));
}

#[test]
fn forced_one_sent_waits_for_the_first_grace() {
    assert_eq!(stop_signal(true, 1, Not), None);
}

#[test]
fn forced_one_sent_after_the_first_grace_owes_the_kill() {
    assert_eq!(stop_signal(true, 1, First), Some(Kill));
}

#[test]
fn a_withdrawal_during_the_second_grace_waits_for_that_grace_to_end() {
    assert_eq!(stop_signal(true, 2, First), None);
}

#[test]
fn forced_two_sent_after_the_second_grace_owes_the_kill() {
    assert_eq!(stop_signal(true, 2, Second), Some(Kill));
}

#[test]
fn forced_three_sent_owes_nothing_more() {
    for fired in [Not, First, Second] {
        assert_eq!(stop_signal(true, 3, fired), None);
    }
}
