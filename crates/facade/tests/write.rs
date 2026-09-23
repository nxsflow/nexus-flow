//! The shared write compute layer (E5 #9t7.5): create/update/claim/close/dep/note/mention over
//! the meaning-free core, with `now`/`actor` as explicit parameters, dependency-cycle rejection
//! at write time, and the closed `FacadeError` kind-set for every rejection. This is the single
//! mutation seam the CLI, the MCP server, and an embedding app all share — the App↔CLI parity
//! differential (#9t7.6) pins it against `nxf` itself; here we pin the contract directly.

use nexus_flow_core::store::Store;
use nexus_flow_facade::error::ErrorKind;
use nexus_flow_facade::plugin::{self, PluginConfig};
use nexus_flow_facade::write::{self, NewItem};

const NOW: &str = "2026-06-17T08:30:00Z";
const ACTOR: &str = "alice";

fn cfg() -> PluginConfig {
    plugin::load("issue-tracker").unwrap()
}

fn store() -> Store {
    Store::open_in_memory(1)
}

/// The mandatory-only `create` payload (description + priority); all optionals empty.
fn minimal<'a>(description: &'a str, priority: &'a str) -> NewItem<'a> {
    NewItem {
        description,
        priority,
        design: None,
        dod: None,
        due: None,
        defer: None,
        parent: None,
        depends_on: &[],
        custom: &[],
    }
}

/// Seed one open issue and return its id — the common precondition for update/close/dep tests.
fn seed(s: &mut Store, cfg: &PluginConfig, title: &str) -> String {
    write::create(
        s,
        cfg,
        "ab12",
        NOW,
        ACTOR,
        "bug",
        title,
        minimal("why", "P1"),
    )
    .unwrap()
    .id
}

#[test]
fn create_writes_the_full_field_model_and_resolves_vocabulary() {
    let mut s = store();
    let cfg = cfg();
    let item = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "bug", // a declared issue-tracker type
        "Title",
        NewItem {
            description: "why",
            priority: "P1",
            design: Some("plan"),
            dod: Some("done-when"),
            due: Some("2026-07-01"),
            defer: Some("2026-06-20"),
            parent: None,
            depends_on: &[],
            custom: &[],
        },
    )
    .unwrap();

    assert_eq!(
        item.item_type.as_deref(),
        Some("bug"),
        "the declared type is stored verbatim"
    );
    assert_eq!(item.title.as_deref(), Some("Title"));
    assert_eq!(item.status.as_deref(), Some("open"));
    assert_eq!(item.description.as_deref(), Some("why"));
    assert_eq!(item.priority.as_deref(), Some("1"), "P1 → ordinal 1");
    assert_eq!(item.design.as_deref(), Some("plan"));
    assert_eq!(item.completion_criterion.as_deref(), Some("done-when"));
    assert_eq!(item.due.as_deref(), Some("2026-07-01"));
    assert_eq!(item.defer_until.as_deref(), Some("2026-06-20"));
}

#[test]
fn now_and_actor_land_on_every_op() {
    // The whole point of the explicit params (#9t7.5): a long-lived host stamps each op with a
    // deterministic time and its own identity, not the process clock / one ambient user.
    let mut s = store();
    let cfg = cfg();
    write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "bug",
        "T",
        minimal("why", "P1"),
    )
    .unwrap();
    let ops = s.export();
    assert!(!ops.is_empty());
    assert!(
        ops.iter().all(|o| o.wall_clock == NOW && o.author == ACTOR),
        "every emitted op carries the explicit now + actor"
    );
}

#[test]
fn writes_reject_a_malformed_now_and_write_nothing() {
    // `now` is an explicit parameter the embedding app supplies directly (no `--now`/`NXF_NOW`
    // validation in front of it), so a malformed pin must fail loudly here rather than silently
    // stamp an arbitrary string into op.wall_clock — matching the CLI's validation (IR review #1).
    let mut s = store();
    let cfg = cfg();

    // create: rejected before anything is minted/written.
    let err = write::create(
        &mut s,
        &cfg,
        "ab12",
        "garbage",
        ACTOR,
        "bug",
        "T",
        minimal("d", "P1"),
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert_eq!(
        s.op_count(),
        0,
        "a write under a malformed now writes nothing"
    );

    // And on existing items, every other write seam rejects a bad now too — each routes its
    // stamp through the same validated helper, after its own existence/cycle checks.
    let a = seed(&mut s, &cfg, "A");
    let b = seed(&mut s, &cfg, "B");
    let before = s.op_count();
    let bad = "nope";
    assert_eq!(
        write::update(&mut s, &cfg, bad, ACTOR, &a, &["title=New".into()])
            .unwrap_err()
            .kind,
        ErrorKind::Validation
    );
    assert!(write::claim(&mut s, bad, ACTOR, &a, None).is_err());
    assert!(write::close(&mut s, bad, ACTOR, &a, Some("r")).is_err());
    assert!(write::note_add(&mut s, bad, ACTOR, &a, "x").is_err());
    assert!(write::dep_add(&mut s, bad, ACTOR, &a, &b).is_err()); // a→b is a valid, non-cyclic edge
    assert!(write::mention_add(&mut s, bad, ACTOR, &a, &b).is_err());
    assert_eq!(
        s.op_count(),
        before,
        "none of the rejected-now writes landed an op"
    );
}

#[test]
fn create_wires_dependencies_as_edges() {
    let mut s = store();
    let cfg = cfg();
    let a = seed(&mut s, &cfg, "A");
    let b = seed(&mut s, &cfg, "B");
    let item = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "bug",
        "C",
        NewItem {
            description: "why",
            priority: "P1",
            design: None,
            dod: None,
            due: None,
            defer: None,
            parent: None,
            depends_on: &[a.clone(), b.clone()],
            custom: &[],
        },
    )
    .unwrap();
    let mut deps = s.deps_of(&item.id).unwrap();
    deps.sort();
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(deps, want, "the new item depends on each listed id");
}

#[test]
fn type_set_is_plugin_declared_not_a_hardcoded_enum() {
    // sp6.2/sp6.4: the valid type set is the ACTIVE PLUGIN's declared `[types].list`, not a
    // hardcoded core {project,task} enum. A plugin that does not declare `task` rejects it — even
    // though it was a core enum variant before — which is the proof the enum gate is gone and the
    // set is config-driven. The stored type is the declared name, verbatim.
    let mut s = store();
    let mut cfg = cfg();
    // Override the type system with a one-type set; nothing else is hardcoded.
    cfg.types = toml::from_str(r#"list = ["epic"]"#).unwrap();

    // The declared type resolves and is stored verbatim.
    assert_eq!(
        write::create(
            &mut s,
            &cfg,
            "ab12",
            NOW,
            ACTOR,
            "epic",
            "E",
            minimal("d", "P1")
        )
        .unwrap()
        .item_type
        .as_deref(),
        Some("epic"),
        "a declared type is stored verbatim"
    );
    // The UNDECLARED former-enum variant `task` is now a validation error.
    let err = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "task",
        "T",
        minimal("d", "P1"),
    )
    .unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::Validation,
        "an undeclared type is rejected — the core {{project,task}} enum no longer gates it"
    );
}

#[test]
fn create_rejects_unknown_type_and_writes_nothing() {
    let mut s = store();
    let cfg = cfg();
    let err = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "nonsense",
        "T",
        minimal("d", "P1"),
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert_eq!(s.op_count(), 0, "a rejected create writes nothing");
}

#[test]
fn create_rejects_unknown_priority() {
    let mut s = store();
    let cfg = cfg();
    let err = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "bug",
        "T",
        minimal("d", "P9"),
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert_eq!(s.op_count(), 0);
}

#[test]
fn create_accepts_the_read_form_priority_ordinal_repairing_the_round_trip() {
    // ee2h (reverses the earlier 3bw6 label-only rule, per the owner's round-trip objection): the
    // write surface now ADDITIVELY accepts the read-form ordinal that `show`/`list`/`next --json`
    // emit (`"2"`), not only the `P2` label — so machine output is valid machine input. It is
    // stored as the ordinal, exactly as the label form is. Pinned here at the facade write seam the
    // `Engine` handle routes through, so the round-trip can't silently regress. An out-of-range
    // ordinal is still rejected (covered by the resolve_priority unit test).
    let mut s = store();
    let cfg = cfg();
    let row = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "bug",
        "T",
        minimal("d", "2"),
    )
    .unwrap();
    assert_eq!(
        row.priority.as_deref(),
        Some("2"),
        "the read-form ordinal key is stored as the ordinal, identical to the label form"
    );
    assert!(s.op_count() > 0, "the accepted create wrote its ops");
}

#[test]
fn create_rejects_a_malformed_due_date() {
    let mut s = store();
    let cfg = cfg();
    let bad = NewItem {
        due: Some("not-a-date"),
        ..minimal("d", "P1")
    };
    let err = write::create(&mut s, &cfg, "ab12", NOW, ACTOR, "bug", "T", bad).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert_eq!(s.op_count(), 0);
}

#[test]
fn create_rejects_a_missing_parent() {
    let mut s = store();
    let cfg = cfg();
    let bad = NewItem {
        parent: Some("ab12.9999"),
        ..minimal("d", "P1")
    };
    let err = write::create(&mut s, &cfg, "ab12", NOW, ACTOR, "bug", "T", bad).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(
        s.op_count(),
        0,
        "a dangling parent reference writes nothing"
    );
}

#[test]
fn update_sets_whitelisted_fields_and_resolves_aliases() {
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    let updated = write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        &[
            "title=New".into(),
            "defer=2026-07-01".into(),
            "priority=P0".into(),
        ],
    )
    .unwrap();
    assert_eq!(updated.title.as_deref(), Some("New"));
    assert_eq!(
        updated.defer_until.as_deref(),
        Some("2026-07-01"),
        "defer alias"
    );
    assert_eq!(updated.priority.as_deref(), Some("0"), "P0 → ordinal 0");
}

#[test]
fn update_clears_an_optional_field_with_an_empty_value() {
    // 6j6v.68ke: `--set defer=` (an empty value) CLEARS the field. A date field was previously
    // rejected outright (iso_date fails on ""), so with no --unset the only escape from the deferred
    // lane was to set a stale past date. Empty-as-clear is consistent across due/defer/assignee and
    // with how declared custom fields already treat an empty value (§4.2).
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        &[
            "defer=2026-07-01".into(),
            "due=2026-08-01".into(),
            "assignee=dev".into(),
        ],
    )
    .unwrap();

    let cleared = write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        &["defer=".into(), "due=".into(), "assignee=".into()],
    )
    .unwrap();
    assert_eq!(cleared.defer_until, None, "empty defer clears defer_until");
    assert_eq!(cleared.due, None, "empty due clears due");
    assert_eq!(cleared.assignee, None, "empty assignee clears assignee");
}

#[test]
fn update_rejects_clearing_a_required_or_validated_field_with_an_empty_value() {
    // Empty-as-clear is ONLY for optional fields: a validated field (status/priority) must still
    // reject an empty value rather than silently blanking a required cell.
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    assert_eq!(
        write::update(&mut s, &cfg, NOW, ACTOR, &id, &["status=".into()])
            .unwrap_err()
            .kind,
        ErrorKind::Validation,
    );
    assert_eq!(
        write::update(&mut s, &cfg, NOW, ACTOR, &id, &["priority=".into()])
            .unwrap_err()
            .kind,
        ErrorKind::Validation,
    );
}

#[test]
fn update_rejects_unknown_field_and_invalid_status() {
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");

    let err = write::update(&mut s, &cfg, NOW, ACTOR, &id, &["bogus=1".into()]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);

    let err = write::update(&mut s, &cfg, NOW, ACTOR, &id, &["status=nope".into()]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
}

#[test]
fn update_is_all_or_nothing_when_one_set_is_invalid() {
    // Parse+validate everything before writing any of it, so a partially-bad batch leaves the
    // item untouched (mirrors the CLI's `update`).
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    let before = s.op_count();
    let err = write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        &["title=Good".into(), "status=nope".into()],
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert_eq!(
        s.op_count(),
        before,
        "no field written when any set is invalid"
    );
    assert_eq!(
        s.get_item(&id).unwrap().unwrap().title.as_deref(),
        Some("A")
    );
}

#[test]
fn update_of_a_missing_item_is_not_found() {
    let mut s = store();
    let cfg = cfg();
    let err =
        write::update(&mut s, &cfg, NOW, ACTOR, "ab12.9999", &["title=x".into()]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn update_rejects_a_dangling_parent_and_writes_nothing() {
    // Mirror `create`'s `--parent` existence check on `update --set parent=<id>` (the
    // `belongs_to` alias): a dangling structural parent is rejected `not_found` and nothing is
    // written, so a host can't point `belongs_to` at a non-existent item (nexus-flow-9t7.10).
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    let before = s.op_count();
    let err =
        write::update(&mut s, &cfg, NOW, ACTOR, &id, &["parent=ab12.9999".into()]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(
        s.op_count(),
        before,
        "a dangling parent reference writes nothing"
    );
    assert_eq!(
        s.get_item(&id).unwrap().unwrap().belongs_to,
        None,
        "the rejected update left belongs_to unset"
    );
}

#[test]
fn update_sets_parent_to_a_live_item() {
    // The accept branch of the parent guard (nexus-flow-9t7.10 review, Test Quality #1): pointing
    // `belongs_to` at a live item succeeds and records the edge. Paired with the dangling-parent
    // test, this proves the guard rejects *missing* parents specifically — not every parent.
    let mut s = store();
    let cfg = cfg();
    let parent = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "epic",
        "P",
        minimal("why", "P1"),
    )
    .unwrap()
    .id;
    let child = seed(&mut s, &cfg, "child");
    let updated = write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &child,
        &[format!("parent={parent}")],
    )
    .unwrap();
    assert_eq!(
        updated.belongs_to.as_deref(),
        Some(parent.as_str()),
        "a live parent is recorded"
    );
}

#[test]
fn update_rejects_a_tombstoned_parent_and_writes_nothing() {
    // `require_live` rejects a *tombstoned* (soft-deleted) parent, not just a never-existed id —
    // the branch sync reaches by delivering a tombstone (nexus-flow-9t7.10 review, Test Quality
    // #2). Same `not_found` + writes-nothing contract as the dangling case.
    let mut s = store();
    let cfg = cfg();
    let parent = seed(&mut s, &cfg, "P");
    let child = seed(&mut s, &cfg, "child");
    s.delete_item(&parent, ACTOR); // tombstone it (deleted = "1")
    let before = s.op_count();
    let err = write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &child,
        &[format!("parent={parent}")],
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(s.op_count(), before, "a tombstoned parent writes nothing");
    assert_eq!(s.get_item(&child).unwrap().unwrap().belongs_to, None);
}

#[test]
fn create_rejects_a_tombstoned_parent() {
    // Same tombstone rejection on the create seam (nexus-flow-9t7.10 review, Test Quality #2).
    let mut s = store();
    let cfg = cfg();
    let parent = seed(&mut s, &cfg, "P");
    s.delete_item(&parent, ACTOR);
    let before = s.op_count();
    let bad = NewItem {
        parent: Some(&parent),
        ..minimal("d", "P1")
    };
    let err = write::create(&mut s, &cfg, "ab12", NOW, ACTOR, "bug", "T", bad).unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
    assert_eq!(s.op_count(), before, "a tombstoned parent writes nothing");
}

#[test]
fn create_rejects_a_non_container_parent_and_writes_nothing() {
    // The relationship matrix (sp6.4) only allows an `epic` parent of a `bug`; a parent that exists
    // and is live but is itself a `bug` (not a declared container) is rejected `validation` before
    // any op is written. The convergence-time `invariant` keeps the structural backstop; the
    // type-pair policy is the plugin's, enforced here.
    let mut s = store();
    let cfg = cfg();
    let bug_parent = seed(&mut s, &cfg, "T"); // a `bug` — not a container
    let before = s.op_count();
    let bad = NewItem {
        parent: Some(&bug_parent),
        ..minimal("d", "P1")
    };
    let err = write::create(&mut s, &cfg, "ab12", NOW, ACTOR, "bug", "C", bad).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert_eq!(
        s.op_count(),
        before,
        "a non-container parent writes nothing"
    );
}

#[test]
fn create_accepts_a_container_parent() {
    // The accept branch of the matrix pair check (sp6.4): a live `epic` (a declared container) is
    // recorded as a `bug`'s parent. Paired with the non-container / missing / tombstoned reject
    // tests, this proves the guard rejects the wrong *type* specifically — not every parent.
    let mut s = store();
    let cfg = cfg();
    let parent = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "epic",
        "P",
        minimal("why", "P1"),
    )
    .unwrap()
    .id;
    let child = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "bug",
        "C",
        NewItem {
            parent: Some(&parent),
            ..minimal("d", "P1")
        },
    )
    .unwrap();
    assert_eq!(
        child.belongs_to.as_deref(),
        Some(parent.as_str()),
        "a container (epic) parent is recorded"
    );
}

#[test]
fn update_rejects_a_non_container_parent_and_writes_nothing() {
    // The same matrix pair check on `update --set parent=` (sp6.4): re-pointing `belongs_to` at a
    // live but non-container item (a `bug`) is rejected `validation` and writes nothing — symmetric
    // with `create` and with the dangling / self / tombstoned guards here.
    let mut s = store();
    let cfg = cfg();
    let bug_parent = seed(&mut s, &cfg, "T"); // a `bug` — not a container
    let child = seed(&mut s, &cfg, "child");
    let before = s.op_count();
    let err = write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &child,
        &[format!("parent={bug_parent}")],
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert_eq!(
        s.op_count(),
        before,
        "a non-container parent writes nothing"
    );
    assert_eq!(
        s.get_item(&child).unwrap().unwrap().belongs_to,
        None,
        "the rejected update left belongs_to unset"
    );
}

#[test]
fn update_rejects_an_item_as_its_own_parent() {
    // A self-parent is a nonsensical (degenerate) hierarchy edge; reject it before any write,
    // mirroring `dep_add`'s self-edge guard (nexus-flow-9t7.10 review, Integrity #2). Only
    // reachable on `update` — `create` mints a fresh unique id, so a new item can never name
    // itself as parent.
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    let before = s.op_count();
    let err = write::update(&mut s, &cfg, NOW, ACTOR, &id, &[format!("parent={id}")]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert_eq!(s.op_count(), before, "a self-parent writes nothing");
    assert_eq!(s.get_item(&id).unwrap().unwrap().belongs_to, None);
}

#[test]
fn update_rejects_a_parent_cycle_and_writes_nothing() {
    // sp6.8 (②d): a parent edge that would close a cycle over the parent relation is rejected at
    // write time with `kind=cycle` — analogous to `dep_add`'s cycle guard (E2 §5) — and nothing is
    // persisted. Only `update` can hit it; `create` mints a fresh id that nothing can reach.
    let mut s = store();
    // issue-tracker forbids parenting an epic, so a parent CHAIN can't form under it; the cycle
    // guard needs a chain-capable matrix. Override the type system with one that lets an epic
    // parent an epic — the rule is the plugin's, and this test declares a permissive one.
    let mut cfg = cfg();
    cfg.types = toml::from_str(
        r#"list = ["epic"]
        [rules.epic]
        parents = ["epic"]
    "#,
    )
    .unwrap();
    // A chain c → b → a (each belongs to the next).
    let epic = |s: &mut Store, t: &str| {
        write::create(s, &cfg, "ab12", NOW, ACTOR, "epic", t, minimal("d", "P1"))
            .unwrap()
            .id
    };
    let a = epic(&mut s, "A");
    let b = epic(&mut s, "B");
    let c = epic(&mut s, "C");
    write::update(&mut s, &cfg, NOW, ACTOR, &b, &[format!("parent={a}")]).unwrap();
    write::update(&mut s, &cfg, NOW, ACTOR, &c, &[format!("parent={b}")]).unwrap();

    // a's parent = c would close a → c → b → a.
    let before = s.op_count();
    let err = write::update(&mut s, &cfg, NOW, ACTOR, &a, &[format!("parent={c}")]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Cycle);
    assert_eq!(s.op_count(), before, "a parent cycle writes nothing");
    assert_eq!(
        s.get_item(&a).unwrap().unwrap().belongs_to,
        None,
        "a stays a root"
    );
}

#[test]
fn claim_marks_in_progress_and_records_assignee() {
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    let item = write::claim(&mut s, NOW, ACTOR, &id, Some("bob")).unwrap();
    assert_eq!(item.status.as_deref(), Some("in_progress"));
    assert_eq!(item.assignee.as_deref(), Some("bob"));
}

#[test]
fn claim_propagates_in_progress_up_the_full_parent_chain() {
    // 07a.1: claiming a child marks every OPEN ancestor `in_progress` (claim-up), up the FULL
    // parent chain — multi-parent, transitively. A `closed` ancestor is NOT revived; an already
    // `in_progress` one is a no-op. Built at the store level so the chain isn't gated by the
    // creation matrix. NOTE (07a.2 review): a blocked/deferred ancestor is stored `open`, so it is
    // flipped too — the existing "claimed-then-blocked" state (reachable via `list`).
    let mut s = store();
    s.create_item("c1.E", "project", "E", "x"); // open grandparent
    s.create_item("c1.G", "project", "G", "x"); // a second parent of F, closed below
    s.create_item("c1.F", "task", "F", "x"); // open parent
    s.create_item("c1.T", "task", "T", "x"); // the claimed child
    s.add_parent("c1.F", "c1.E", "x");
    s.add_parent("c1.F", "c1.G", "x");
    s.add_parent("c1.T", "c1.F", "x");
    s.set_field("c1.G", "status", Some("closed".into()), "x"); // a closed ancestor

    write::claim(&mut s, NOW, ACTOR, "c1.T", None).unwrap();

    let st = |id: &str| s.get_item(id).unwrap().unwrap().status;
    assert_eq!(
        st("c1.T").as_deref(),
        Some("in_progress"),
        "the claimed item"
    );
    assert_eq!(
        st("c1.F").as_deref(),
        Some("in_progress"),
        "an open parent is flipped to in_progress"
    );
    assert_eq!(
        st("c1.E").as_deref(),
        Some("in_progress"),
        "an open grandparent is flipped too (the full chain)"
    );
    assert_eq!(
        st("c1.G").as_deref(),
        Some("closed"),
        "a closed ancestor is NOT revived"
    );
}

#[test]
fn claim_up_terminates_on_a_diamond_and_flips_a_shared_ancestor() {
    // Determinism/termination (spec §7): the ancestor walk is bounded by a visited set, so a diamond
    // (two parent paths converging on one grandparent) terminates and the shared grandparent is
    // flipped — once. If the walk looped, this test would hang rather than return.
    let mut s = store();
    s.create_item("c1.G", "project", "G", "x"); // shared grandparent
    s.create_item("c1.P1", "task", "P1", "x");
    s.create_item("c1.P2", "task", "P2", "x");
    s.create_item("c1.T", "task", "T", "x");
    s.add_parent("c1.P1", "c1.G", "x");
    s.add_parent("c1.P2", "c1.G", "x");
    s.add_parent("c1.T", "c1.P1", "x");
    s.add_parent("c1.T", "c1.P2", "x"); // T → {P1, P2} → G (diamond)

    write::claim(&mut s, NOW, ACTOR, "c1.T", None).unwrap();
    for id in ["c1.P1", "c1.P2", "c1.G"] {
        assert_eq!(
            s.get_item(id).unwrap().unwrap().status.as_deref(),
            Some("in_progress"),
            "{id} is flipped exactly once along the diamond"
        );
    }
}

#[test]
fn reopening_a_closed_item_clears_closed_at() {
    // C5 review #3: `closed_at` is present iff status == closed. Reopening via `claim` must clear
    // the stale close instant, exactly as `unarchive` clears `archived`.
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    let closed = write::close(&mut s, NOW, ACTOR, &id, Some("done")).unwrap();
    assert_eq!(closed.closed_at.as_deref(), Some(NOW));

    let reopened = write::claim(&mut s, NOW, ACTOR, &id, None).unwrap();
    assert_eq!(reopened.status.as_deref(), Some("in_progress"));
    assert_eq!(
        reopened.closed_at, None,
        "reopening via claim clears the stale close instant"
    );
}

#[test]
fn update_keeps_closed_at_in_sync_with_a_status_transition() {
    // The same invariant via `update`: a status move INTO closed stamps the instant, a move OUT
    // of closed clears it. (The sanctioned close path is `close`; `update status=…` must not be
    // able to leave a closed item without an instant or a reopened one with a stale one.)
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");

    let closed = write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        &["status=closed".to_string()],
    )
    .unwrap();
    assert_eq!(closed.status.as_deref(), Some("closed"));
    assert_eq!(
        closed.closed_at.as_deref(),
        Some(NOW),
        "update into closed stamps the close instant"
    );

    let reopened =
        write::update(&mut s, &cfg, NOW, ACTOR, &id, &["status=open".to_string()]).unwrap();
    assert_eq!(reopened.status.as_deref(), Some("open"));
    assert_eq!(
        reopened.closed_at, None,
        "update out of closed clears the close instant"
    );
}

#[test]
fn close_requires_a_reason_and_records_it() {
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");

    let err = write::close(&mut s, NOW, ACTOR, &id, None).unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::Validation,
        "closing without a reason is rejected"
    );

    let item = write::close(&mut s, NOW, ACTOR, &id, Some("done")).unwrap();
    assert_eq!(item.status.as_deref(), Some("closed"));
    assert_eq!(item.closing_comment.as_deref(), Some("done"));
    // C3 (#916.4): close stamps the `closed_at` instant from the injected `now`, so the `closed`
    // lane can order by close-date — exactly the role `archived` plays for the archived lane.
    assert_eq!(
        item.closed_at.as_deref(),
        Some(NOW),
        "close records the close instant"
    );
}

#[test]
fn dep_add_rejects_self_and_transitive_cycles() {
    let mut s = store();
    let cfg = cfg();
    let a = seed(&mut s, &cfg, "A");
    let b = seed(&mut s, &cfg, "B");

    write::dep_add(&mut s, NOW, ACTOR, &a, &b).unwrap(); // A depends on B

    let err = write::dep_add(&mut s, NOW, ACTOR, &b, &a).unwrap_err(); // B->A closes a loop
    assert_eq!(err.kind, ErrorKind::Cycle);

    let err = write::dep_add(&mut s, NOW, ACTOR, &a, &a).unwrap_err(); // self-edge
    assert_eq!(err.kind, ErrorKind::Cycle);
}

#[test]
fn dep_add_requires_both_endpoints_to_exist() {
    let mut s = store();
    let cfg = cfg();
    let a = seed(&mut s, &cfg, "A");
    let err = write::dep_add(&mut s, NOW, ACTOR, &a, "ab12.9999").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn note_add_returns_an_id_and_stamps_now_actor() {
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "A");
    let note_id = write::note_add(&mut s, NOW, ACTOR, &id, "worklog").unwrap();
    let notes = s.notes_of(&id).unwrap();
    assert_eq!(notes, vec![(note_id, "worklog".to_string())]);
    // The note op (and only it, beyond the seed) carries the explicit now+actor.
    assert!(s
        .export()
        .iter()
        .filter(|o| o.target_kind == "note")
        .all(|o| o.wall_clock == NOW && o.author == ACTOR));
}

#[test]
fn mention_and_contributes_add_then_remove() {
    let mut s = store();
    let cfg = cfg();
    let a = seed(&mut s, &cfg, "A");
    let b = seed(&mut s, &cfg, "B");

    write::mention_add(&mut s, NOW, ACTOR, &a, &b).unwrap();
    assert_eq!(s.mentions_of(&a), vec![b.clone()]);
    write::mention_remove(&mut s, NOW, ACTOR, &a, &b).unwrap();
    assert!(s.mentions_of(&a).is_empty());

    write::contributes_add(&mut s, NOW, ACTOR, &a, &b).unwrap();
    assert_eq!(s.contributes_to_of(&a).unwrap(), vec![b.clone()]);
    write::contributes_remove(&mut s, NOW, ACTOR, &a, &b).unwrap();
    assert!(s.contributes_to_of(&a).unwrap().is_empty());

    let err = write::mention_add(&mut s, NOW, ACTOR, &a, "ab12.9999").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

// ---- archive / unarchive (C5 #916.5) ---------------------------------------

/// Create a child issue under `parent` and return its id.
fn seed_child(s: &mut Store, cfg: &PluginConfig, title: &str, parent: &str) -> String {
    let new = NewItem {
        parent: Some(parent),
        ..minimal("why", "P1")
    };
    write::create(s, cfg, "ab12", NOW, ACTOR, "bug", title, new)
        .unwrap()
        .id
}

/// Seed a closed epic with two closed children; returns (epic, child_a, child_b).
fn closed_epic_with_two_children(s: &mut Store, cfg: &PluginConfig) -> (String, String, String) {
    let epic = write::create(
        s,
        cfg,
        "ab12",
        NOW,
        ACTOR,
        "epic",
        "E",
        minimal("why", "P1"),
    )
    .unwrap()
    .id;
    let a = seed_child(s, cfg, "A", &epic);
    let b = seed_child(s, cfg, "B", &epic);
    write::close(s, NOW, ACTOR, &a, Some("done")).unwrap();
    write::close(s, NOW, ACTOR, &b, Some("done")).unwrap();
    write::close(s, NOW, ACTOR, &epic, Some("done")).unwrap();
    (epic, a, b)
}

fn is_archived(s: &Store, id: &str) -> bool {
    s.get_item(id).unwrap().unwrap().archived.is_some()
}

#[test]
fn archive_cascades_down_a_fully_closed_subtree() {
    let mut s = store();
    let cfg = cfg();
    let (epic, a, b) = closed_epic_with_two_children(&mut s, &cfg);

    let res = write::archive(&mut s, NOW, ACTOR, std::slice::from_ref(&epic)).unwrap();
    assert_eq!(res.outcomes.len(), 1);
    assert!(res.outcomes[0].ok, "a fully-closed subtree archives");
    assert_eq!(res.outcomes[0].id, epic);
    // affected carries the whole cascaded subtree, id-sorted.
    let mut expected = vec![epic.clone(), a.clone(), b.clone()];
    expected.sort();
    assert_eq!(
        res.affected, expected,
        "cascade archives the epic AND every descendant"
    );
    assert!(is_archived(&s, &epic) && is_archived(&s, &a) && is_archived(&s, &b));
    // The stamped instant is the injected `now`.
    assert_eq!(
        s.get_item(&epic).unwrap().unwrap().archived.as_deref(),
        Some(NOW)
    );

    // Idempotent: re-archiving an already-archived subtree succeeds and changes nothing new.
    let again = write::archive(&mut s, NOW, ACTOR, std::slice::from_ref(&epic)).unwrap();
    assert!(again.outcomes[0].ok);
    assert!(
        again.affected.is_empty(),
        "nothing newly archived on a re-run"
    );
}

#[test]
fn archive_rejects_an_open_root_and_writes_nothing() {
    let mut s = store();
    let cfg = cfg();
    let epic = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "epic",
        "E",
        minimal("w", "P1"),
    )
    .unwrap()
    .id; // left open

    let res = write::archive(&mut s, NOW, ACTOR, std::slice::from_ref(&epic)).unwrap();
    assert!(!res.outcomes[0].ok);
    assert_eq!(
        res.outcomes[0].reason,
        Some(write::ArchiveReason::NotClosed)
    );
    assert!(res.affected.is_empty());
    assert!(!is_archived(&s, &epic), "a rejected archive writes nothing");
}

#[test]
fn archive_rejects_a_subtree_with_an_open_descendant_atomically() {
    let mut s = store();
    let cfg = cfg();
    let epic = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "epic",
        "E",
        minimal("w", "P1"),
    )
    .unwrap()
    .id;
    let a = seed_child(&mut s, &cfg, "A", &epic);
    let b = seed_child(&mut s, &cfg, "B", &epic);
    write::close(&mut s, NOW, ACTOR, &a, Some("done")).unwrap();
    write::close(&mut s, NOW, ACTOR, &epic, Some("done")).unwrap();
    // b stays OPEN.

    let res = write::archive(&mut s, NOW, ACTOR, std::slice::from_ref(&epic)).unwrap();
    assert!(!res.outcomes[0].ok);
    assert_eq!(
        res.outcomes[0].reason,
        Some(write::ArchiveReason::HasOpenDescendants)
    );
    // Atomic per root: NOTHING in the subtree is archived, not even the closed root/child.
    assert!(!is_archived(&s, &epic) && !is_archived(&s, &a) && !is_archived(&s, &b));
    assert!(res.affected.is_empty());
    let _ = b;
}

#[test]
fn archive_an_already_archived_descendant_counts_as_closed() {
    let mut s = store();
    let cfg = cfg();
    let (epic, a, b) = closed_epic_with_two_children(&mut s, &cfg);
    // Pre-archive child A at an earlier instant; it must count as "closed" for the precondition
    // and keep its original archive instant (idempotent — not re-stamped).
    s.set_field(&a, "archived", Some("2026-01-01T00:00:00Z".into()), "t");

    let res = write::archive(&mut s, NOW, ACTOR, std::slice::from_ref(&epic)).unwrap();
    assert!(
        res.outcomes[0].ok,
        "a pre-archived descendant satisfies the closed precondition"
    );
    assert_eq!(
        s.get_item(&a).unwrap().unwrap().archived.as_deref(),
        Some("2026-01-01T00:00:00Z"),
        "an already-archived descendant keeps its original instant"
    );
    assert!(is_archived(&s, &epic) && is_archived(&s, &b));
    // Only the newly-archived ids are reported as affected (A was already archived).
    let mut expected = vec![epic, b];
    expected.sort();
    assert_eq!(res.affected, expected);
}

#[test]
fn archive_not_found_root_fails_without_touching_others() {
    let mut s = store();
    let cfg = cfg();
    let (epic, _, _) = closed_epic_with_two_children(&mut s, &cfg);

    let res = write::archive(&mut s, NOW, ACTOR, &["nope".to_string(), epic.clone()]).unwrap();
    assert_eq!(
        res.outcomes.len(),
        2,
        "one outcome per input id, input order"
    );
    assert!(!res.outcomes[0].ok);
    assert_eq!(res.outcomes[0].reason, Some(write::ArchiveReason::NotFound));
    assert!(
        res.outcomes[1].ok,
        "the second, valid root still archives (roots are independent)"
    );
}

#[test]
fn archive_batch_is_independent_between_roots() {
    let mut s = store();
    let cfg = cfg();
    let ok_epic = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "epic",
        "OK",
        minimal("w", "P1"),
    )
    .unwrap()
    .id;
    write::close(&mut s, NOW, ACTOR, &ok_epic, Some("done")).unwrap();
    let bad_epic = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "epic",
        "BAD",
        minimal("w", "P1"),
    )
    .unwrap()
    .id; // left open

    let res = write::archive(&mut s, NOW, ACTOR, &[ok_epic.clone(), bad_epic.clone()]).unwrap();
    assert!(res.outcomes[0].ok, "the closed root archives");
    assert!(!res.outcomes[1].ok, "the open root fails");
    assert!(is_archived(&s, &ok_epic) && !is_archived(&s, &bad_epic));
    assert_eq!(res.affected, vec![ok_epic]);
}

#[test]
fn unarchive_cascades_up_only_surfacing_the_ancestor_chain() {
    let mut s = store();
    let cfg = cfg();
    let (epic, a, b) = closed_epic_with_two_children(&mut s, &cfg);
    write::archive(&mut s, NOW, ACTOR, std::slice::from_ref(&epic)).unwrap(); // archive the whole subtree

    // Unarchive child A: it and its ancestor (the epic) resurface; sibling B stays archived.
    let res = write::unarchive(&mut s, NOW, ACTOR, std::slice::from_ref(&a)).unwrap();
    assert!(res.outcomes[0].ok);
    let mut expected = vec![a.clone(), epic.clone()];
    expected.sort();
    assert_eq!(
        res.affected, expected,
        "unarchive surfaces the item AND its ancestors"
    );
    assert!(
        !is_archived(&s, &a) && !is_archived(&s, &epic),
        "A and its ancestor are visible"
    );
    assert!(
        is_archived(&s, &b),
        "the sibling stays archived — no downward/sideways cascade"
    );
}

#[test]
fn unarchive_a_parent_does_not_restore_its_children() {
    let mut s = store();
    let cfg = cfg();
    let (epic, a, b) = closed_epic_with_two_children(&mut s, &cfg);
    write::archive(&mut s, NOW, ACTOR, std::slice::from_ref(&epic)).unwrap();

    let res = write::unarchive(&mut s, NOW, ACTOR, std::slice::from_ref(&epic)).unwrap();
    assert!(res.outcomes[0].ok);
    assert_eq!(res.affected, vec![epic.clone()], "only the epic resurfaces");
    assert!(!is_archived(&s, &epic));
    assert!(
        is_archived(&s, &a) && is_archived(&s, &b),
        "children stay archived (up-only cascade)"
    );
}

#[test]
fn unarchive_rejects_a_not_archived_item() {
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "live");

    let res = write::unarchive(&mut s, NOW, ACTOR, std::slice::from_ref(&id)).unwrap();
    assert!(!res.outcomes[0].ok);
    assert_eq!(
        res.outcomes[0].reason,
        Some(write::ArchiveReason::NotArchived)
    );
    assert!(res.affected.is_empty());
}

#[test]
fn unarchive_not_found_root_fails() {
    let mut s = store();
    let res = write::unarchive(&mut s, NOW, ACTOR, &["nope".to_string()]).unwrap();
    assert!(!res.outcomes[0].ok);
    assert_eq!(res.outcomes[0].reason, Some(write::ArchiveReason::NotFound));
}

#[test]
fn archive_and_unarchive_reject_a_malformed_now() {
    let mut s = store();
    let cfg = cfg();
    let (epic, _, _) = closed_epic_with_two_children(&mut s, &cfg);
    assert!(write::archive(&mut s, "not-a-date", ACTOR, std::slice::from_ref(&epic)).is_err());
    assert!(write::unarchive(&mut s, "not-a-date", ACTOR, &[epic]).is_err());
}

// ---- labels (h89s.2): the additive write seam over the core OR-set ----------

#[test]
fn label_add_attaches_and_label_remove_detaches() {
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "labelled");
    write::label_add(&mut s, NOW, ACTOR, &id, "urgent").unwrap();
    write::label_add(&mut s, NOW, ACTOR, &id, "backend").unwrap();
    assert_eq!(
        s.labels_of(&id).unwrap(),
        vec!["backend".to_string(), "urgent".to_string()],
        "both labels present, sorted"
    );
    write::label_remove(&mut s, NOW, ACTOR, &id, "urgent").unwrap();
    assert_eq!(
        s.labels_of(&id).unwrap(),
        vec!["backend".to_string()],
        "observed-remove detaches just the one label"
    );
}

#[test]
fn label_add_trims_surrounding_whitespace() {
    // A padded label canonicalizes to its trimmed form, so `" urgent"` and `"urgent"` are one
    // label — and a later remove of the trimmed form still matches.
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "x");
    write::label_add(&mut s, NOW, ACTOR, &id, "  urgent  ").unwrap();
    assert_eq!(s.labels_of(&id).unwrap(), vec!["urgent".to_string()]);
    write::label_remove(&mut s, NOW, ACTOR, &id, "urgent").unwrap();
    assert!(s.labels_of(&id).unwrap().is_empty());
}

#[test]
fn label_add_on_a_missing_item_is_not_found() {
    let mut s = store();
    let err = write::label_add(&mut s, NOW, ACTOR, "nope", "x").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn label_remove_on_a_missing_item_is_not_found() {
    let mut s = store();
    let err = write::label_remove(&mut s, NOW, ACTOR, "nope", "x").unwrap_err();
    assert_eq!(err.kind, ErrorKind::NotFound);
}

#[test]
fn label_add_rejects_a_blank_label() {
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "x");
    let err = write::label_add(&mut s, NOW, ACTOR, &id, "   ").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
}

#[test]
fn label_add_rejects_a_separator_in_the_label() {
    // A U+001F would break the composite target_id; the facade rejects it as a loud validation
    // error rather than letting it reach the core's panic guard.
    let mut s = store();
    let cfg = cfg();
    let id = seed(&mut s, &cfg, "x");
    let err = write::label_add(&mut s, NOW, ACTOR, &id, "a\u{1f}b").unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
}

// ---- plugin custom fields (T3, 6j6v.p7b2): the write path --------------------------------------
//
// The bundled plugins declare no `[fields]`, so these build a PluginConfig inline that DOES — a
// `file` type with a required `uri` text field + a crdt-text `body`, and a `project` with a `stage`
// enum + a `deadline` date. The assertions read the emitted `("field", set*)` ops back off the log
// (`export`), the shape T4 will later fold + surface.

/// A PluginConfig declaring custom fields (plugin-custom-fields §3), the fixture the T3 write-path
/// tests vary. `uri` (required text on `file`), `body` (crdt-text longtext on `file`), `stage`
/// (enum on `project`), `deadline` (date on `project`).
fn cfg_custom() -> PluginConfig {
    toml::from_str(
        r#"
        name = "custom-fixture"
        [description]
        en = "x"
        de = "y"
        [priority]
        labels = ["P1"]
        [types]
        list = ["file", "project"]
        [vocabulary.status]
        open = "open"
        in_progress = "in progress"
        closed = "closed"
        [ranking.next]
        order = [ { field = "id", dir = "asc" } ]
        [presentation.list]
        columns = ["id"]

        [fields.uri]
        type = "text"
        on = ["file"]
        required = true

        [fields.body]
        type = "longtext"
        on = ["file"]
        merge = "crdt-text"

        [fields.stage]
        type = "enum"
        values = ["backlog", "active", "done"]
        on = ["project"]

        [fields.deadline]
        type = "date"
        on = ["project"]
        "#,
    )
    .unwrap()
}

/// The `(field, op_type, value)` of every `target_kind = "field"` op on `item_id` (the custom-field
/// writes), in log order — the shape the assertions read back.
fn custom_ops(s: &Store, item_id: &str) -> Vec<(String, String, String)> {
    s.export()
        .into_iter()
        .filter(|o| o.target_kind == "field" && o.target_id == item_id)
        .map(|o| {
            (
                o.field.clone(),
                o.op_type.clone(),
                o.value.clone().unwrap_or_default(),
            )
        })
        .collect()
}

/// Create a `file` supplying the required `uri` — the precondition for the update tests.
fn seed_file(s: &mut Store, cfg: &PluginConfig, uri: &str) -> String {
    let custom = vec![format!("uri={uri}")];
    write::create(
        s,
        cfg,
        "ab12",
        NOW,
        ACTOR,
        "file",
        "F",
        NewItem {
            description: "why",
            priority: "P1",
            design: None,
            dod: None,
            due: None,
            defer: None,
            parent: None,
            depends_on: &[],
            custom: &custom,
        },
    )
    .unwrap()
    .id
}

#[test]
fn create_with_a_custom_set_emits_a_field_op_with_the_right_name_value_and_strategy() {
    let mut s = store();
    let cfg = cfg_custom();
    let custom = vec![
        "uri=.nxs/files/report.md".to_string(),
        "body=draft".to_string(),
    ];
    let item = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "file",
        "F",
        NewItem {
            description: "why",
            priority: "P1",
            design: None,
            dod: None,
            due: None,
            defer: None,
            parent: None,
            depends_on: &[],
            custom: &custom,
        },
    )
    .unwrap();
    let ops = custom_ops(&s, &item.id);
    assert!(
        ops.contains(&("uri".into(), "set".into(), ".nxs/files/report.md".into())),
        "the lww uri custom set emits a bare ('field','set') op: {ops:?}"
    );
    assert!(
        ops.contains(&("body".into(), "set:crdt-text".into(), "draft".into())),
        "the crdt-text body custom set carries the strategy marker: {ops:?}"
    );
}

#[test]
fn update_with_a_custom_set_emits_a_field_op() {
    let mut s = store();
    let cfg = cfg_custom();
    let id = seed_file(&mut s, &cfg, "old");
    write::update(&mut s, &cfg, NOW, ACTOR, &id, &["uri=new".to_string()]).unwrap();
    let ops = custom_ops(&s, &id);
    assert_eq!(
        ops.last(),
        Some(&("uri".to_string(), "set".to_string(), "new".to_string())),
        "update --set uri= emits a ('field','set') op with the new value: {ops:?}"
    );
}

#[test]
fn a_custom_field_set_on_a_non_applicable_type_is_a_validation_error() {
    // `uri` is scoped `on = ["file"]`; setting it on a `project` (via update) is rejected. Create a
    // project (no required field applies to it) then try to set uri.
    let mut s = store();
    let cfg = cfg_custom();
    let id = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "project",
        "P",
        minimal("why", "P1"),
    )
    .unwrap()
    .id;
    let err = write::update(&mut s, &cfg, NOW, ACTOR, &id, &["uri=x".to_string()]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("uri") && err.msg.contains("project"),
        "the error names the field and the offending type: {}",
        err.msg
    );
}

#[test]
fn create_missing_a_required_custom_field_is_rejected_and_writes_nothing() {
    let mut s = store();
    let cfg = cfg_custom();
    let before = s.export().len();
    let err = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "file",
        "F",
        minimal("why", "P1"), // no uri
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("uri") && err.msg.contains("required"),
        "the error names the missing required field: {}",
        err.msg
    );
    assert_eq!(s.export().len(), before, "a rejected create writes nothing");
}

#[test]
fn required_is_not_enforced_on_update_clearing_is_allowed() {
    // TB-CF-7: `required` is a create-time guard only. Clearing a required custom field via
    // `update --set uri=` (empty) is allowed and stores an empty-value field set.
    let mut s = store();
    let cfg = cfg_custom();
    let id = seed_file(&mut s, &cfg, "present");
    write::update(&mut s, &cfg, NOW, ACTOR, &id, &["uri=".to_string()]).unwrap();
    let ops = custom_ops(&s, &id);
    assert_eq!(
        ops.last(),
        Some(&("uri".to_string(), "set".to_string(), String::new())),
        "clearing a required field on update is allowed — an empty-value ('field','set'): {ops:?}"
    );
}

#[test]
fn an_undeclared_custom_field_lists_canonical_and_custom_names() {
    let mut s = store();
    let cfg = cfg_custom();
    let id = seed_file(&mut s, &cfg, "u");
    let err = write::update(&mut s, &cfg, NOW, ACTOR, &id, &["nope=x".to_string()]).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("title") && err.msg.contains("uri") && err.msg.contains("stage"),
        "the not-updatable error lists both canonical (title) and custom (uri/stage) names: {}",
        err.msg
    );
}

#[test]
fn an_enum_custom_field_rejects_a_non_member_and_accepts_a_member() {
    let mut s = store();
    let cfg = cfg_custom();
    let id = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "project",
        "P",
        minimal("why", "P1"),
    )
    .unwrap()
    .id;
    let err =
        write::update(&mut s, &cfg, NOW, ACTOR, &id, &["stage=nope".to_string()]).unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::Validation,
        "a non-member enum value is rejected"
    );
    write::update(&mut s, &cfg, NOW, ACTOR, &id, &["stage=active".to_string()]).unwrap();
    let ops = custom_ops(&s, &id);
    assert!(
        ops.contains(&("stage".into(), "set".into(), "active".into())),
        "a declared enum member is accepted: {ops:?}"
    );
}

#[test]
fn a_date_custom_field_rejects_a_non_iso_value() {
    let mut s = store();
    let cfg = cfg_custom();
    let id = write::create(
        &mut s,
        &cfg,
        "ab12",
        NOW,
        ACTOR,
        "project",
        "P",
        minimal("why", "P1"),
    )
    .unwrap()
    .id;
    let err = write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        &["deadline=not-a-date".to_string()],
    )
    .unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::Validation,
        "a non-ISO date is rejected"
    );
    write::update(
        &mut s,
        &cfg,
        NOW,
        ACTOR,
        &id,
        &["deadline=2026-07-01".to_string()],
    )
    .unwrap();
    let ops = custom_ops(&s, &id);
    assert!(
        ops.contains(&("deadline".into(), "set".into(), "2026-07-01".into())),
        "a valid ISO date is accepted: {ops:?}"
    );
}
