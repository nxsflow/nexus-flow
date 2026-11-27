//! `nxs decision add` (RFC 0003, "Writing a decision", "Public surfaces this RFC brings"): it
//! prints the decision as its `Engine::decision_add` counterpart returns it, warns on stderr when
//! the handle is taken already in the local copy and writes all the same, and refuses with
//! `validation` or `not_found` and a message that names the problem, writing nothing.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{block_on, json, nxs, options, scratch, stdout};
use nexus_flow_seam::{Decision, Engine, NewDecision};
use serde_json::{json, Value};

struct Repo {
    base: PathBuf,
}

impl Repo {
    fn new(test: &str) -> Repo {
        let base = scratch(test);
        let out = nxs(&base, &base.join("repo"), &["init"]);
        assert_eq!(out.code, 0, "{}", stdout(&out));
        Repo { base }
    }

    fn nxs(&self, args: &[&str]) -> nxs_v2::Outcome {
        nxs(&self.base, &self.base.join("repo"), args)
    }

    fn engine(&self) -> Engine {
        let setup = options(&self.base.join("data"));
        block_on(Engine::open_with(&self.base.join("repo"), setup)).unwrap()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// A decision in full, as RFC 0003, "Reading decisions", prints it.
fn full(d: &Decision) -> Value {
    let linked = |l: &[nexus_flow_seam::Linked]| {
        l.iter()
            .map(|l| json!({ "id": l.id, "handle": l.handle }))
            .collect::<Vec<_>>()
    };
    json!({
        "id": d.id,
        "handle": d.handle,
        "date": d.date,
        "tickets": d.tickets,
        "subjects": d.subjects.iter().map(|s| json!({
            "label": s.label,
            "kind": s.kind.as_str(),
            "space": s.space,
            "object": s.object,
            "status": s.status,
        })).collect::<Vec<_>>(),
        "title": d.title,
        "decided_by": d.decided_by,
        "why": d.why,
        "instead_of": d.instead_of,
        "topics": d.topics,
        "body": d.body,
        "supersedes": linked(&d.supersedes),
        "superseded_by": linked(&d.superseded_by),
        "in_force": d.in_force,
        "pr": d.pr,
        "on_main": d.on_main,
        "author": d.author,
        "edited": d.edited,
        "deleted": d.deleted,
    })
}

const ADD: &[&str] = &[
    "decision",
    "add",
    "--title",
    "A decision in one change",
    "--why",
    "Because it holds",
    "--instead-of",
    "The option not taken",
    "--topic",
    "decisions",
    "--topic",
    "nxs",
    "--ticket",
    "NEX-6 to NEX-13",
    "--decided-by",
    "Carsten",
    "--date",
    "2026-11-24",
];

#[test]
fn add_prints_the_decision_as_the_engine_returns_it() {
    let repo = Repo::new("decision-add");
    let body = repo.base.join("body.md");
    fs::write(&body, "## The rule\n\nAs written.\n").unwrap();
    let mut args = ADD.to_vec();
    args.extend(["--pr", "https://github.com/nxsflow/nexus-flow/pull/9"]);
    args.extend(["--body-file", body.to_str().unwrap()]);

    let out = repo.nxs(&args);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    assert!(out.stderr.is_empty(), "no warning for a new handle");
    let printed = json(&out);
    let id = printed["id"].as_str().unwrap().to_owned();
    let engine = repo.engine();
    let mut decision = NewDecision {
        title: "A decision in one change".to_owned(),
        why: "Because it holds".to_owned(),
        instead_of: "The option not taken".to_owned(),
        topics: vec!["decisions".to_owned(), "nxs".to_owned()],
        tickets: vec!["NEX-6 to NEX-13".to_owned()],
        decided_by: Some("Carsten".to_owned()),
        date: Some("2026-11-24".to_owned()),
        pr: Some("https://github.com/nxsflow/nexus-flow/pull/9".to_owned()),
        body: Some("## The rule\n\nAs written.\n".to_owned()),
        slug: Some("parity".to_owned()),
        ..NewDecision::default()
    };
    decision.supersedes = vec![];
    let twin = block_on(engine.decision_add(decision)).unwrap();
    let mut expected = json!({
        "id": id,
        "handle": "2026-11-24-NEX-6-a-decision-in-one-change",
        "date": "2026-11-24",
        "tickets": "NEX-6 to NEX-13",
        "subjects": (6..=13).map(|n| json!({
            "label": format!("NEX-{n}"),
            "kind": "ticket",
            "space": null,
            "object": null,
            "status": null,
        })).collect::<Vec<_>>(),
        "title": "A decision in one change",
        "decided_by": "Carsten",
        "why": "Because it holds",
        "instead_of": "The option not taken",
        "topics": ["decisions", "nxs"],
        "body": "## The rule\n\nAs written.\n",
        "supersedes": [],
        "superseded_by": [],
        "in_force": true,
        "pr": "https://github.com/nxsflow/nexus-flow/pull/9",
        "on_main": null,
        "author": twin.author,
        "edited": false,
        "deleted": false,
    });
    assert_eq!(printed, expected, "the decision as RFC 0003 prints it");
    expected["id"] = json!(twin.id);
    expected["handle"] = json!("2026-11-24-NEX-6-parity");
    assert_eq!(
        full(&twin),
        expected,
        "the engine returns the same decision"
    );
}

#[test]
fn a_taken_handle_warns_on_stderr_and_is_written_all_the_same() {
    let repo = Repo::new("decision-taken");
    let first = repo.nxs(ADD);
    assert_eq!(first.code, 0, "{}", stdout(&first));

    let second = repo.nxs(ADD);

    assert_eq!(second.code, 0, "{}", stdout(&second));
    let stderr = String::from_utf8(second.stderr.clone()).unwrap();
    let first_id = json(&first)["id"].as_str().unwrap().to_owned();
    assert!(
        stderr.starts_with("warning: ")
            && stderr.contains("2026-11-24-NEX-6-a-decision-in-one-change")
            && stderr.contains(&first_id),
        "{stderr}"
    );
    let handle = json(&second)["handle"].as_str().unwrap().to_owned();
    assert_eq!(
        block_on(repo.engine().decisions_with_handle(&handle))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn a_refusal_names_its_problem_and_writes_nothing() {
    let repo = Repo::new("decision-refused");
    let mut args = ADD.to_vec();
    args.extend(["--pr", "http://example.com/pull/1"]);

    let out = repo.nxs(&args);

    assert_eq!(out.code, 1);
    let error = &json(&out)["error"];
    assert_eq!(error["kind"], "validation");
    assert!(
        error["msg"]
            .as_str()
            .unwrap()
            .contains("http://example.com/pull/1"),
        "{error}"
    );
    let handle = "2026-11-24-NEX-6-a-decision-in-one-change";
    assert!(block_on(repo.engine().decisions_with_handle(handle))
        .unwrap()
        .is_empty());

    let mut args = ADD.to_vec();
    args.extend(["--supersedes", "2026-11-01-no-ticket-nothing"]);
    let out = repo.nxs(&args);
    assert_eq!(json(&out)["error"]["kind"], "not_found");
}

#[test]
fn author_cannot_be_given() {
    let repo = Repo::new("decision-author");
    let mut args = ADD.to_vec();
    args.extend(["--author", "someone"]);

    let out = repo.nxs(&args);

    assert_eq!(out.code, 1);
    assert_eq!(json(&out)["error"]["kind"], "validation");
}

#[test]
fn body_and_body_file_exclude_each_other() {
    let repo = Repo::new("decision-body");
    let mut args = ADD.to_vec();
    args.extend(["--body", "text", "--body-file", "body.md"]);

    let out = repo.nxs(&args);

    assert_eq!(json(&out)["error"]["kind"], "validation");
}

#[test]
fn add_in_markdown_prints_the_decision_as_a_file_reads() {
    let repo = Repo::new("decision-md");
    let mut args = ADD.to_vec();
    args.push("--md");

    let out = repo.nxs(&args);

    assert_eq!(out.code, 0);
    assert_eq!(
        stdout(&out),
        "# 2026-11-24 · NEX-6 to NEX-13 · A decision in one change\n\
         \n\
         - Decided by: Carsten\n\
         - Why: Because it holds\n\
         - Instead of: The option not taken\n\
         - Topics: decisions, nxs\n"
    );
}
