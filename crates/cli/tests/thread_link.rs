//! `nxf thread` — a chat thread as an endpoint of a link to a board item (nxf 6j6v.8dbe).
//!
//! The black-box half of the ticket's acceptance criteria: a thread can be linked with a relation
//! and a weight and unlinked again, `show` reports that there are conversations about an item,
//! `next` shows the hint WITHOUT a grazing conversation flooding the work list, and a thread can
//! enumerate its own items. The convergence and OR-set properties live one layer down, in
//! `crates/core/src/store.rs`'s unit tests.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxf(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxf");
    c.current_dir(dir);
    c
}

fn run_json(dir: &Path, args: &[&str]) -> serde_json::Value {
    let out = nxf(dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json output")
}

fn run_text(dir: &Path, args: &[&str]) -> String {
    let out = nxf(dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("utf-8 output")
}

fn init(dir: &Path) {
    nxf(dir)
        .args(["init", "--plugin", "issue-tracker"])
        .assert()
        .success();
}

fn create(dir: &Path, title: &str) -> String {
    run_json(
        dir,
        &[
            "create",
            "--type",
            "bug",
            "--title",
            title,
            "--description",
            "d",
            "--priority",
            "P1",
            "--json",
        ],
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_thread_links_to_an_item_with_both_attributes_and_unlinks_again() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let item = create(tmp.path(), "the ticket the conversation was about");

    let linked = run_json(
        tmp.path(),
        &[
            "thread",
            "link",
            "m-thread-1",
            &item,
            "--relation",
            "worked_on",
            "--weight",
            "bearing",
            "--json",
        ],
    );
    assert_eq!(linked["thread_id"], "m-thread-1");
    assert_eq!(linked["item_id"], item.as_str());
    assert_eq!(linked["relation"], "worked_on");
    assert_eq!(linked["weight"], "bearing");

    // `show` says there ARE conversations — the whole point: a later reader sees it without looking.
    let shown = run_text(tmp.path(), &["show", &item]);
    assert!(
        shown.contains("CONVERSATIONS: m-thread-1 (worked_on, bearing)"),
        "show must surface the conversation:\n{shown}"
    );

    // And it can be taken back — a link is movable, unlike a memory.
    run_json(
        tmp.path(),
        &["thread", "unlink", "m-thread-1", &item, "--json"],
    );
    let shown = run_text(tmp.path(), &["show", &item]);
    assert!(
        !shown.contains("CONVERSATIONS"),
        "an unlinked thread leaves no trace on the item:\n{shown}"
    );
}

#[test]
fn a_thread_enumerates_its_own_items() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let a = create(tmp.path(), "the one being worked on");
    let b = create(tmp.path(), "the one merely cited");

    run_json(tmp.path(), &["thread", "link", "m-1", &a, "--json"]);
    run_json(
        tmp.path(),
        &[
            "thread",
            "link",
            "m-1",
            &b,
            "--relation",
            "cited",
            "--weight",
            "passing",
            "--json",
        ],
    );

    let listed = run_json(tmp.path(), &["thread", "list", "m-1", "--json"]);
    let rows: Vec<(String, String, String)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["item_id"].as_str().unwrap().to_string(),
                r["relation"].as_str().unwrap().to_string(),
                r["weight"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let mut expected = vec![
        (a, "worked_on".to_string(), "bearing".to_string()),
        (b, "cited".to_string(), "passing".to_string()),
    ];
    expected.sort();
    assert_eq!(rows, expected, "both directions of the same n:m set");
}

#[test]
fn next_advertises_a_bearing_conversation_and_stays_quiet_about_a_grazing_one() {
    // The weight's whole reason for existing: without it, a long wandering conversation would make
    // every item it touched read "there are conversations about this" — noise exactly where a coding
    // agent looks. A `passing` link stays visible on the item itself.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let subject = create(tmp.path(), "what the thread is about");
    let grazed = create(tmp.path(), "what the thread merely touched");

    run_json(
        tmp.path(),
        &[
            "thread", "link", "m-1", &subject, "--weight", "bearing", "--json",
        ],
    );
    run_json(
        tmp.path(),
        &[
            "thread", "link", "m-1", &grazed, "--weight", "passing", "--json",
        ],
    );

    let next = run_json(tmp.path(), &["next", "--json"]);
    let rows = next.as_array().unwrap();
    let conversations = |id: &str| -> Option<u64> {
        rows.iter()
            .find(|r| r["id"].as_str() == Some(id))
            .and_then(|r| r["conversations"].as_u64())
    };
    assert_eq!(
        conversations(&subject),
        Some(1),
        "a bearing link is advertised in next: {next}"
    );
    assert_eq!(
        conversations(&grazed),
        None,
        "a grazing link must NOT surface in next: {next}"
    );

    // But the grazing one IS on the item itself, where the reader asked about it by name.
    let shown = run_text(tmp.path(), &["show", &grazed]);
    assert!(
        shown.contains("CONVERSATIONS: m-1 (worked_on, passing)"),
        "show carries both weights:\n{shown}"
    );

    // The PLAIN-TEXT lane carries the same gate as `--json` (PR #273 review, Test Quality #4): the
    // two renderers read the same bulk count, and only the `--json` half was pinned.
    let next_text = run_text(tmp.path(), &["next"]);
    let line_for = |id: &str| -> String {
        let short = id.split('.').next_back().unwrap_or(id);
        next_text
            .lines()
            .find(|l| l.contains(short))
            .unwrap_or_else(|| panic!("no `next` row for {id}:\n{next_text}"))
            .to_string()
    };
    assert!(
        line_for(&subject).contains("(1 conversation)"),
        "the bearing row is decorated: {}",
        line_for(&subject)
    );
    assert!(
        !line_for(&grazed).contains("conversation"),
        "the grazing row is not: {}",
        line_for(&grazed)
    );
}

#[test]
fn the_next_decoration_pluralizes_with_the_count() {
    // The decoration is a word, not a pictogram, so it has to read correctly at both counts.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let item = create(tmp.path(), "two conversations about it");
    run_json(tmp.path(), &["thread", "link", "m-1", &item, "--json"]);
    run_json(tmp.path(), &["thread", "link", "m-2", &item, "--json"]);

    let next_text = run_text(tmp.path(), &["next"]);
    assert!(
        next_text.contains("(2 conversations)"),
        "two links, plural:\n{next_text}"
    );
}

#[test]
fn a_link_firms_up_in_place_rather_than_doubling() {
    // Re-linking the same pair UPDATES the attributes; it does not add a second link. This is how a
    // conversation that turned out to be about something graduates from `passing` to `bearing`.
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let item = create(tmp.path(), "turned out to be the subject");

    run_json(
        tmp.path(),
        &[
            "thread",
            "link",
            "m-1",
            &item,
            "--relation",
            "cited",
            "--weight",
            "passing",
            "--json",
        ],
    );
    run_json(
        tmp.path(),
        &[
            "thread",
            "link",
            "m-1",
            &item,
            "--relation",
            "cited",
            "--weight",
            "bearing",
            "--json",
        ],
    );

    let shown = run_text(tmp.path(), &["show", &item]);
    assert!(
        shown.contains("CONVERSATIONS: m-1 (cited, bearing)"),
        "the later link wins, in place:\n{shown}"
    );
    assert_eq!(
        shown.matches("m-1").count(),
        1,
        "and there is exactly ONE link, not two:\n{shown}"
    );
}

#[test]
fn an_unknown_attribute_or_a_missing_item_is_a_loud_error() {
    let tmp = TempDir::new().unwrap();
    init(tmp.path());
    let item = create(tmp.path(), "real");

    nxf(tmp.path())
        .args(["thread", "link", "m-1", &item, "--relation", "pondered"])
        .assert()
        .failure();
    nxf(tmp.path())
        .args(["thread", "link", "m-1", &item, "--weight", "heavy"])
        .assert()
        .failure();
    // The ITEM endpoint is existence-checked; the thread endpoint deliberately is not — flow does
    // not own the chat id space, so an arbitrary thread id is a perfectly ordinary address.
    nxf(tmp.path())
        .args(["thread", "link", "m-1", "c1.nope"])
        .assert()
        .failure();
    nxf(tmp.path())
        .args(["thread", "link", "any-thread-at-all", &item])
        .assert()
        .success();
}
