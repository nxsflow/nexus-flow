//! sp6.4 — the declarative type system, proven against the TWO bundled plugins through the real
//! write seam (no test matrix). The same `create` + `set parent` commands behave differently purely
//! because each plugin's `[types]` declares a different shape: issue-tracker is single-parent
//! (cardinality 1 → re-point), personal-todo is multi-parent (cardinality "many" → accumulate).
//! Both are 2 levels and reject anything deeper or off the allowed parent→child pairs.

use nexus_flow_core::store::Store;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::plugin::{self, PluginConfig};
use nexus_flow_facade::write::{self, NewItem};

const NOW: &str = "2026-06-17T08:30:00Z";
const ACTOR: &str = "alice";

fn store() -> Store {
    Store::open_in_memory(1)
}

/// Create an item of `ty` with `priority` (a label the active plugin accepts) and return its id.
fn create(s: &mut Store, cfg: &PluginConfig, ty: &str, title: &str, priority: &str) -> String {
    write::create(
        s,
        cfg,
        "ab12",
        NOW,
        ACTOR,
        ty,
        title,
        NewItem {
            description: "d",
            priority,
            design: None,
            dod: None,
            due: None,
            defer: None,
            parent: None,
            depends_on: &[],
            custom: &[],
        },
    )
    .unwrap()
    .id
}

/// `set parent` via the same `update` seam the CLI uses; return the result (Ok/Err) for assertions.
fn set_parent(
    s: &mut Store,
    cfg: &PluginConfig,
    child: &str,
    parent: &str,
) -> nexus_flow_facade::error::Result<()> {
    write::update(s, cfg, NOW, ACTOR, child, &[format!("parent={parent}")]).map(|_| ())
}

#[test]
fn parent_cardinality_is_plugin_declared_single_vs_many() {
    // The headline seam proof (sp6.4): the SAME two `set parent` writes leave a DIFFERENT parent
    // count purely from the active plugin's `[types]` — no command code branches on the plugin.

    // issue-tracker — cardinality Single: the second `set parent` RE-POINTS to the latest epic.
    let cfg = plugin::load("issue-tracker").unwrap();
    let mut s = store();
    let e1 = create(&mut s, &cfg, "epic", "E1", "P1");
    let e2 = create(&mut s, &cfg, "epic", "E2", "P1");
    let bug = create(&mut s, &cfg, "bug", "B", "P1");
    set_parent(&mut s, &cfg, &bug, &e1).unwrap();
    set_parent(&mut s, &cfg, &bug, &e2).unwrap();
    assert_eq!(
        s.parents_of(&bug),
        vec![e2.clone()],
        "single-parent re-points: exactly one parent, the latest"
    );

    // personal-todo — cardinality Many: the second `set parent` ADDS, so the todo keeps BOTH.
    let cfg = plugin::load("personal-todo").unwrap();
    let mut s = store();
    let p1 = create(&mut s, &cfg, "project", "P1", "now");
    let p2 = create(&mut s, &cfg, "project", "P2", "now");
    let todo = create(&mut s, &cfg, "todo", "T", "now");
    set_parent(&mut s, &cfg, &todo, &p1).unwrap();
    set_parent(&mut s, &cfg, &todo, &p2).unwrap();
    let mut parents = s.parents_of(&todo);
    parents.sort();
    let mut want = vec![p1, p2];
    want.sort();
    assert_eq!(
        parents, want,
        "many-cardinality accumulates: the todo belongs to both projects"
    );
}

#[test]
fn many_cardinality_has_no_upper_bound() {
    // personal-todo's "beliebig viele": a todo can belong to an arbitrary number of projects — no
    // count ever trips a cardinality reject (the seam genuinely encodes n, not a fixed cap).
    let cfg = plugin::load("personal-todo").unwrap();
    let mut s = store();
    let todo = create(&mut s, &cfg, "todo", "T", "now");
    let mut projects = Vec::new();
    for i in 0..7 {
        let p = create(&mut s, &cfg, "project", &format!("P{i}"), "now");
        set_parent(&mut s, &cfg, &todo, &p).unwrap();
        projects.push(p);
    }
    let mut parents = s.parents_of(&todo);
    parents.sort();
    projects.sort();
    assert_eq!(parents, projects, "all 7 project parents stick");
}

#[test]
fn each_plugin_rejects_a_disallowed_parent_pair() {
    // A parent edge off the declared parent→child pairs is a loud `validation` reject in BOTH
    // plugins — only the declared container type may be a parent.

    // issue-tracker: a `bug` is not a container, so it may not parent another `bug`.
    let cfg = plugin::load("issue-tracker").unwrap();
    let mut s = store();
    let b1 = create(&mut s, &cfg, "bug", "B1", "P1");
    let b2 = create(&mut s, &cfg, "bug", "B2", "P1");
    let before = s.op_count();
    assert_eq!(
        set_parent(&mut s, &cfg, &b2, &b1).unwrap_err().kind,
        ErrorKind::Validation,
        "a bug may not parent a bug"
    );
    assert_eq!(s.op_count(), before, "the rejected pair writes nothing");

    // personal-todo: a `todo` is not a container, so it may not parent another `todo`.
    let cfg = plugin::load("personal-todo").unwrap();
    let mut s = store();
    let t1 = create(&mut s, &cfg, "todo", "T1", "now");
    let t2 = create(&mut s, &cfg, "todo", "T2", "now");
    assert_eq!(
        set_parent(&mut s, &cfg, &t2, &t1).unwrap_err().kind,
        ErrorKind::Validation,
        "a todo may not parent a todo"
    );
}

#[test]
fn issue_tracker_forbids_a_third_hierarchy_level() {
    // 2 levels is the ceiling, enforced by the pair rules: `epic → bug` is allowed, but nesting
    // BELOW a bug (a bug is not a container) or ABOVE an epic (an epic has no parent rule) is
    // rejected — so a third level can never form.
    let cfg = plugin::load("issue-tracker").unwrap();
    let mut s = store();
    let epic = create(&mut s, &cfg, "epic", "E", "P1");
    let bug = create(&mut s, &cfg, "bug", "B", "P1");
    set_parent(&mut s, &cfg, &bug, &epic).unwrap(); // level 1 → 2, fine

    // A third level under the bug: a second bug parented to the first is rejected (bug ≠ container).
    let bug2 = create(&mut s, &cfg, "bug", "B2", "P1");
    assert_eq!(
        set_parent(&mut s, &cfg, &bug2, &bug).unwrap_err().kind,
        ErrorKind::Validation,
        "no nesting below a bug"
    );

    // A level above the epic: parenting the epic under another epic is rejected (epic has no rule).
    let epic2 = create(&mut s, &cfg, "epic", "E2", "P1");
    assert_eq!(
        set_parent(&mut s, &cfg, &epic, &epic2).unwrap_err().kind,
        ErrorKind::Validation,
        "no nesting above an epic"
    );
}

#[test]
fn pair_rejection_message_distinguishes_top_level_from_undeclared_types() {
    // Integrity review #3: when no parent is allowed, the rejection self-explains rather than
    // emitting a bare "allowed parent types: (none)" — distinguishing a top-level type from a
    // type the active plugin does not declare (cross-plugin / merge-delivered data).
    let cfg = plugin::load("issue-tracker").unwrap();
    let mut s = store();
    let e1 = create(&mut s, &cfg, "epic", "E1", "P1");
    let e2 = create(&mut s, &cfg, "epic", "E2", "P1");

    // A declared but top-level type (epic) cannot be parented.
    let err = set_parent(&mut s, &cfg, &e1, &e2).unwrap_err();
    assert!(
        err.msg.contains("top-level type"),
        "top-level message, got: {}",
        err.msg
    );

    // A type the active plugin does NOT declare: the core is type-agnostic, so seed a foreign-typed
    // item directly, then a reparent attempt names that undeclared type.
    s.create_item("ab12.foreign", "todo", "from another plugin", ACTOR);
    let err = set_parent(&mut s, &cfg, "ab12.foreign", &e1).unwrap_err();
    assert!(
        err.msg.contains("not a type the active plugin declares"),
        "undeclared-type message, got: {}",
        err.msg
    );
}

#[test]
fn personal_todo_forbids_nesting_its_container() {
    // personal-todo is likewise 2 levels: a `project` is the only container and itself has no
    // parent rule, so project-under-project (a third level) is rejected.
    let cfg = plugin::load("personal-todo").unwrap();
    let mut s = store();
    let p1 = create(&mut s, &cfg, "project", "P1", "now");
    let p2 = create(&mut s, &cfg, "project", "P2", "now");
    assert_eq!(
        set_parent(&mut s, &cfg, &p1, &p2).unwrap_err().kind,
        ErrorKind::Validation,
        "a project may not be parented under a project"
    );
}
