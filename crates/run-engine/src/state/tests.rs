use super::*;

#[test]
fn a_slot_timer_id_names_its_slot_back() {
    let slot = Slot {
        step: "build".into(),
        visit: 2,
        member: "code-quality".into(),
        attempt: 3,
    };
    let start = slot_timer(&TimerPurpose::StartDeadline, &slot);
    assert_eq!(start, "start_deadline/build/v2/code-quality/a3");
    assert_eq!(timer_slot(&start), Some(slot.clone()));
    let grace = grace_timer(2, &slot);
    assert_eq!(grace, "stop_grace/2/build/v2/code-quality/a3");
    assert_eq!(timer_slot(&grace), Some(slot));
    assert_eq!(timer_slot(ROOT_DEADLINE_TIMER), None);
    assert_eq!(timer_slot(TREE_WINDOW_TIMER), None);
}

#[test]
fn a_grace_timer_id_names_which_grace_it_is_and_no_other_timer_names_one() {
    let slot = Slot {
        step: "build".into(),
        visit: 1,
        member: "coder".into(),
        attempt: 1,
    };
    assert_eq!(grace_nth(&grace_timer(1, &slot)), Some(1));
    assert_eq!(grace_nth(&grace_timer(2, &slot)), Some(2));
    assert_eq!(grace_nth(&slot_timer(&TimerPurpose::Quiet, &slot)), None);
    assert_eq!(
        grace_nth(&slot_timer(&TimerPurpose::StopGrace, &slot)),
        None
    );
    assert_eq!(grace_nth(ROOT_DEADLINE_TIMER), None);
}
