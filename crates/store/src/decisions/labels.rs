//! A decision's ticket labels (RFC 0003, "Writing a decision", "decisionsSubject"): each is a
//! ticket id, such as `NEX-22`, or a range of them, such as `NEX-6 to NEX-13`, as a heading in
//! `factory/decisions/` names them today. A range names every ticket in it, each of which becomes
//! a subject, while the decision's `tickets` keeps the range as written.

use super::DecisionError;

/// The ticket ids `label` names: itself, or every id of its range in order. A label that is
/// neither is refused.
pub fn expand(label: &str) -> Result<Vec<String>, DecisionError> {
    let refused = || DecisionError::NotATicket {
        text: label.to_owned(),
    };
    if let Some((from, to)) = label.split_once(" to ") {
        let (prefix, first) = split(from).ok_or_else(refused)?;
        let (prefix_to, last) = split(to).ok_or_else(refused)?;
        if prefix != prefix_to || first > last {
            return Err(refused());
        }
        return Ok((first..=last).map(|n| format!("{prefix}-{n}")).collect());
    }
    split(label).ok_or_else(refused)?;
    Ok(vec![label.to_owned()])
}

/// A ticket id like `ABC-22` into (`ABC`, 22): an uppercase letter, then uppercase letters or
/// digits, `-`, and digits.
fn split(text: &str) -> Option<(&str, u64)> {
    let (prefix, number) = text.rsplit_once('-')?;
    let mut chars = prefix.chars();
    let prefix_ok = chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    let number_ok = !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit());
    if !(prefix_ok && number_ok) {
        return None;
    }
    Some((prefix, number.parse().ok()?))
}
