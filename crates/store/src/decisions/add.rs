//! Writing a decision (RFC 0003, "Writing a decision"): `add` checks what the writer gives, then
//! writes the decision with its topics, subjects and supersessions through the store's one write
//! path, as one change of the repository space, so they arrive together on every member or not
//! at all. Every check runs before the write, and a refusal writes nothing.
//!
//! `decidedBy` defaults to the writer's role from its role record (RFC 0001, "The role record"):
//! `pm`, or `coder on <ticket key>` for a member started on a ticket. `author` is always the
//! writer's member id.
//!
//! A subject is written with its label alone; `ticket` and `elsewhere` are left for the reads
//! that resolve a label. The handle is not unique (RFC 0003, "Handles"), so a taken handle is
//! written all the same; [`with_handle`] finds every decision of a handle, which is how a writer
//! learns that its handle was taken already.

use nocentraldb::{MemberId, Object, ObjectId, Space, Value};

use super::date::{is_date, today};
use super::handle::{handle, is_slug, slug};
use super::labels::expand;
use super::{Decision, DecisionError, Linked, NewDecision, Subject, SubjectKind};
use crate::id::SpaceId;
use crate::members::record;
use crate::opened::Opened;

const DECISION: &str = "decisionsDecision";
const TOPIC: &str = "decisionsTopic";
const SUBJECT: &str = "decisionsSubject";
const SUPERSESSION: &str = "decisionsSupersession";
const BY_HANDLE: &str = "byHandle";
const TITLE_MAX: usize = 200;

/// Checks `new` and writes it into the repository space of `opened`, as one change, with its
/// topics, subjects and supersessions; returns it in full.
pub async fn add(opened: &Opened, new: NewDecision) -> Result<Decision, DecisionError> {
    let checked = check(new)?;
    let (space_id, space) = repository(opened)?;
    let supersedes = superseded(opened, &space_id, &checked).await?;
    let me = me(space).await?;
    let decided_by = match checked.decided_by.clone() {
        Some(given) => given,
        None => role(opened, space, me).await?,
    };
    let author = me.to_string();
    let mut decision_id = None;
    opened
        .write(&space_id, |w| {
            let mut values = vec![
                ("handle", Value::from(checked.handle.as_str())),
                ("date", Value::from(checked.date.as_str())),
                ("tickets", Value::from(checked.tickets.as_str())),
                ("title", Value::from(checked.title.as_str())),
                ("decidedBy", Value::from(decided_by.as_str())),
                ("why", Value::from(checked.why.as_str())),
                ("insteadOf", Value::from(checked.instead_of.as_str())),
                ("body", Value::from(checked.body.as_str())),
                ("author", Value::from(author.as_str())),
            ];
            if let Some(pr) = &checked.pr {
                values.push(("pr", Value::from(pr.as_str())));
            }
            let id = w.create(DECISION, values)?;
            decision_id = Some(id);
            let date = || Value::from(checked.date.as_str());
            for topic in &checked.topics {
                let values = [
                    ("decision", Value::from(id)),
                    ("topic", Value::from(topic.as_str())),
                    ("date", date()),
                ];
                w.create(TOPIC, values)?;
            }
            for label in &checked.subjects {
                let values = [
                    ("decision", Value::from(id)),
                    ("label", Value::from(label.as_str())),
                    ("date", date()),
                ];
                w.create(SUBJECT, values)?;
            }
            for (older, _) in &supersedes {
                let values = [
                    ("decision", Value::from(id)),
                    ("supersedes", Value::from(*older)),
                    ("date", date()),
                ];
                w.create(SUPERSESSION, values)?;
            }
            Ok(())
        })
        .await?;
    let id = decision_id.expect("the decision is created in the change");
    let on_main = checked.pr.is_none().then_some(true);
    Ok(Decision {
        id: id.to_string(),
        handle: checked.handle,
        date: checked.date,
        tickets: checked.tickets,
        subjects: checked
            .subjects
            .into_iter()
            .map(|label| Subject {
                label,
                kind: SubjectKind::Ticket,
                space: None,
                object: None,
                status: None,
            })
            .collect(),
        title: checked.title,
        decided_by,
        why: checked.why,
        instead_of: checked.instead_of,
        topics: checked.topics,
        body: checked.body,
        supersedes: supersedes.into_iter().map(|(_, linked)| linked).collect(),
        superseded_by: Vec::new(),
        in_force: true,
        pr: checked.pr,
        on_main,
        author,
        edited: false,
        deleted: false,
    })
}

/// The object ids of every decision of the repository space with the handle `handle`.
pub async fn with_handle(opened: &Opened, handle: &str) -> Result<Vec<String>, DecisionError> {
    let (space_id, _) = repository(opened)?;
    Ok(by_handle(opened, &space_id, handle)
        .await?
        .iter()
        .map(|o| o.id().to_string())
        .collect())
}

/// A new decision after the checks that need no database.
struct Checked {
    title: String,
    why: String,
    instead_of: String,
    topics: Vec<String>,
    tickets: String,
    subjects: Vec<String>,
    supersedes: Vec<String>,
    pr: Option<String>,
    decided_by: Option<String>,
    date: String,
    handle: String,
    body: String,
}

fn check(new: NewDecision) -> Result<Checked, DecisionError> {
    let title = required(&new.title, "title")?;
    if title.contains(['\n', '\r']) {
        return Err(DecisionError::TitleNotOneLine);
    }
    let chars = title.chars().count();
    if chars > TITLE_MAX {
        return Err(DecisionError::TitleTooLong { chars });
    }
    let why = required(&new.why, "why")?;
    let instead_of = required(&new.instead_of, "instead of")?;
    if new.topics.is_empty() {
        return Err(DecisionError::NoTopic);
    }
    let mut topics: Vec<String> = Vec::new();
    for topic in &new.topics {
        if !is_topic(topic) {
            return Err(DecisionError::NotATopic {
                text: topic.clone(),
            });
        }
        if !topics.contains(topic) {
            topics.push(topic.clone());
        }
    }
    let decided_by = new
        .decided_by
        .as_deref()
        .map(|given| required(given, "decided by"))
        .transpose()?;
    let date = match new.date {
        Some(date) if is_date(&date) => date,
        Some(text) => return Err(DecisionError::NotADate { text }),
        None => today(),
    };
    let mut subjects: Vec<String> = Vec::new();
    for label in &new.tickets {
        for id in expand(label)? {
            if !subjects.contains(&id) {
                subjects.push(id);
            }
        }
    }
    if let Some(pr) = &new.pr {
        if !is_https(pr) {
            return Err(DecisionError::NotAnHttpsUrl { text: pr.clone() });
        }
    }
    let slug = match new.slug {
        Some(given) if is_slug(&given) => given,
        Some(text) => return Err(DecisionError::NotASlug { text }),
        None => slug(&title).ok_or(DecisionError::NoSlug)?,
    };
    let handle = handle(&date, subjects.first().map(String::as_str), &slug);
    let tickets = if new.tickets.is_empty() {
        "no ticket".to_owned()
    } else {
        new.tickets.join(", ")
    };
    Ok(Checked {
        title,
        why,
        instead_of,
        topics,
        tickets,
        subjects,
        supersedes: new.supersedes,
        pr: new.pr,
        decided_by,
        date,
        handle,
        body: new.body.unwrap_or_default(),
    })
}

/// `text` without surrounding whitespace, refused when nothing is left.
fn required(text: &str, field: &'static str) -> Result<String, DecisionError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(DecisionError::Missing { field });
    }
    Ok(text.to_owned())
}

/// A topic: a lowercase letter or digit, then lowercase letters, digits or `-`.
fn is_topic(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && bytes.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// An `https` URL: the scheme, then a host, and no whitespace.
fn is_https(text: &str) -> bool {
    text.strip_prefix("https://").is_some_and(|rest| {
        rest.chars().next().is_some_and(|c| c != '/') && !rest.contains(char::is_whitespace)
    })
}

/// The repository space: the first open space, which must be held.
fn repository(opened: &Opened) -> Result<(SpaceId, &Space), DecisionError> {
    match opened.spaces().first() {
        Some((id, Some(space))) => Ok((*id, space)),
        Some((id, None)) => Err(DecisionError::NotHeld { space: *id }),
        None => Err(DecisionError::Database {
            message: "no repository space is open".to_owned(),
        }),
    }
}

/// The decisions `checked` supersedes, each found in the local copy and made on or before its
/// date, with its object id, in date order.
async fn superseded(
    opened: &Opened,
    space: &SpaceId,
    checked: &Checked,
) -> Result<Vec<(ObjectId, Linked)>, DecisionError> {
    let mut found: Vec<(String, ObjectId, Linked)> = Vec::new();
    for key in &checked.supersedes {
        if *key == checked.handle {
            return Err(DecisionError::SupersedesItself);
        }
        let object = find(opened, space, key).await?;
        let date = text(&object, "date")?;
        if date > checked.date {
            return Err(DecisionError::SupersedesLater {
                key: key.clone(),
                date,
            });
        }
        let linked = Linked {
            id: object.id().to_string(),
            handle: text(&object, "handle")?,
        };
        if !found.iter().any(|(_, id, _)| *id == object.id()) {
            found.push((date, object.id(), linked));
        }
    }
    found.sort_by(|a, b| (&a.0, &a.2.handle).cmp(&(&b.0, &b.2.handle)));
    Ok(found.into_iter().map(|(_, id, l)| (id, l)).collect())
}

/// The decision `key` names, by object id or by handle.
async fn find(opened: &Opened, space: &SpaceId, key: &str) -> Result<Object, DecisionError> {
    let unknown = || DecisionError::SupersedesUnknown {
        key: key.to_owned(),
    };
    if let Ok(id) = key.parse::<ObjectId>() {
        return match opened.get(space, DECISION, id).await? {
            Some(object) if !object.deleted() => Ok(object),
            _ => Err(unknown()),
        };
    }
    let mut matches = by_handle(opened, space, key).await?;
    match matches.len() {
        0 => Err(unknown()),
        1 => Ok(matches.remove(0)),
        _ => Err(DecisionError::SupersedesAmbiguous {
            handle: key.to_owned(),
            ids: matches.iter().map(|o| o.id().to_string()).collect(),
        }),
    }
}

async fn by_handle(
    opened: &Opened,
    space: &SpaceId,
    handle: &str,
) -> Result<Vec<Object>, DecisionError> {
    Ok(opened
        .find(space, DECISION, BY_HANDLE, vec![Value::from(handle)])
        .await?)
}

fn text(object: &Object, field: &str) -> Result<String, DecisionError> {
    Ok(object.get::<&str>(field)?.unwrap_or_default().to_owned())
}

/// This database's member in `space`.
async fn me(space: &Space) -> Result<MemberId, DecisionError> {
    space
        .members()
        .list()
        .await?
        .into_iter()
        .find(|m| m.is_self)
        .map(|m| m.id)
        .ok_or_else(|| DecisionError::Database {
            message: "this database's member is not a member of the repository space".to_owned(),
        })
}

/// Who decided, by the role record of `me`: its role, and `on <ticket key>` when it names a
/// ticket. A record without a role, such as one only `nxs member retag` wrote, does not say it.
async fn role(opened: &Opened, space: &Space, me: MemberId) -> Result<String, DecisionError> {
    let record = record::read(opened.db(), space, me)
        .await
        .map_err(|e| DecisionError::Database {
            message: e.to_string(),
        })?
        .ok_or(DecisionError::NoRole)?;
    let role = record
        .role
        .filter(|role| !role.trim().is_empty())
        .ok_or(DecisionError::NoRole)?;
    let Some(ticket) = record.ticket else {
        return Ok(role);
    };
    let key = opened
        .get(&ticket.space, "ticketsTicket", ticket.object)
        .await?
        .and_then(|t| t.get::<&str>("key").ok().flatten().map(str::to_owned))
        .ok_or(DecisionError::NoRole)?;
    Ok(format!("{role} on {key}"))
}
