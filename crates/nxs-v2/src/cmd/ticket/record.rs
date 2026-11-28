//! A ticket as `nxs ticket show` prints it (RFC 0002, "Commands, JSON and the seam"): the
//! fields of a ticket in `list`, then those `show` adds, in the RFC's order, with snake_case
//! keys, `[]` for an empty list and `null` for an unset value, so every record has the same
//! keys. The Markdown gives a heading, the fields that are set, and the texts below.

use nexus_flow_seam::{Edge, Holder, Ticket};
use serde::Serialize;

use crate::output::Render;

/// A ticket as `show` prints it.
#[derive(Debug, Serialize)]
pub struct Record {
    id: String,
    key: String,
    #[serde(rename = "type")]
    kind: String,
    title: String,
    status: String,
    resolution: Option<String>,
    priority: Option<i64>,
    assignee: Option<HolderRow>,
    parent: Option<String>,
    labels: Vec<String>,
    due: Option<String>,
    defer_until: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    closed_at: Option<String>,
    archived_at: Option<String>,
    request: Option<RequestRow>,
    ready: Option<bool>,
    blocked: Option<bool>,
    deferred: Option<bool>,
    done: Option<bool>,
    on_main: Option<bool>,
    focus: Option<String>,
    focus_goal: Option<FocusGoalRow>,
    flags: Vec<String>,
    description: String,
    design: String,
    completion_criterion: String,
    closing_comment: Option<String>,
    replaced_by: Option<String>,
    replaces: Vec<String>,
    deps: Vec<EdgeRow>,
    dependents: Vec<EdgeRow>,
    children: Vec<String>,
    contributes_to: Vec<String>,
    mentions: Vec<String>,
    mentioned_by: Vec<String>,
    notes: Vec<NoteRow>,
    work: Vec<WorkRow>,
    references: Vec<ReferenceRow>,
}

#[derive(Debug, Serialize)]
struct HolderRow {
    kind: String,
    id: String,
    role: Option<String>,
}

#[derive(Debug, Serialize)]
struct RequestRow {
    id: String,
    key: String,
    relation: String,
    status: Option<String>,
}

#[derive(Debug, Serialize)]
struct FocusGoalRow {
    id: String,
    key: String,
    title: String,
    position: i64,
}

#[derive(Debug, Serialize)]
struct EdgeRow {
    key: String,
    status: String,
    done: Option<bool>,
    on_main: Option<bool>,
}

#[derive(Debug, Serialize)]
struct NoteRow {
    id: String,
    body: String,
    reply_to: Option<String>,
    created_at: Option<String>,
    created_by: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorkRow {
    kind: String,
    url: Option<String>,
    branch: Option<String>,
    state: Option<String>,
    commit: Option<String>,
    on_main: Option<bool>,
    created_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReferenceRow {
    kind: String,
    space: String,
    id: String,
    label: Option<String>,
    state: String,
}

fn holder(h: Holder) -> HolderRow {
    HolderRow {
        kind: h.kind,
        id: h.id,
        role: h.role,
    }
}

fn edges(list: Vec<Edge>) -> Vec<EdgeRow> {
    list.into_iter()
        .map(|e| EdgeRow {
            key: e.key,
            status: e.status,
            done: e.done,
            on_main: e.on_main,
        })
        .collect()
}

impl From<Ticket> for Record {
    fn from(t: Ticket) -> Self {
        let d = t.derived;
        Record {
            id: t.id,
            key: t.key,
            kind: t.kind,
            title: t.title,
            status: t.status,
            resolution: t.resolution,
            priority: t.priority,
            assignee: t.assignee.map(holder),
            parent: t.parent,
            labels: t.labels,
            due: t.due,
            defer_until: t.defer_until,
            created_at: t.created_at,
            updated_at: t.updated_at,
            closed_at: t.closed_at,
            archived_at: t.archived_at,
            request: t.request.map(|r| RequestRow {
                id: r.id,
                key: r.key,
                relation: r.relation,
                status: r.status,
            }),
            ready: d.ready,
            blocked: d.blocked,
            deferred: d.deferred,
            done: d.done,
            on_main: d.on_main,
            focus: d.focus,
            focus_goal: d.focus_goal.map(|g| FocusGoalRow {
                id: g.id,
                key: g.key,
                title: g.title,
                position: g.position,
            }),
            flags: d.flags,
            description: t.description,
            design: t.design,
            completion_criterion: t.completion_criterion,
            closing_comment: t.closing_comment,
            replaced_by: t.replaced_by,
            replaces: t.replaces,
            deps: edges(t.deps),
            dependents: edges(t.dependents),
            children: t.children,
            contributes_to: t.contributes_to,
            mentions: t.mentions,
            mentioned_by: t.mentioned_by,
            notes: t
                .notes
                .into_iter()
                .map(|n| NoteRow {
                    id: n.id,
                    body: n.body,
                    reply_to: n.reply_to,
                    created_at: n.created_at,
                    created_by: n.created_by,
                })
                .collect(),
            work: t
                .work
                .into_iter()
                .map(|w| WorkRow {
                    kind: w.kind,
                    url: w.url,
                    branch: w.branch,
                    state: w.state,
                    commit: w.commit,
                    on_main: w.on_main,
                    created_at: w.created_at,
                })
                .collect(),
            references: t
                .references
                .into_iter()
                .map(|r| ReferenceRow {
                    kind: r.kind,
                    space: r.space,
                    id: r.id,
                    label: r.label,
                    state: r.state,
                })
                .collect(),
        }
    }
}

impl Render for Record {
    /// `# <key> · <type> · <title>`, a list of the fields that are set, and the description,
    /// design and completion criterion under headings of their own when they are not empty.
    fn markdown(&self) -> String {
        let mut text = format!(
            "# {} · {} · {}\n\n- Status: {}\n",
            self.key, self.kind, self.title, self.status
        );
        let mut line = |name: &str, value: Option<String>| {
            if let Some(value) = value {
                text.push_str(&format!("- {name}: {value}\n"));
            }
        };
        let list = |items: &[String]| (!items.is_empty()).then(|| items.join(", "));
        line("Resolution", self.resolution.clone());
        line("Priority", self.priority.map(|p| format!("P{p}")));
        line(
            "Assignee",
            self.assignee.as_ref().map(|h| match &h.role {
                Some(role) => format!("{} {} ({role})", h.kind, h.id),
                None => format!("{} {}", h.kind, h.id),
            }),
        );
        line("Parent", self.parent.clone());
        let deps: Vec<String> = self
            .deps
            .iter()
            .map(|e| format!("{} ({})", e.key, e.status))
            .collect();
        line("Depends on", list(&deps));
        line("Children", list(&self.children));
        line("Labels", list(&self.labels));
        line("Due", self.due.clone());
        line("Defer until", self.defer_until.clone());
        line("Closed", self.closed_at.clone());
        line("Archived", self.archived_at.clone());
        line("Created", self.created_at.clone());
        line("Updated", self.updated_at.clone());
        line("Id", Some(self.id.clone()));
        for (heading, body) in [
            ("Description", &self.description),
            ("Design", &self.design),
            ("Completion criterion", &self.completion_criterion),
        ] {
            if !body.trim().is_empty() {
                text.push_str(&format!("\n## {heading}\n\n{}\n", body.trim_end()));
            }
        }
        text
    }
}
