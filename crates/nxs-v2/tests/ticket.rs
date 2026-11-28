//! `nxs ticket create` and `nxs ticket show` (RFC 0002, "Commands, JSON and the seam"): create
//! takes every flag of the RFC, or the whole ticket as JSON on stdin with `-`, and prints the
//! ticket as show prints it, or its key alone with `-q`. Both print what their `Engine`
//! counterparts return, as one record with every key, `[]` and `null` for empty values, in JSON
//! by default and in Markdown with `--md`. A refusal is v1's error object, with RFC 0002's codes
//! `key_not_found`, `ambiguous_key`, `not_synced` and `no_tag`, and writes nothing.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{block_on, json, nxs, options, scratch, stdout};
use nexus_flow_seam::{Engine, Ticket};
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

    /// Runs `nxs <args>` with `input` on stdin.
    fn nxs_stdin(&self, args: &[&str], input: &str) -> nxs_v2::Outcome {
        let data = self.base.join("data");
        let argv = std::iter::once("nxs").chain(args.iter().copied());
        nxs_v2::run_with_stdin(
            argv,
            &self.base.join("repo"),
            || Ok(options(&data)),
            &mut input.as_bytes(),
        )
    }

    fn engine(&self) -> Engine {
        let setup = options(&self.base.join("data"));
        block_on(Engine::open_with(&self.base.join("repo"), setup)).unwrap()
    }

    /// Creates a ticket of `kind` titled `title` with `args` besides, and returns its JSON.
    fn create(&self, kind: &str, title: &str, args: &[&str]) -> Value {
        let mut all = vec![
            "ticket",
            "create",
            "--type",
            kind,
            "--title",
            title,
            "--description",
            "Why it exists",
        ];
        all.extend_from_slice(args);
        let out = self.nxs(&all);
        assert_eq!(out.code, 0, "{}", stdout(&out));
        json(&out)
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn kind(out: &nxs_v2::Outcome) -> String {
    json(out)["error"]["kind"].as_str().unwrap().to_owned()
}

/// The ticket as RFC 0002, "Commands, JSON and the seam", prints it in `show`.
fn record(t: &Ticket) -> Value {
    let edges = |list: &[nexus_flow_seam::Edge]| {
        list.iter()
            .map(|e| json!({ "key": e.key, "status": e.status, "done": e.done, "on_main": e.on_main }))
            .collect::<Vec<_>>()
    };
    assert!(t.notes.is_empty() && t.work.is_empty() && t.references.is_empty());
    assert!(t.request.is_none() && t.derived.focus_goal.is_none());
    json!({
        "id": t.id,
        "key": t.key,
        "type": t.kind,
        "title": t.title,
        "status": t.status,
        "resolution": t.resolution,
        "priority": t.priority,
        "assignee": t.assignee.as_ref().map(|h| json!({ "kind": h.kind, "id": h.id, "role": h.role })),
        "parent": t.parent,
        "labels": t.labels,
        "due": t.due,
        "defer_until": t.defer_until,
        "created_at": t.created_at,
        "updated_at": t.updated_at,
        "closed_at": t.closed_at,
        "archived_at": t.archived_at,
        "request": null,
        "ready": t.derived.ready,
        "blocked": t.derived.blocked,
        "deferred": t.derived.deferred,
        "done": t.derived.done,
        "on_main": t.derived.on_main,
        "focus": t.derived.focus,
        "focus_goal": null,
        "flags": t.derived.flags,
        "description": t.description,
        "design": t.design,
        "completion_criterion": t.completion_criterion,
        "closing_comment": t.closing_comment,
        "replaced_by": t.replaced_by,
        "replaces": t.replaces,
        "deps": edges(&t.deps),
        "dependents": edges(&t.dependents),
        "children": t.children,
        "contributes_to": t.contributes_to,
        "mentions": t.mentions,
        "mentioned_by": t.mentioned_by,
        "notes": [],
        "work": [],
        "references": [],
    })
}

#[test]
fn create_takes_every_flag_and_prints_the_ticket_as_the_engine_returns_it() {
    let repo = Repo::new("ticket-create-flags");
    let epic = repo.create("epic", "The epic", &[]);
    let dep = repo.create("question", "A question", &[]);
    let member = block_on(repo.engine().members()).unwrap().remove(0).member;
    let assignee = format!("member:{member}");
    let file = repo.base.join("description.md");
    fs::write(&file, "From a file\n").unwrap();

    let out = repo.nxs(&[
        "ticket",
        "create",
        "--type",
        "feature",
        "--title",
        "Every flag",
        "--description-file",
        file.to_str().unwrap(),
        "--priority",
        "P1",
        "--design",
        "The path",
        "--dod",
        "It holds",
        "--due",
        "2026-12-01",
        "--defer",
        "2026-11-30T09:00:00Z",
        "--parent",
        epic["key"].as_str().unwrap(),
        "--depends-on",
        dep["key"].as_str().unwrap(),
        "--label",
        "rfc",
        "--label",
        "cli",
        "--assignee",
        &assignee,
    ]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let printed = json(&out);
    let engine = repo.engine();
    let ticket = block_on(engine.ticket(printed["id"].as_str().unwrap())).unwrap();
    assert_eq!(printed, record(&ticket));
    assert_eq!(printed["description"], "From a file\n");
    assert_eq!(printed["priority"], 1);
    assert_eq!(printed["parent"], epic["key"]);
    assert_eq!(printed["deps"][0]["key"], dep["key"]);
    assert_eq!(printed["labels"], json!(["cli", "rfc"]));
    assert_eq!(
        printed["assignee"],
        json!({ "kind": "member", "id": member, "role": "owner" })
    );
}

#[test]
fn show_prints_the_ticket_as_the_engine_returns_it_by_key_or_object_id() {
    let repo = Repo::new("ticket-show");
    let epic = repo.create("epic", "The epic", &[]);
    let child = repo.create(
        "bug",
        "A child",
        &["--parent", epic["key"].as_str().unwrap()],
    );

    let by_key = repo.nxs(&["ticket", "show", epic["key"].as_str().unwrap()]);
    let by_id = repo.nxs(&["ticket", "show", epic["id"].as_str().unwrap()]);

    assert_eq!(by_key.code, 0, "{}", stdout(&by_key));
    let ticket = block_on(repo.engine().ticket(epic["key"].as_str().unwrap())).unwrap();
    assert_eq!(json(&by_key), record(&ticket));
    assert_eq!(json(&by_key)["children"], json!([child["key"]]));
    assert_eq!(by_id.stdout, by_key.stdout);
}

#[test]
fn a_priority_is_its_ordinal_from_the_label_or_the_number_and_unset_is_null() {
    let repo = Repo::new("ticket-priority");

    let label = repo.create("feature", "By label", &["--priority", "P2"]);
    let number = repo.create("feature", "By number", &["--priority", "2"]);
    let unset = repo.create("feature", "Unset", &[]);

    assert_eq!(label["priority"], 2);
    assert_eq!(number["priority"], 2);
    assert_eq!(unset["priority"], Value::Null);
}

/// The keys of `show`'s record (RFC 0002, "Commands, JSON and the seam"); the golden file holds
/// the order they print in, which a parsed JSON object does not keep.
const KEYS: [&str; 40] = [
    "id",
    "key",
    "type",
    "title",
    "status",
    "resolution",
    "priority",
    "assignee",
    "parent",
    "labels",
    "due",
    "defer_until",
    "created_at",
    "updated_at",
    "closed_at",
    "archived_at",
    "request",
    "ready",
    "blocked",
    "deferred",
    "done",
    "on_main",
    "focus",
    "focus_goal",
    "flags",
    "description",
    "design",
    "completion_criterion",
    "closing_comment",
    "replaced_by",
    "replaces",
    "deps",
    "dependents",
    "children",
    "contributes_to",
    "mentions",
    "mentioned_by",
    "notes",
    "work",
    "references",
];

#[test]
fn every_record_has_every_key_with_empty_values_as_empty_arrays_and_null() {
    let repo = Repo::new("ticket-keys");
    let created = repo.create("chore", "Bare", &[]);
    let shown = json(&repo.nxs(&["ticket", "show", created["key"].as_str().unwrap()]));

    for record in [&created, &shown] {
        let mut keys: Vec<&str> = record
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut expected = KEYS.to_vec();
        keys.sort_unstable();
        expected.sort_unstable();
        assert_eq!(keys, expected);
        for key in [
            "resolution",
            "priority",
            "assignee",
            "parent",
            "due",
            "defer_until",
            "closed_at",
            "archived_at",
            "request",
            "ready",
            "blocked",
            "deferred",
            "done",
            "on_main",
            "focus",
            "focus_goal",
            "closing_comment",
            "replaced_by",
        ] {
            assert_eq!(record[key], Value::Null, "{key}");
        }
        for key in [
            "labels",
            "flags",
            "replaces",
            "deps",
            "dependents",
            "children",
            "contributes_to",
            "mentions",
            "mentioned_by",
            "notes",
            "work",
            "references",
        ] {
            assert_eq!(record[key], json!([]), "{key}");
        }
        assert_eq!(record["design"], "");
        assert_eq!(record["completion_criterion"], "");
    }
}

#[test]
fn quiet_prints_the_key_alone() {
    let repo = Repo::new("ticket-quiet");

    let out = repo.nxs(&[
        "ticket",
        "create",
        "--type",
        "feature",
        "--title",
        "Quiet",
        "--description",
        "Why",
        "-q",
    ]);

    assert_eq!(out.code, 0, "{}", stdout(&out));
    let key = stdout(&out);
    let shown = json(&repo.nxs(&["ticket", "show", key.trim_end()]));
    assert_eq!(key, format!("{}\n", shown["key"].as_str().unwrap()));
}

#[test]
fn a_description_of_dash_and_a_dash_payload_read_stdin() {
    let repo = Repo::new("ticket-stdin");

    let described = repo.nxs_stdin(
        &[
            "ticket",
            "create",
            "--type",
            "bug",
            "--title",
            "Piped",
            "--description",
            "-",
        ],
        "Two\nlines\n",
    );
    let payload = repo.nxs_stdin(
        &["ticket", "create", "-"],
        r#"{"type": "chore", "title": "Whole", "description": "From JSON", "priority": "P3",
            "dod": "Done", "labels": ["json"], "depends_on": []}"#,
    );
    let unknown = repo.nxs_stdin(
        &["ticket", "create", "-"],
        r#"{"type": "chore", "title": "Whole", "description": "From JSON", "colour": "red"}"#,
    );
    let mixed = repo.nxs_stdin(&["ticket", "create", "--title", "Both", "-"], "{}");

    assert_eq!(described.code, 0, "{}", stdout(&described));
    assert_eq!(json(&described)["description"], "Two\nlines\n");
    assert_eq!(payload.code, 0, "{}", stdout(&payload));
    let whole = json(&payload);
    assert_eq!(whole["title"], "Whole");
    assert_eq!(whole["description"], "From JSON");
    assert_eq!(whole["priority"], 3);
    assert_eq!(whole["completion_criterion"], "Done");
    assert_eq!(whole["labels"], json!(["json"]));
    assert_eq!(kind(&unknown), "validation");
    assert_eq!(kind(&mixed), "validation");
}

#[test]
fn a_refusal_is_an_error_object_with_its_code_and_writes_nothing() {
    let repo = Repo::new("ticket-refused");
    let feature = repo.create("feature", "A feature", &[]);
    let key = feature["key"].as_str().unwrap().to_owned();
    let tag = key.split('.').next().unwrap().to_owned();

    let under_feature = repo.nxs(&[
        "ticket",
        "create",
        "--type",
        "bug",
        "--title",
        "Under a feature",
        "--description",
        "Why",
        "--parent",
        &key,
    ]);
    let no_type = repo.nxs(&[
        "ticket",
        "create",
        "--title",
        "No type",
        "--description",
        "Why",
    ]);
    let unknown = repo.nxs(&["ticket", "show", &format!("{tag}.9")]);
    let unknown_dep = repo.nxs(&[
        "ticket",
        "create",
        "--type",
        "bug",
        "--title",
        "Blocked",
        "--description",
        "Why",
        "--depends-on",
        &format!("{tag}.9"),
    ]);

    assert_eq!(kind(&under_feature), "validation");
    let msg = json(&under_feature)["error"]["msg"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(msg.contains(&key) && msg.contains("epic"), "{msg}");
    assert_eq!(kind(&no_type), "validation");
    assert_eq!(kind(&unknown), "key_not_found");
    assert_eq!(kind(&unknown_dep), "key_not_found");
    let next = repo.create("chore", "The next", &[]);
    assert_eq!(next["key"], format!("{tag}.2"), "nothing was written");
}

#[test]
fn a_key_two_tickets_hold_is_ambiguous_key() {
    let repo = Repo::new("ticket-ambiguous");
    let first = repo.create("chore", "The first", &[]);
    let key = first["key"].as_str().unwrap();
    let setup = options(&repo.base.join("data"));
    let twin = block_on(nexus_flow_seam::testing::ticket_with_key(
        &repo.base.join("repo"),
        &setup.data,
        setup.keys,
        key,
    ));

    let out = repo.nxs(&["ticket", "show", key]);
    let by_suffix = repo.nxs(&["ticket", "show", &twin]);

    assert_eq!(kind(&out), "ambiguous_key");
    assert_eq!(by_suffix.code, 0, "{}", stdout(&by_suffix));
    assert_eq!(json(&by_suffix)["key"], twin.as_str());
}

/// The golden file `name`, with what each test draws anew filled in: the object id, the key,
/// the time of the create and the member that created it.
fn golden(name: &str, ticket: &Value) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .replace("{id}", ticket["id"].as_str().unwrap())
        .replace("{key}", ticket["key"].as_str().unwrap())
        .replace("{created_at}", ticket["created_at"].as_str().unwrap())
}

#[test]
fn the_same_local_copy_prints_the_same_bytes_as_the_golden_files() {
    let repo = Repo::new("ticket-golden");
    let ticket = repo.create(
        "feature",
        "Show a ticket",
        &[
            "--priority",
            "P2",
            "--design",
            "The path to it",
            "--dod",
            "It prints the same bytes",
            "--due",
            "2026-12-01",
            "--label",
            "cli",
        ],
    );
    let key = ticket["key"].as_str().unwrap();

    let json_first = repo.nxs(&["ticket", "show", key]);
    let json_second = repo.nxs(&["ticket", "show", key]);
    let md_first = repo.nxs(&["ticket", "show", key, "--md"]);
    let md_second = repo.nxs(&["--md", "ticket", "show", key]);

    assert_eq!(stdout(&json_first), golden("ticket-show.json", &ticket));
    assert_eq!(json_second.stdout, json_first.stdout);
    assert_eq!(stdout(&md_first), golden("ticket-show.md", &ticket));
    assert_eq!(md_second.stdout, md_first.stdout);
}

/// The golden error file `name`, with the repository space's id filled in.
fn golden_error(repo: &Repo, name: &str) -> String {
    let space = json(&repo.nxs(&["init"]))["space"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .replace("{space}", &space)
}

const CREATE: &[&str] = &[
    "ticket",
    "create",
    "--type",
    "feature",
    "--title",
    "Minted",
    "--description",
    "Why",
];

#[test]
fn a_freed_tag_before_a_sync_is_not_synced() {
    let repo = Repo::new("ticket-not-synced");
    let setup = options(&repo.base.join("data"));
    block_on(nexus_flow_seam::testing::take_over_freed_tag(
        &repo.base.join("repo"),
        &setup.data,
        setup.keys,
    ));

    let out = repo.nxs(CREATE);

    assert_eq!(kind(&out), "not_synced");
    assert_eq!(
        stdout(&out),
        golden_error(&repo, "ticket-create-not-synced.json")
    );
}

#[test]
fn a_member_without_a_tag_is_no_tag() {
    let repo = Repo::new("ticket-no-tag");
    let setup = options(&repo.base.join("data"));
    block_on(nexus_flow_seam::testing::clear_device_tag(
        &repo.base.join("repo"),
        &setup.data,
        setup.keys,
    ));

    let out = repo.nxs(CREATE);

    assert_eq!(kind(&out), "no_tag");
    assert_eq!(
        stdout(&out),
        golden_error(&repo, "ticket-create-no-tag.json")
    );
}
