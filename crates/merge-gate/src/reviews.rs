//! The pull request's reviews, as the gate reads them (RFC 0010, "The owner's approval").
//!
//! The reviews are GitHub's "list reviews for a pull request" response, all pages as one JSON
//! array, of which the gate reads `user.login`, `state`, `commit_id` and `submitted_at`. A login's
//! deciding reviews are those in the state `APPROVED`, `CHANGES_REQUESTED` or `DISMISSED`; its
//! latest is the one with the greatest `submitted_at`, and then the greatest position in the array.
//! Logins are compared without regard to ASCII case.

use serde_json::Value;

use crate::verdict_comments::is_utc_timestamp;

/// A deciding review's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Approved,
    ChangesRequested,
    Dismissed,
}

/// One deciding review: who, what, on which commit, and when.
#[derive(Debug, Clone)]
struct Deciding {
    login: String,
    state: State,
    commit: Option<String>,
    submitted_at: String,
}

/// The deciding reviews of a pull request, in the order the forge listed them.
#[derive(Debug, Clone, Default)]
pub struct Reviews {
    deciding: Vec<Deciding>,
}

impl Reviews {
    /// The reviews in `bytes`; the whole file is refused, one line per problem, when it is not a
    /// JSON array or an entry lacks what the gate reads.
    pub fn read(bytes: &[u8]) -> Result<Reviews, Vec<String>> {
        let not_reviews = || vec!["the reviews are not a JSON array of reviews".to_string()];
        let value: Value = serde_json::from_slice(bytes).map_err(|_| not_reviews())?;
        let array = value.as_array().ok_or_else(not_reviews)?;
        let mut deciding = Vec::new();
        let mut problems = Vec::new();
        for (i, item) in array.iter().enumerate() {
            match read_review(item) {
                Some(Some(review)) => deciding.push(review),
                Some(None) => {}
                None => problems.push(format!(
                    "review {} of the reviews lacks a user, a string state, or, for a deciding \
                     review, a commit_id and a submitted_at in YYYY-MM-DDTHH:MM:SSZ",
                    i + 1
                )),
            }
        }
        if problems.is_empty() {
            Ok(Reviews { deciding })
        } else {
            Err(problems)
        }
    }

    /// The latest deciding review of each login in `owners`, in the order of `owners`, with the
    /// login as the review names it. A login listed twice is looked up once.
    fn latest_of_owners<'a>(&'a self, owners: &'a [String]) -> impl Iterator<Item = &'a Deciding> {
        owners.iter().enumerate().filter_map(move |(i, owner)| {
            if owners[..i].iter().any(|o| o.eq_ignore_ascii_case(owner)) {
                return None;
            }
            self.deciding
                .iter()
                .enumerate()
                .filter(|(_, r)| r.login.eq_ignore_ascii_case(owner))
                .max_by(|(ia, a), (ib, b)| (&a.submitted_at, ia).cmp(&(&b.submitted_at, ib)))
                .map(|(_, r)| r)
        })
    }

    /// Whether an owner's latest deciding review approves `head`.
    pub fn owner_approved(&self, owners: &[String], head: &str) -> bool {
        self.latest_of_owners(owners)
            .any(|r| r.state == State::Approved && r.commit.as_deref() == Some(head))
    }

    /// The owners whose latest deciding review requests changes, on any commit.
    pub fn changes_requested_by(&self, owners: &[String]) -> Vec<String> {
        self.latest_of_owners(owners)
            .filter(|r| r.state == State::ChangesRequested)
            .map(|r| r.login.clone())
            .collect()
    }
}

/// `Some(Some(_))` for a deciding review, `Some(None)` for another one or one by no user, `None`
/// for an entry the gate cannot read.
fn read_review(item: &Value) -> Option<Option<Deciding>> {
    let login = match item.get("user")? {
        Value::Null => None,
        user => Some(user.get("login")?.as_str()?.to_string()),
    };
    let state = match item.get("state")?.as_str()? {
        "APPROVED" => State::Approved,
        "CHANGES_REQUESTED" => State::ChangesRequested,
        "DISMISSED" => State::Dismissed,
        _ => return Some(None),
    };
    let commit = match item.get("commit_id")? {
        Value::Null => None,
        commit => Some(commit.as_str()?.to_string()),
    };
    let submitted_at = item.get("submitted_at")?.as_str()?;
    if !is_utc_timestamp(submitted_at) {
        return None;
    }
    Some(login.map(|login| Deciding {
        login,
        state,
        commit,
        submitted_at: submitted_at.to_string(),
    }))
}
