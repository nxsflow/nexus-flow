//! be9y — the App↔CLI parity differential, the seam-invariant acceptance test of the nexus-chat
//! app-facade (the mirror of memory's #3rx.3 `parity.rs` / flow's #9t7.6).
//!
//! Two seams sit over one chat core: the `nxc` CLI (the agent seam, a process per call) and the
//! in-process [`Engine`] handle (the app seam). The seam invariant is "no new semantics — identical
//! derivations, and error/rejection behavior is part of the contract." This pins it three ways:
//!
//!   1. **read parity** — over the SAME store, the embed surface's serialized value is byte-equal to
//!      `nxc <cmd> --json` (channels list, search, inbox);
//!   2. **write parity** — driving the SAME send/reply/read script through both seams against two
//!      workspaces seeded identically yields byte-equal op logs (once the per-op random `op_id` is
//!      dropped — everything load-bearing matches: `(lamport, site)`, target, field, op_type, value,
//!      the pinned **author** and **wall_clock**) plus byte-equal post-write reads;
//!   3. **orchestration parity** (nxf 6j6v.8cs7) — the same, for the ROLE RUNTIME the epic
//!      6j6v.qvfp extraction lifted onto the handle: `workflow start` → `step done` → a channel
//!      step's completion, plus the rejections, with the recorded TRIGGER STREAM compared too. See
//!      the section header above [`orchestration_parity_engine_and_nxc_drive_one_run_identically`].
//!
//! `now` (`NXC_NOW`) and `actor` (`NXC_ACTOR`) are pinned on BOTH sides. `origin` is pinned on
//! NEITHER since nxf 6j6v.07me: the handle answers it from its own workspace and the CLI, with
//! `NXC_ORIGIN` unset, falls back to that same workspace's replica prefix — so the differential
//! hands each seam the same WORKSPACE and lets both derive, rather than telling either the answer.
//! `NXC_ORIGIN` is therefore `env_remove`d in [`nxc`]/[`nxc_orch`] (an outer shell must not leak
//! into the child), and the agreement of the two derivations is a gate of its own,
//! [`both_adapters_resolve_one_origin_for_one_workspace`] — it is the premise every
//! `<origin>/<agent>` compared here rests on. And `NXF_DETERMINISTIC_IDS=1` makes both seams mint
//! the same replica (via `nxc init`) AND the same sequential message/channel ids — so the op-log
//! `site` + `target_id` compare byte-for-byte, and so the two separately-seeded workspaces of the
//! write differential derive the SAME origin. The env is only ever SET to `1` (never unset), so
//! concurrent tests in this binary never race on it.
//!
//! `session` (`NXC_SESSION`) joined this list at T6 (nxf epic 6j6v.zenf): `nxc send` now stamps the
//! ambient caller session into `refs.session_id` when the caller didn't pass an explicit `--ref
//! session_id=…` — a CLI-only env-derived default the in-process [`Engine`]/[`SendRequest`] has no
//! equivalent for (it always takes `refs` explicitly). So it is pinned the same way `origin`/`actor`
//! are: `SESSION` on both sides (`NXC_SESSION` for the CLI, an explicit `Refs.session_id` literal for
//! the `Engine`) in [`write_parity_engine_and_nxc_emit_identical_ops_and_reads`]. `nxc reply` gained
//! the identical ambient-stamping fallback in a later fix round (final review on the same epic) — the
//! `Engine`-side `reply` call below is pinned the same way for the same reason.

//!
//! # What this differential still claims after the consolidation (nxf 6j6v.dvyq)
//!
//! Item 6j6v.33bz rests on this suite for "the facade adds NO semantics the nxc CLI lacks". That
//! claim has two halves, and the consolidation keeps one and gives up the other DELIBERATELY —
//! written here so the next reader does not take the narrowing for an accident:
//!
//! * **KEPT, and the reason this gate exists at all: where the two seams run the SAME verb core,
//!   they must produce the same bytes.** `send --to`, `reply --thread`, `list`, `prime`, the
//!   messaging writes, the run script and the rejections all still drive both sides and compare op
//!   logs and read results byte for byte. This half caught a real defect it was built for
//!   (`channels list parity`, PR #277), and it caught two more the day 6j6v.dvyq landed: a
//!   `prime` on the handle reporting a declaration count of `0` for a workspace that declared four,
//!   and a `list --json` that carried the declaration source on the CLI and not on the seam.
//!   Silencing this gate would have let both through.
//!
//! * **GIVEN UP: that the two surfaces carry the same SET of verbs.** Once a verb deliberately
//!   lives on only one side — the agent surface loses entrances an app keeps (`inbox`), and the
//!   app seam loses writes an agent never had — a differential over the whole verb list checks a
//!   congruence the design has abandoned. That question moved to a gate of its own rather than
//!   being dropped: `verb_seam.rs` compares the two verb LISTS in one direction, and
//!   `seam_disposition.rs` records, for every verb taken off the CLI, whether its read/write stays
//!   on `Engine` — and fails if one that must stay disappears.
//!
//! One consequence is visible in the code below: a record that carries a workspace-ABSOLUTE path
//! (the declaration source, since 6j6v.dvyq) cannot be byte-equal across two separately-seeded
//! workspaces, so those comparisons substitute each side's own root — see
//! [`without_workspace_root`], which is a normalization, not a relaxation.

use assert_cmd::Command;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::facade::{self, StatusScope};
use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs};
use nexus_chat::orchestration::Caller;
use nexus_chat::surface::{ReplyThreadRequest, SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};
use nxs_foundation::model::Op;
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

const NOW: &str = "2026-07-10T10:00:00Z";
const ACTOR: &str = "a";
/// The ambient caller session (T6's `NXC_SESSION`/`session()`), pinned on both seams.
const SESSION: &str = "s-parity";

/// The minting workspace identity BOTH seams resolve, and since nxf 6j6v.07me NEITHER of them is
/// told (owner, 2026-08-21): the handle answers from its own workspace ([`Engine::origin`]), and
/// the CLI falls back to that same workspace's replica prefix when `NXC_ORIGIN` is unset — which is
/// why [`nxc`] REMOVES that variable instead of pinning it. A pin on one side would have made this
/// differential pass while the two derivations silently drifted apart, and `<origin>/<agent>` is
/// WRITTEN into every value compared below (senders, `expects_reply_from`, the default consumer),
/// never re-derived per read. [`both_adapters_resolve_one_origin_for_one_workspace`] is the gate
/// for that agreement; everything else here depends on it holding.
///
/// **Read back from the workspace, not spelled out.** `NXF_DETERMINISTIC_IDS` gives two separately
/// `nxc init`-ed workspaces the SAME replica (`ab12`), which is what keeps the write differential
/// comparable — but writing that literal here would pin the harness's determinism switch instead of
/// the property, and would go stale the day the switch picks another prefix.
fn origin(dir: &Path) -> String {
    Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .replica
        .prefix
}

/// The caller's qualified handle in `dir`'s workspace: `<origin>/<actor>`, exactly what the CLI's
/// `caller_handle()` defaults `--consumer` to.
fn handle(dir: &Path) -> String {
    format!("{}/{}", origin(dir), ACTOR)
}

/// Enable deterministic id minting for THIS test process too, so the in-process [`Engine`] mints the
/// same sequential message ids as the `nxc` subprocesses. Only ever set to `1` (never removed), so
/// parallel tests in this binary see a stable value.
fn deterministic_ids() {
    std::env::set_var("NXF_DETERMINISTIC_IDS", "1");
}

/// The library seam's half of the pinning [`nxc`] does for the CLI side: who is calling on this
/// call, with the clock fixed to exactly what that side's `NXC_NOW` resolves to, so this stays a
/// differential of two SEAMS rather than of two ambient environments. `actor` and `session` vary
/// per call and stay at the call site.
///
/// **Two values that used to be pinned here are not passed any more** (nxf 6j6v.07me): `origin`,
/// which the handle answers for itself (see [`origin`]), and `hop`, which the seam no longer claims
/// at all — the depth comes from the session map, and the one case that used to pin an over-cap
/// claim on both sides says below what became of it.
fn caller<'a>(actor: &'a str, session: Option<&'a str>) -> Caller<'a> {
    Caller {
        session,
        actor: Some(actor),
        now: Some(NOW),
    }
}

/// An `nxc` invocation with `now`/`actor`/ids pinned — the deterministic agent seam.
///
/// **`NXC_ORIGIN` is REMOVED rather than pinned** (nxf 6j6v.07me). The CLI derives its origin from
/// the workspace now, exactly as the handle does, so a pin would silence the one difference this
/// differential is here to catch — and an outer shell that happened to export the variable would
/// otherwise leak into the child. See [`origin`].
fn nxc(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(dir)
        .env_remove("NXC_ORIGIN")
        .env("NXC_ACTOR", ACTOR)
        .env("NXC_NOW", NOW)
        .env("NXC_SESSION", SESSION)
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

/// [`nxc`] with the DRY worker and timer, for a script that TRIGGERS a persona.
///
/// The messaging differential never needed one until 6j6v.dvyq §3 moved its write script onto
/// `send --to <persona>` / `reply --thread`: a plain post summoned nobody, so the CLI side could
/// run with whatever worker the environment happened to have. It cannot now — the app side of the
/// same script runs `WorkerConfig::Dry` explicitly, so the CLI side has to, or the two seams are
/// not handed the same runtime at all. CI is where that shows, and did: a machine WITHOUT `claude`
/// on its PATH makes the real sidecar fail the trigger and `send --to` exit non-zero, while a
/// developer machine with one sails through. The orchestration half below has always pinned this
/// (`orch_nxc`); this is the same pin for the messaging half.
fn nxc_dry(dir: &Path, log: &Path) -> Command {
    let mut c = nxc(dir);
    c.env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", log)
        .env("NXC_TIMER", "dry");
    c
}

/// [`nxc_json`] driven by [`nxc_dry`].
fn nxc_json_dry(dir: &Path, log: &Path, args: &[&str]) -> String {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = nxc_dry(dir, log)
        .args(full)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("utf8").trim_end().to_string()
}

/// Initialize a chat workspace via the external `nxc` (so its replica is the pinned deterministic
/// site 1, adopted by the in-process `Engine` on open).
fn init(dir: &Path) {
    nxc(dir).arg("init").assert().success();
}

/// Run `nxc --json <args>` to success and return its trimmed stdout (the exact bytes an agent parses).
fn nxc_json(dir: &Path, args: &[&str]) -> String {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = nxc(dir)
        // The dry runtime, on the READ side too (nxf 6j6v.qmy6): `status` reports what became of a
        // thread's session, so an unpinned worker would make this differential a statement about
        // what happens to be installed on the machine running it. The engine half of every read
        // below is pinned to the same one. No read starts anything, so this changes nothing else.
        .env("NXC_WORKER", "dry")
        .args(full)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("utf8").trim_end().to_string()
}

/// Serialize a value the SAME way the CLI does for `--json` (declaration-order `to_string`).
fn json_str<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).expect("serialize")
}

/// Replace a workspace's own absolute root with `[WS]`, so two seams driven against two SEPARATE
/// workspaces can still be compared byte for byte (nxf 6j6v.dvyq).
///
/// **This is a normalization, not a relaxation, and the difference is the whole point.** Since
/// 6j6v.dvyq every directory record carries the resolved declaration path, which is
/// workspace-absolute by construction — the same way trycmd's own goldens carry `[CWD]`. Two
/// workspaces cannot agree on it and are not supposed to; what they must agree on is everything the
/// path is made OF: the same folder name, the same source kind, the same count, the same relative
/// position under the root. Substituting the root and comparing the rest keeps all of that under
/// the differential. Blanking the whole `declarations` object instead would give up exactly the
/// field this item added.
///
/// BOTH spellings of the root are substituted, because the two seams genuinely reach the workspace
/// by different routes: the `nxc` subprocess resolves it through the process cwd (`/var` →
/// `/private/var` on macOS) while the in-process handle is handed the `TempDir` path verbatim. The
/// resolution deliberately reports the caller's OWN spelling rather than a canonicalized one — the
/// path is shown to a human, and one they did not type is worse than one that varies — so
/// normalizing that away belongs here, in the differential, not in the library.
fn without_workspace_root(json: &str, root: &Path) -> String {
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    json.replace(&canonical.display().to_string(), "[WS]")
        .replace(&root.display().to_string(), "[WS]")
}

/// Pull a string field out of a one-object `--json` reply.
fn field(json: &str, key: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json).unwrap()[key]
        .as_str()
        .expect("string field")
        .to_string()
}

/// The `general` group channel with `o/a` in it, and its minted id.
///
/// **Written to the store since 6j6v.dvyq §3 removed `nxc channels create`** — a channel is a
/// declaration now, and nothing mints one from the command line. The ops are exactly the ones that
/// verb emitted (the three channel fields plus the creator's membership add) with the same author
/// and the same pinned wall clock, which is what keeps two separately-seeded workspaces byte-equal
/// for the write differential below.
fn seed_channel(dir: &Path) -> String {
    let mut store = Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    store.set_wall_clock(NOW);
    let (origin, handle) = (origin(dir), handle(dir));
    let cid = store.mint_channel_id();
    store.set_channel_field(&cid, "name", "general", &handle);
    store.set_channel_field(&cid, "kind", "group", &handle);
    store.set_channel_field(&cid, "origin", &origin, &handle);
    store.add_member(&cid, &handle, &handle);
    cid
}

/// Post `body` into `channel` as `actor` — the fixture `nxc send <channel> <body>` was, before the
/// raw channels went (6j6v.dvyq §3). The tests that use it are about a READ; this is what has to
/// have happened first.
fn seed_post(dir: &Path, channel: &str, actor: &str, body: &str, disposition: Disposition) {
    let mut store = Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    store.set_wall_clock(NOW);
    let origin = origin(dir);
    store.post_message(&MessageEnvelope {
        origin: origin.clone(),
        channel_id: channel.into(),
        sender: format!("{origin}/{actor}"),
        kind: MessageKind::Info,
        priority: Priority::Normal,
        disposition,
        thread_id: None,
        refs: Refs::default(),
        body: body.into(),
    });
}

/// Open a quorum board expecting `expect` and return its thread id — the fixture
/// `nxc ask <channel> <body> --expect <h>` was.
///
/// `facade::ask` is the write that verb delegated to and is untouched; only the entrance went
/// (6j6v.dvyq §3).
fn seed_board(dir: &Path, channel: &str, body: &str, expect: &[&str]) -> String {
    let mut store = Workspace::resolve(None, dir)
        .expect("resolve workspace")
        .open_chat_store()
        .expect("open chat store");
    let expect: Vec<String> = expect.iter().map(|h| (*h).to_string()).collect();
    let origin = origin(dir);
    nexus_chat::facade::ask(
        &mut store,
        nexus_chat::facade::AskRequest {
            now: NOW,
            origin: &origin,
            actor: ACTOR,
            channel,
            body,
            expect: &expect,
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
    )
    .expect("the board opens")
    .thread_id
}

/// The op log as comparable rows with the per-op random `op_id` dropped — everything else is what
/// parity is about. Ordered as `export` returns them: by `(lamport, site)`.
type OpRow = (
    i64,            // lamport
    i64,            // site
    String,         // domain
    String,         // target_kind
    String,         // target_id
    String,         // field
    String,         // op_type
    Option<String>, // value
    String,         // author
    String,         // wall_clock
);

fn op_rows(dir: &Path) -> Vec<OpRow> {
    let store = Workspace::resolve(None, dir)
        .unwrap()
        .open_chat_store()
        .unwrap();
    store
        .export()
        .into_iter()
        .map(|o: Op| {
            (
                o.lamport,
                o.site,
                o.domain,
                o.target_kind,
                o.target_id,
                o.field,
                o.op_type,
                o.value,
                o.author,
                o.wall_clock,
            )
        })
        .collect()
}

// ---- the origin seam (nxf 6j6v.07me) ----------------------------------------------------------

#[test]
fn both_adapters_resolve_one_origin_for_one_workspace() {
    // THE property this whole differential rests on, and the only one nothing else holds.
    //
    // `origin` is not compared field-by-field anywhere below — it is DISSOLVED into every value
    // that is: a message `sender`, a channel's `origin` field, `expects_reply_from`, the default
    // `--consumer`. All of those are `<origin>/<agent>`, WRITTEN once and matched later by exact
    // string, never re-derived per read. So if the two adapters answered differently, the failure
    // would not read as "the origins disagree": an app's `send --to coder` and a terminal's would
    // simply address two different members of one declared channel, and nothing would report it.
    //
    // Since nxf 6j6v.07me neither adapter is TOLD the value any more — the handle answers from its
    // own workspace (`Engine::origin`) and the CLI falls back to the same workspace's replica
    // prefix when `NXC_ORIGIN` is unset. Two derivations from one source is exactly the shape that
    // can silently split, which is why it is pinned here rather than assumed.
    deterministic_ids();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init(dir);

    let engine = Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .unwrap();

    // The CLI's answer, observed the only way a black-box invocation can show it: a verb that
    // defaults `--consumer` echoes the handle it defaulted TO, and that default IS `caller_handle()`
    // = `<origin>/<actor>`. That verb was `inbox` until nxf 6j6v.1gm9 removed it; `prime` is the
    // survivor that resolves the consumer through the very same [`resolve_consumer`] and prints it.
    // The `env_remove` is restated here even though [`nxc`] already does it for every command in
    // this file: this is the ONE case whose subject is the unset default, so it must not depend on
    // a helper further up keeping a habit. `env_remove`, not merely "unset", because an outer shell
    // that exports `NXC_ORIGIN` would otherwise be inherited and answer the question for us.
    let mut cmd = nxc(dir);
    cmd.env_remove("NXC_ORIGIN");
    let out = String::from_utf8(
        cmd.args(["--json", "prime"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .expect("prime --json is utf-8");
    let consumer = field(&out, "consumer");
    let (cli_origin, cli_actor) = consumer
        .split_once('/')
        .expect("the default consumer is a qualified handle");

    assert_eq!(
        cli_origin,
        engine.origin(),
        "the CLI's default origin and the handle's answer must be the SAME value for one workspace"
    );
    assert_eq!(
        cli_actor, ACTOR,
        "sanity: the actor half is still the pinned one"
    );
    assert_eq!(
        engine.origin(),
        origin(dir),
        "and that value is the workspace's own replica prefix, not a constant"
    );
}

#[test]
fn read_parity_embed_surface_equals_nxc_json_over_the_same_store() {
    // One store, seeded by `nxc`; both seams READ it. The embed surface's serialized values must be
    // byte-equal to `nxc <cmd> --json`.
    deterministic_ids();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init(dir);
    let ch = seed_channel(dir);
    seed_post(dir, &ch, ACTOR, "hello world", Disposition::InTurn);
    seed_post(dir, &ch, ACTOR, "later note", Disposition::NextSession);
    // Two more from SOMEBODY ELSE, so the unread differential below is not two empty lists agreeing
    // about nothing: hj12 (spec §4.5) keeps a caller's own posts out of its own unread, and the two
    // above are the caller's. A non-member sender is deliberate and correct — unread is scoped by
    // the READER's channels, not the writer's.
    seed_post(dir, &ch, "bob", "act on this now", Disposition::InTurn);
    seed_post(
        dir,
        &ch,
        "bob",
        "and this one later",
        Disposition::NextSession,
    );

    let engine = Engine::open(None, dir).unwrap();
    let handle = handle(dir);

    // The `channels list` and `channels public` cases stood here. Both verbs went with 6j6v.dvyq
    // §3, so neither read has a CLI half left to be compared against — and a differential over one
    // surface is not a differential. Their coverage MOVED rather than went: `tests/seam_reads.rs`
    // holds the membership scoping and the degraded-DM flag, `tests/public_channel.rs` holds the
    // discovery read. That is the owner's decision of 2026-08-19 in code — the differential keeps
    // its core, and an orphaned read gets a gate of its own instead of quietly losing one.

    // `search` parity — the same `MessageHit` shape, driven from the HANDLE.
    //
    // It briefly ran off `facade::search` instead: nxf 6j6v.yr59 removed `Engine::search`, and a
    // read with no handle method has no second seam to be compared against, so the case fell back
    // to the compute layer rather than being deleted. THE OWNER PUT THE METHOD BACK on 2026-08-21
    // (app-foundations consumes it; `tests/seam_disposition.rs`'s `search` row carries the whole
    // argument), so the case is driven from `Engine` again — which is what this file is for. A
    // differential over one surface is not a differential, and while `search` HAS two surfaces the
    // gate should be reading both.
    assert_eq!(
        json_str(&engine.search(&handle, "hello").unwrap()),
        nxc_json(dir, &["search", "hello"]),
        "search parity"
    );

    // **The unread differential stood here and has no two surfaces left to compare.** It projected
    // and partitioned `facade::inbox` on one side and read `prime --json`'s `in_turn`/`next_session`
    // /`count` on the other — the case that survived `nxc inbox`'s removal (6j6v.1gm9) because
    // `prime` filled those fields from that very call, so the comparison still crossed a process
    // boundary. nxf 6j6v.4d2z removed the derivation and the fields together; a differential over
    // nothing is not a differential.
    //
    // Nothing moved to a gate of its own, and that is the honest bookkeeping: what `seam_reads.rs`
    // exists for is a read whose VERB went while the read stayed. Here the read went.
}

#[test]
fn read_parity_quorum_surface_equals_nxc_json_over_the_same_store() {
    // T4 seam invariant (§8): the app-facade's M2 quorum reads serialize byte-equal to `nxc … --json`
    // over the SAME store — no new semantics. One board (opened through `facade::ask`, the write
    // `nxc ask` delegated to before 6j6v.dvyq §3 removed the verb; completed by a reply)
    // is read through both seams: the bulk board list, one board in full, and the opener wake.
    deterministic_ids();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init(dir);
    let ch = seed_channel(dir);
    // o/a opens a board expecting o/b (the request message is posted by o/a). Through the library
    // since 6j6v.dvyq §3 removed `nxc ask`: what is under test is the READ, and a multi-handle
    // board on a channel nothing declares no longer has a CLI door to be opened by.
    let tid = seed_board(dir, &ch, "please review", &["o/b"]);
    let handle = handle(dir);
    // o/b posts the awaited reply straight into the shared store → the board completes. (A reply is
    // any message from an expected handle; posting via the store keeps the parity about the READ.)
    {
        let mut s = Workspace::resolve(None, dir)
            .unwrap()
            .open_chat_store()
            .unwrap();
        s.set_wall_clock(NOW);
        s.post_message(&MessageEnvelope {
            origin: origin(dir),
            channel_id: ch.clone(),
            sender: "o/b".into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some(tid.clone()),
            refs: Refs::default(),
            body: "lgtm".into(),
        });
    }

    // **The SAME runtime on both sides, pinned** (nxf 6j6v.qmy6). `status` now reports what became
    // of each thread's session and whether the worker could answer that at all, so the two halves
    // of this differential are only comparable when one runtime answered for both: an embedder's
    // default worker starts no process and says so, while a bare `nxc` resolves whatever this
    // machine has installed. That difference is not what this test is about — it is about one READ
    // over one store — so it is taken out rather than asserted, here and on the CLI side below.
    let engine = Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            ..EngineConfig::default()
        },
    )
    .unwrap();
    let store = Workspace::resolve(None, dir)
        .unwrap()
        .open_chat_store()
        .unwrap();

    // THE THREE READS BELOW LOST THEIR `Engine` HALF IN nxf 6j6v.yr59 and kept their verb, so the
    // differential is driven from the compute layer instead. It claims exactly what it claimed:
    // `Engine::threads`/`thread_board`/`opener_wake` were passthroughs to these three functions
    // (`thread_board` plus the `visibility` resolution the CLI does for itself, which is why
    // `AllMembers` is passed here — this board's channel is not a declared one, so that is the
    // value both sides resolve). Deleting the cases instead would have taken the only differential
    // of three LIVE verbs with it.
    //
    // `threads list` parity — Vec<ThreadQuorum>, the CLI's exact `--json`.
    assert_eq!(
        json_str(&facade::threads(&store, &handle, NOW, None).unwrap()),
        nxc_json(dir, &["threads", "list"]),
        "threads list parity"
    );
    // `threads show` parity — the flattened ThreadBoardView.
    assert_eq!(
        json_str(
            &facade::thread_board(
                &store,
                &tid,
                NOW,
                &handle,
                nexus_chat::channel::Visibility::AllMembers
            )
            .unwrap()
        ),
        nxc_json(dir, &["threads", "show", &tid]),
        "threads show parity"
    );
    // `opener_wake` parity — the `threads_you_opened` block `nxc prime --json` embeds.
    //
    // **It embeds NOTHING here, and that is the assertion since nxf 6j6v.2hx9.** The block used to
    // be present because `o/a` had a complete-and-unacked board; it is now derived from a WINDOW —
    // what finished since the caller's previous session ended — and a caller that is not a declared
    // persona has no session whose end could be that watermark. So both sides say nothing, and the
    // parity is that they say it the same way: the derivation is empty, and the key is absent
    // rather than an empty object.
    let prime: serde_json::Value = serde_json::from_str(&nxc_json(dir, &["prime"])).unwrap();
    assert!(
        facade::opener_wake(&store, &handle, NOW, None)
            .unwrap()
            .is_empty(),
        "no watermark, no window"
    );
    assert!(
        prime.get("threads_you_opened").is_none(),
        "and `prime --json` omits the block entirely rather than carrying an empty one: {prime}"
    );
    // `status` parity, EVERY form (nxf 6j6v.a71h §5, and the fourth since nxf 6j6v.1vxs): the
    // operation view an app builds is the same bytes `nxc status --json` prints. The board above is
    // a ROOT operation (opened by `ask` from a bare terminal), so each form has something to say
    // about it — `--all` because it is finished, the other three because they show it either way.
    assert_eq!(
        json_str(&engine.status(NOW, StatusScope::Workspace).unwrap()),
        nxc_json(dir, &["status"]),
        "status parity"
    );
    assert_eq!(
        json_str(&engine.status(NOW, StatusScope::Threads(&[&tid])).unwrap()),
        nxc_json(dir, &["status", "--thread", &tid]),
        "status --thread parity"
    );
    assert_eq!(
        json_str(&engine.status(NOW, StatusScope::Channel(&ch)).unwrap()),
        nxc_json(dir, &["status", "--channel", &ch]),
        "status --channel parity"
    );
    assert_eq!(
        json_str(&engine.status(NOW, StatusScope::All(None)).unwrap()),
        nxc_json(dir, &["status", "--all"]),
        "status --all parity"
    );
    assert_eq!(
        json_str(&engine.status(NOW, StatusScope::All(Some(&ch))).unwrap()),
        nxc_json(dir, &["status", "--all", "--channel", &ch]),
        "status --all --channel parity"
    );
}

#[test]
fn read_parity_prime_report_equals_nxc_prime_on_both_views() {
    // nxf 6j6v.r5a2. `prime` used to exist on ONE seam only — a private function in `cli.rs` that
    // reached PAST this facade straight into the store (`store.inbox`, `store.opener_wake`) — and
    // this differential was blind to it by construction: it compares verbs that exist on both sides,
    // so an omission (as opposed to a divergence) slipped straight through. Now the handle serves
    // the same report, and BOTH of its views are pinned against the CLI: the `--json` projection and
    // the rendered human Markdown the SessionStart hook actually ships.
    deterministic_ids();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init(dir);
    let ch = seed_channel(dir);
    // o/b posts into the shared channel, so o/a's catch-up is non-empty on both dispositions, and
    // o/a opens a board so the requester wake has something to say.
    seed_post(dir, &ch, "b", "please review PR 42", Disposition::InTurn);
    seed_post(dir, &ch, "b", "later", Disposition::NextSession);
    seed_board(dir, &ch, "sign off", &["o/b"]);
    // A declared team with one clean role and one broken CHANNEL reference, so the roster AND the
    // declaration errors are both non-empty — the branch the interactive half below compares. It
    // was a clean and a broken WORKFLOW until 6j6v.dvyq §3; the channel partition is the same
    // shape, and it is the one that is left.
    let roles = dir.join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: be helpful\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: standup\n  members: [pm]\n- name: ghosts\n  members: [ghost]\n",
    )
    .unwrap();

    let engine = Engine::open(None, dir).unwrap();
    let handle = handle(dir);
    // `prime_as` with no persona IS `prime`, which is why the latter went (nxf 6j6v.yr59).
    let report = engine.prime_as(&handle, None, NOW).unwrap();

    // The CLI subprocess inherits `NXC_ACTOR` from `nxc()`, so it renders as a SPAWNED context —
    // declaration errors suppressed. The handle is told the same thing explicitly, which is the
    // point: the decision is the caller's on BOTH seams, not something the facade quietly makes.
    assert_eq!(
        without_workspace_root(&report.to_value(false).to_string(), dir),
        without_workspace_root(&nxc_json(dir, &["prime"]), dir),
        "prime --json parity: one report, one projection"
    );
    // The human form is the SessionStart-hook contract, so it is the half that must be byte-equal:
    // `nxc prime` prints exactly `render_markdown()` plus the trailing newline `println!` adds.
    let human = nxc(dir)
        .arg("prime")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        format!("{}\n", report.render_markdown(false)),
        String::from_utf8(human).expect("utf8"),
        "prime human parity: the hook block an embedder renders IS the one the CLI prints"
    );
    // ---- the INTERACTIVE branch (independent review, Test Quality #1) ----------------------
    //
    // The two halves above compare `declaration_errors = false`. That is the cheaper branch, and on
    // its own it would leave the roster + declaration-errors sections of the `--json` view — the
    // very part whose absence from the library forced app-foundations to hand-rebuild the assembly
    // — unpinned across the seams. **The HUMAN view is narrower than that description since nxf
    // h4d3 (task 3, review correction):** "## Declared Team" no longer renders at all, in either
    // branch, so the interactive assertion below proves Declaration Errors parity alone there, not
    // a roster the human view has stopped showing. So drive the CLI as a human at the keyboard (all
    // three spawned-context signals unset, `--consumer` pinning the handle that `NXC_ACTOR` would
    // otherwise have supplied) and compare the SAME report rendered with the flag on.
    let interactive = |args: &[&str]| {
        let mut c = nxc(dir);
        c.env_remove("NXC_ACTOR")
            .env_remove("NXC_WORKER")
            .env_remove("NXC_SESSION");
        let out = c.args(args).assert().success().get_output().stdout.clone();
        String::from_utf8(out).expect("utf8")
    };
    assert_eq!(
        without_workspace_root(&format!("{}\n", report.to_value(true)), dir),
        without_workspace_root(
            &interactive(&["--json", "prime", "--consumer", &handle]),
            dir
        ),
        "prime --json parity, interactive: the roster and its errors project identically"
    );
    assert_eq!(
        format!("{}\n", report.render_markdown(true)),
        interactive(&["prime", "--consumer", &handle]),
        "prime human parity, interactive: the address book + Declaration Errors render identically"
    );
    // The flag is the ONLY difference between the two renderings — and for the human view (see
    // above) what it actually gates is Declaration Errors alone, since "## Declared Team" never
    // renders either way now. So the assertion below would be vacuous with an EMPTY roster.errors,
    // not an empty roster.roles: assert both are non-empty (a vacuous pass is the exact failure
    // mode the finding described), and keep `roster.roles` asserted too because the `--json` half
    // above still projects it.
    assert_eq!(report.roster.roles, ["pm"], "declared role offered");
    assert!(
        !report.roster.errors.is_empty(),
        "the broken channel produced a declaration error to gate on"
    );
    // …and the same gate now carries the QUALITY warnings and the writing-declarations pointer
    // (nxf 6j6v.9w08). This fixture trips them without being changed for it — `pm.yaml` is the
    // two-line minimum and neither channel declares a `description` — so the interactive parity
    // assertions above are not vacuous about them either.
    assert!(
        !report.roster.warnings.is_empty(),
        "the fixture's declarations produce quality warnings to gate on"
    );
    assert_ne!(
        report.render_markdown(true),
        report.render_markdown(false),
        "the interactive rendering genuinely differs — otherwise the pair above proves nothing"
    );

    // And the parts are the shared reads, not second ones of their own — which is exactly why
    // `Engine::opener_wake` and `Engine::inbox` could leave the handle in nxf 6j6v.yr59: this
    // assertion is the evidence that `prime_as` already carries both, so an app calling it needs
    // neither. Read from the compute layer now, since the seam no longer offers them.
    let store = Workspace::resolve(None, dir)
        .unwrap()
        .open_chat_store()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&report.wake).unwrap(),
        serde_json::to_value(facade::opener_wake(&store, &handle, NOW, None).unwrap()).unwrap(),
        "the report's wake IS `opener_wake`, not a second derivation"
    );
    assert!(
        report.wake.is_empty(),
        "…and for a caller that is not a declared persona it is empty, because there is no \
         previous session end to derive a window from (nxf 6j6v.2hx9)"
    );
    // A second assertion of the same shape stood beside it — that the report's catch-up IS
    // `facade::inbox` rather than a second read. Both halves of it went with the unread apparatus
    // (nxf 6j6v.4d2z), so the wake is the one shared derivation this record is assembled from, and
    // the claim above is now the whole of it.
}

#[test]
fn write_parity_engine_and_nxc_emit_identical_ops_and_reads() {
    // Two workspaces seeded IDENTICALLY (the same declared persona), then the same
    // `send --to` → `reply --thread` → `read` script through the two seams.
    //
    // **The script moved with the surface** (nxf 6j6v.dvyq §3). It used to be a plain
    // `send <channel> <body>` followed by a reply to that MESSAGE id, and neither is a thing an
    // agent can type any more: a channel no declaration names is not a target, and a conversation
    // has one address. So the differential drives the pair that DID survive, which is also the pair
    // §4 names as the whole agent write surface — and that is the half this gate was always for.
    deterministic_ids();
    let app = TempDir::new().unwrap();
    let cli = TempDir::new().unwrap();
    init(app.path());
    init(cli.path());
    let handle = handle(app.path());
    for dir in [app.path(), cli.path()] {
        let personas = dir.join(".nxs-personas");
        std::fs::create_dir_all(&personas).unwrap();
        std::fs::write(
            personas.join("pm.yaml"),
            "handle: pm\njob_title: Product manager\nsystem_prompt: You are the PM.\n",
        )
        .unwrap();
    }

    // App seam: send --to → reply --thread → read, all in one long-lived Engine. The worker is the
    // DRY one on both sides, so a trigger is recorded and never actually spawns.
    let (ch_app, thread_app, mid_app) = {
        let e = Engine::open_with(
            None,
            app.path(),
            EngineConfig {
                worker: WorkerConfig::Dry { log: None },
                timer: TimerConfig::Disabled,
                ..EngineConfig::default()
            },
        )
        .unwrap();
        let r = e
            .send_to(
                caller(ACTOR, Some(SESSION)),
                SendToRequest {
                    machine: None,
                    to: "pm",
                    body: "hello world",
                    // The ambient session used to be passed HERE as an explicit `refs.session_id`,
                    // mirroring the CLI's `--ref session_id=…`. Since nxf 6j6v.ckeq the seam takes
                    // only what a first message is ABOUT, and the return address is stamped by
                    // `orchestration` from the caller's own `Caller::session` on both seams — which
                    // is what this differential now proves rather than assumes.
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .unwrap();
        // Since nxf 6j6v.d49b the handle's reply IS the orchestration verb (requester wake +
        // declared-channel completion routing), so it takes the ambient values the CLI reads from
        // the environment as an explicit `Caller` — pinned by `caller()` above to exactly what the
        // `nxc` side's `NXC_NOW`/`NXC_ACTOR`/`NXC_SESSION` resolve to. (`NXC_ORIGIN` dropped off
        // that list with nxf 6j6v.07me: neither seam is told the origin any more, both read it off
        // the workspace.) Since nxf 6j6v.ckeq there is one reply on the seam and this is it.
        e.reply_thread(
            caller(ACTOR, Some(SESSION)),
            ReplyThreadRequest {
                machine: None,
                thread: &r.thread_id,
                body: "on it",
                escalate: false,
                needs_rework: false,
                accept: false,
            },
        )
        .unwrap();
        (r.channel, r.thread_id, r.message_id)
    };

    // CLI seam: the identical script via `nxc` subprocesses, on the SAME dry worker the app side
    // above was opened with.
    let dry = cli.path().join("dry.log");
    let send_out = nxc_json_dry(
        cli.path(),
        &dry,
        &[
            "send",
            "--to",
            "pm",
            "hello world",
            // `--kind`/`--priority` stood here and went with 6j6v.ckeq's decision 3. Nothing is
            // lost from the differential: the point is that both seams write the SAME envelope, and
            // with neither side able to choose, both write the defaults.
            "--no-ref",
        ],
    );
    let (ch_cli, thread_cli, mid) = (
        field(&send_out, "channel"),
        field(&send_out, "thread_id"),
        field(&send_out, "message_id"),
    );
    assert_eq!(ch_app, ch_cli, "deterministic channel id on both sides");
    assert_eq!(thread_app, thread_cli, "and the same thread id");
    assert_eq!(mid_app, mid, "and the same message id");
    nxc_dry(cli.path(), &dry)
        .args(["reply", "--thread", &thread_cli, "on it"])
        .assert()
        .success();
    // A third step stood here — `Engine::mark_read` against `nxc read <channel> --through <id>` —
    // and it went when nxf 6j6v.1gm9 removed the verb. **The ACK itself is untouched**
    // (`Engine::mark_read`, `Standing` nowhere and `Stays` in `seam_disposition.rs`); what is gone
    // is the second adapter this file exists to differentiate against, so there is no differential
    // left to run. Named as the loss it is rather than quietly dropped: the write's own coverage is
    // `embed.rs`'s `mark_read` case and `public_channel.rs`'s membership refusal, both seam-side.

    // (1) Identical ops: every load-bearing field matches once the random op_id is dropped —
    // crucially the explicit `wall_clock` (now) and `author` (sender/consumer) on every op.
    let app_ops = op_rows(app.path());
    let cli_ops = op_rows(cli.path());
    assert_eq!(app_ops, cli_ops, "the two seams emit identical op logs");
    assert!(
        app_ops.iter().all(|r| r.1 == 1),
        "every op is on the pinned deterministic replica (site 1)"
    );
    // The two script ops (message post, reply post) carry now + the acting handle. It was three
    // until 6j6v.1gm9 removed `nxc read` and with it the `read_cursor` op this script emitted, and
    // 6j6v.4d2z has since removed the op KIND itself — the reducer no longer claims it, so one
    // arriving from an older peer is stored and never folded. `read_cursor` stays in the FILTER
    // for exactly that reason: it is now the assertion that neither seam EMITS one, and a
    // reappearing cursor op would move the count and be seen rather than silently pass.
    let script: Vec<&OpRow> = app_ops
        .iter()
        .filter(|r| r.3 == "message" || r.3 == "read_cursor")
        .collect();
    assert_eq!(script.len(), 2, "send + reply");
    assert!(
        script.iter().all(|r| r.9 == NOW && r.8 == handle),
        "every script op carries the pinned now + the o/a author"
    );

    // (2) A post-write READ differential stood here — the app workspace's unread against the
    // catch-up `nxc prime --json` printed, byte for byte, including the hj12 case where the caller
    // authored both messages and so has an empty inbox on both sides. It went with the unread
    // apparatus (nxf 6j6v.4d2z), like its twin in the read-parity test above. What (1) proves is
    // untouched and is the load-bearing half here: the two seams emit IDENTICAL OPS, so any read
    // over the result reads the same thing on both sides by construction.
}

// =================================================================================================
// The ORCHESTRATION differential (nxf 6j6v.8cs7) — the role runtime, both seams
// =================================================================================================
//
// Everything above pins the messaging half. Epic 6j6v.qvfp extracted the orchestration verbs out of
// `cli.rs` into `crate::orchestration`, where `cli.rs` fills a `Ctx` from `NXC_*` + the filesystem
// and `Engine` fills one from explicit arguments + injected `Definitions`. One implementation, two
// adapters — so "no new semantics" ought to hold BY CONSTRUCTION. That is an argument, not
// evidence; this section is the evidence, in exactly the shape `write_parity` already has.
//
// WHAT IS HELD CONSTANT. Two workspaces, `nxc init`-seeded (so both replicas are the pinned
// deterministic site 1) and given BYTE-IDENTICAL `.nxs-personas/` folders. `now` (`NXC_NOW` / the
// explicit argument), `origin`, `actor`, the caller `session`, the depth-guard `hop` and
// `NXF_DETERMINISTIC_IDS=1` are pinned on both sides — every one of them is an argument on the
// handle and an environment read on the CLI, which is the whole difference between the seams and
// therefore the one thing that must NOT be allowed to vary.
//
// WHAT IS COMPARED. (1) the op logs, byte for byte, minus the per-op random `op_id`; (2) the
// receipts and the reads after the writes — `workflow start`/`step done`, `workflow status`,
// `threads list`, `threads show`; (3) the recorded trigger stream (role, session, resume, message),
// which is why both sides run on the DRY worker: no node process, and the spawn decisions become a
// comparable artifact instead of a side effect; (4) the REJECTIONS, in the sibling test below.
//
// WHY `FromRolesDir` ON BOTH SIDES, when an app would inject `Definitions::Supplied`: holding the
// catalogue itself constant is what makes a failure here mean "the seams differ". Supplying one
// side from a folder and the other from a `Vec` would make every red run ambiguous between that and
// "the two catalogues differ". The supplied-definitions path is `tests/embed_orchestration.rs`'s
// subject; `Definitions` is a single validated value either way, which is the point of it existing.
//
// WHAT IT FOUND, first run: the `workflow status`/`step done` reads were NOT byte-equal. Both seams
// derived identical VALUES, but `cli.rs` rebuilt them into a `serde_json::json!` object — a
// `BTreeMap`, since `preserve_order` is not enabled here — and so emitted them under alphabetically
// sorted keys, while the library seam serialized `WorkflowRunView`/`WorkflowAdvanceReceipt` in
// declared order. Those types' own docs assert "declared field order = the JSON contract", which was
// therefore true of one seam only. Fixed at the root rather than asserted around: `workflow
// start`/`step done`/`status`/`tick` now SERIALIZE the shared type, as `print_ask_receipt` already
// did — one definition of the shape, both seams rendering it.
//
// WHAT IS DELIBERATELY NOT EQUAL, and why none of it is a defect:
//
//   · the workspace `db_path`. Two workspaces means two paths; it reaches only a spawned session's
//     `NXC_DB` stamp (`orchestration::trigger_env`), which is not part of what the dry worker
//     records, and there is no third answer a differential could ask for.
//   · `nxc reply --json`'s shape. It renders `{persisted, posted, thread_id, resumed, warnings}` on
//     EVERY reply — `warnings` included, empty or not (nxf 6j6v.93zd: a field that appears only on
//     failure is one a reader can forget to look for) — plus `message_id` only when one was actually
//     written (nxf 6j6v.ww0a made it `Option<String>` on both seams, because `--if-unanswered` is
//     what first made a successful reply that wrote nothing possible), plus
//     `wake_skipped`/`advance_failed` when there is one.
//     The handle returns `ReplyReceipt`, which also carries `woke`/`completed` — see the assertion
//     that compares the shared fields for why that pair is an adapter's answer to a question the
//     CLI never had to ask, not a semantic difference. `posted` is NOT in the carve-out and never
//     was: 6j6v.ww0a put it on `ReplyReceipt` and on the CLI's object in the same ticket, so it is
//     a shared field and the assertions below compare it like any other. Neither is `persisted`,
//     which is the CLI's older name for the same boolean and is asserted against it here. The two
//     FINDINGS are deliberately NOT in that carve-out either (nxf 6j6v.bxdd, and the PR #265 review
//     for its sibling): "the hand-off did not happen" and "the run did not move on" are what a
//     caller of either seam must be able to see, so both render them, and the assertions below pin
//     that both OMIT them when nothing failed.
//
// The depth-guard message used to be a THIRD entry here: it named `NXC_HOP`, a variable a library
// caller has no equivalent of, oddly but identically on both seams. nxf 6j6v.m48m removed the reason
// for it — the counter is no longer primarily that variable on either seam, since the authoritative
// depth comes from the session map — so the message names the hop itself and the carve-out is gone
// rather than merely justified.

/// The one ambient still left. `NXC_DRY_LOG` used to be the other one — `DryWorker::trigger` read
/// it straight from `std::env::var`, with no injected seam, unlike `WorkerConfig::from_ambient` and
/// `worker::forwarded_real_env`, which were both deliberately built over an injected lookup so
/// parallel tests in one binary cannot race on process-global env. That gap was recorded here
/// rather than papered over, and nxf 6j6v.570x closed it after it produced a real flake: the log
/// path is a field on `WorkerConfig::Dry` now, so the `Engine` side of this differential is TOLD
/// where to record and this guard no longer touches that variable at all.
///
/// `NXC_TIMER` is the remaining one, for the same reason the old note gave: `timer::select_timer`
/// reads process env with no injected seam, and unset it picks the REAL `at`, which on a machine
/// with no `atd` costs eight seconds per armed liveness window before giving up. Serialized against
/// itself and cleaned up on drop.
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard<'a>(#[allow(dead_code)] std::sync::MutexGuard<'a, ()>);

impl Drop for EnvGuard<'_> {
    fn drop(&mut self) {
        std::env::remove_var("NXC_TIMER");
    }
}

/// Pins `NXC_TIMER=dry` for the in-process side (nxf 6j6v.d9cb). `nxc_orch` pins the same value per
/// invocation for the CLI side.
fn lock_env() -> EnvGuard<'static> {
    let guard = EnvGuard(ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner()));
    std::env::set_var("NXC_TIMER", "dry");
    guard
}

/// The kickoff actor (a human at a terminal / an app's user), with no ambient session of its own.
const KICKOFF: &str = "alice";
/// The declared review channel's one member — a BARE handle, because `ensure_declared_channel` joins
/// a channel's declared `members:` verbatim while `expects_reply_from` carries the QUALIFIED form.
/// Membership-scoped reads therefore take `bob`, and the quorum's expects read `o/bob`.
const MEMBER: &str = "bob";
const WORK_ORDER: &str = "ship the whole thing";
const VERDICT: &str = "looks good";
/// The internal session the depth-guard differential seeds past the cap in BOTH workspaces. Its id
/// has to be pinned, not minted: `session_map` is device-local (never op-folded), so the two seams
/// can only be compared over a row each workspace was given deliberately.
const DEEP_SESSION: &str = "m-deep";

/// The declarations BOTH seams resolve through: two roles and one `pass_through` review channel.
/// It also declared a two-step WORKFLOW until 6j6v.dvyq §3, which is what the removed run
/// differential above was driven over.
fn seed_declarations(dir: &Path) {
    let roles = dir.join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    for handle in ["coder", MEMBER] {
        std::fs::write(
            roles.join(format!("{handle}.yaml")),
            format!("handle: {handle}\nsystem_prompt: You are {handle}.\n"),
        )
        .unwrap();
    }
    std::fs::write(
        roles.join("channels.yaml"),
        format!("- name: review\n  members: [{MEMBER}]\n  on_complete: pass_through\n"),
    )
    .unwrap();
}

/// The app seam's handle: the same `.nxs-personas/` folder the CLI reads, and the same DRY worker
/// recording into `log`.
fn orch_engine(dir: &Path, log: &Path) -> Engine {
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            // Where the dry worker records is TOLD, exactly like the timer below — the CLI side
            // gets the same file through `NXC_DRY_LOG` per invocation (see `nxc_orch`), and since
            // nxf 6j6v.570x the in-process side no longer has to reach for that variable at all.
            worker: WorkerConfig::Dry {
                log: Some(log.to_path_buf()),
            },
            // The CLI side resolves `NXC_TIMER=dry` from its own env (see `nxc_orch`); the handle is
            // TOLD, which is the whole point of the seam being a value now (PR #269 review, Code
            // Quality #3). Same timer either way, so the differential still compares like with like.
            timer: nexus_chat::timer::TimerConfig::Dry,
            ..EngineConfig::default()
        },
    )
    .expect("open the orchestration handle")
}

/// An `nxc` invocation for the orchestration script: every value the handle takes as an ARGUMENT,
/// pinned into the environment the CLI reads it from.
///
/// `session: None` is pinned as the EMPTY string rather than left unset. `cli.rs::session()` filters
/// empty to `None`, so this states the value the handle is being given explicitly instead of
/// inheriting whatever this test process happens to have in its environment — the difference between
/// a pinned differential and a coincidence. `NXC_HOP` gets the same treatment. `NXC_ORIGIN` is the
/// one value that is REMOVED instead: since nxf 6j6v.07me the handle takes no argument for it and
/// the CLI derives it from the same workspace, so pinning it would hide a split (see [`origin`]).
fn nxc_orch(dir: &Path, log: &Path, actor: &str, session: Option<&str>) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxc");
    c.current_dir(dir)
        .env_remove("NXC_ORIGIN")
        .env("NXC_ACTOR", actor)
        .env("NXC_NOW", NOW)
        .env("NXC_SESSION", session.unwrap_or(""))
        .env("NXC_HOP", "0")
        .env("NXC_WORKER", "dry")
        .env("NXC_DRY_LOG", log)
        .env("NXC_TIMER", "dry")
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

/// Run a prepared `nxc` to success with `--json` and return its trimmed stdout.
fn run_json(mut cmd: Command, args: &[&str]) -> String {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = cmd
        .args(full)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("utf8").trim_end().to_string()
}

/// Run a prepared `nxc` expecting REJECTION, and return its `(kind, msg)` — `error::emit`'s
/// structured `--json` envelope, the exact pair the handle returns as `NxfError { kind, msg }`.
fn run_err(mut cmd: Command, args: &[&str]) -> (String, String) {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    let out = cmd
        .args(full)
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8(out).expect("utf8").trim()).expect("error envelope");
    (
        v["error"]["kind"].as_str().expect("kind").to_string(),
        v["error"]["msg"].as_str().expect("msg").to_string(),
    )
}

/// A `--json` object's string field, as the CLI emitted it.
fn json_field(json: &str, path: &[&str]) -> String {
    let mut v: serde_json::Value = serde_json::from_str(json).expect("json");
    for key in path {
        v = v[key].clone();
    }
    v.as_str()
        .unwrap_or_else(|| panic!("no string at {path:?} in {json}"))
        .to_string()
}

// `orchestration_parity_engine_and_nxc_drive_one_run_identically` stood here — one declared run
// driven end to end on BOTH seams, comparing every receipt, every read and the whole op log.
// REMOVED with the run record (6j6v.dvyq §3). Its successor is
// `surface_parity_engine_and_nxc_open_and_answer_one_thread_identically` below, which drives the
// same differential over the verbs that replaced it.

#[test]
fn orchestration_rejection_parity_both_seams_refuse_identically_and_persist_nothing() {
    // Rejection behaviour is part of the seam contract, not an afterthought: refusals that each
    // come from a DIFFERENT place in the shared implementation — the declaration catalogue
    // (`Definitions::role`) and the depth guard, twice over — asserted to produce the identical
    // `(kind, msg)` pair on both seams AND to leave the op log exactly as `init` left it. "Refuses the same way" without "persists nothing" would
    // miss the failure mode the preflight-before-persist ordering exists to prevent.
    let _env = lock_env();
    deterministic_ids();
    let app = TempDir::new().unwrap();
    let cli = TempDir::new().unwrap();
    init(app.path());
    init(cli.path());
    seed_declarations(app.path());
    seed_declarations(cli.path());
    let app_log = app.path().join("dry.log");
    let cli_log = cli.path().join("dry.log");
    let baseline = op_rows(app.path());

    let e = orch_engine(app.path(), &app_log);
    // `Commission` / `send --role` stood here on both sides until nxf 6j6v.dvyq §3 removed
    // the entrance from CLI and seam together. The refusals below are unchanged — they come from
    // `Definitions::role` and the depth guard, both of which `send --to <persona>` reaches through
    // the very same `orchestration::coordinator_commission` body — so what moved is the door, not
    // the
    // contract this case pins.
    let trigger = |to: &'static str| SendToRequest {
        machine: None,
        to,
        body: "go",
        refs: SendToRefs::ExplicitlyNone,
    };
    let pair = |e: nexus_chat::error::NxfError| (e.kind.as_str().to_string(), e.msg);

    // (a) an undeclared role — `Definitions::role`'s `not_found`, raised before anything is posted.
    assert_eq!(
        pair(
            e.send_to(caller(KICKOFF, None), trigger("nope"))
                .map(|_| ())
                .unwrap_err()
        ),
        run_err(
            nxc_orch(cli.path(), &cli_log, KICKOFF, None),
            &["send", "--to", "nope", "go"],
        ),
        "unknown role"
    );

    // (b) an over-cap hop CLAIMED BY THE CALLER stood here — `NXC_HOP` on the CLI against
    // `Caller { hop: over_cap, .. }` on the handle, pinned as one refusal worded once in
    // `orchestration::check_depth_guard`. REMOVED with the field (nxf 6j6v.07me): the seam has no
    // way left to claim a depth, so there is no second side to compare against and a differential
    // cannot be written at all.
    //
    // **Nothing about the GUARD is given up, and this case was the weaker of the pair anyway.** It
    // only ever proved the two seams agree on the FALLBACK — no session, so both read the claim —
    // which is the path that predates 6j6v.m48m. What decides for a real chain is (b2) below,
    // where the depth comes from `session_map` and both seams read the SAME row; that one is
    // untouched and still drives both sides. The claim itself survives where it is still made: the
    // CLI's `NXC_HOP`, which a spawned `nxc` process is handed and has no other way to be told, is
    // covered on the CLI side by `depth_guard.rs`, and the seam's session-carried depth by
    // `caller_seam.rs`, which walks a chain to the cap with nothing passed at all.
    let over_cap = nexus_chat::orchestration::MAX_HOP + 1;

    // (b2) the same refusal, reached the OTHER way (nxf 6j6v.m48m; PR #267 review, Test Quality #1).
    // (b) above only proves the two seams agree on the FALLBACK — no session, so both read the
    // caller's claim, which is the path that predates this ticket. The mechanism the ticket actually
    // added is the one below: a caller claiming a fresh chain (`hop: 0` / `NXC_HOP=0`, what
    // `unset NXC_HOP` produces) whose OWN session is recorded past the cap. That resolution reads
    // `session_map` — device-local, one row per workspace — so the two workspaces each need their
    // own seeded session, and holding it constant is what makes a divergence here mean the seams
    // differ rather than the fixtures do. Worth pinning precisely because the depth guard has
    // already cost one round of cross-seam drift.
    for path in [app.path(), cli.path()] {
        let mut s = Workspace::resolve(None, path)
            .unwrap()
            .open_chat_store()
            .unwrap();
        s.create_pending_session(DEEP_SESSION, "coder").unwrap();
        s.record_trigger_depth(DEEP_SESSION, over_cap).unwrap();
    }
    let mut deep_cli = nxc_orch(cli.path(), &cli_log, KICKOFF, Some(DEEP_SESSION));
    deep_cli.env("NXC_HOP", "0");
    assert_eq!(
        pair(
            e.send_to(caller(KICKOFF, Some(DEEP_SESSION)), trigger("coder"))
                .map(|_| ())
                .unwrap_err()
        ),
        run_err(deep_cli, &["send", "--to", "coder", "go"]),
        "a wiped hop counter over a session the store knows is past the cap"
    );

    // Two more cases stood here — a `--name` matching no declared workflow
    // (`Definitions::workflow`'s 0/1/N rule) and a `step done` with neither an explicit run nor an
    // ambient session to resolve one from. Both went with the run record (6j6v.dvyq §3). The two
    // places they came from are gone; the two above are the two that are left, and they are the
    // ones the preflight-before-persist ordering is really about.

    // Nothing was persisted by any of them, on either seam.
    assert_eq!(
        op_rows(app.path()),
        baseline,
        "the app seam persisted nothing"
    );
    assert_eq!(
        op_rows(cli.path()),
        baseline,
        "the CLI seam persisted nothing"
    );
    assert!(
        !app_log.exists() && !cli_log.exists(),
        "a rejected verb never reaches the worker"
    );
}

// ---- liveness parity (nxf 6j6v.d9cb) ----------------------------------------------------------
//
// `liveness_parity_both_seams_watch_nudge_and_escalate_identically` and its `seed_watched_workflow`
// fixture stood here: the step-liveness clock driven through both seams, asserting the same nudge,
// the same budget and the same escalation. REMOVED with the run record (6j6v.dvyq §3) — the clock
// was keyed on a run's CURRENT STEP and armed only when one fired, so it went with the runs and
// takes its differential with it.

// =================================================================================================
// The SURFACE differential (nxf 6j6v.p6m1) — `list` / `send --to` / `reply --thread` / `prime
// --persona`, both seams
// =================================================================================================
//
// The four verbs of the addressing surface, in the shape the two sections above already have. They
// were each tested thoroughly on their OWN seam — `tests/embed_surface.rs` for the handle,
// `tests/surface_cli.rs` for the CLI — and never against each other, which is precisely the gap
// this file's own history warns about: a differential compares verbs that exist on both sides, so
// what it cannot see is not a divergence but an OMISSION. `prime` was already caught that way once
// (see `read_parity_prime_report_equals_nxc_prime_on_both_views`); this closes the same door on the
// verbs added beside it (PR #295 review, Test Quality #1).
//
// WHAT IS COMPARED: the two directory projections (full, and one persona's own), the `send --to`
// receipt, both `reply --thread` receipts, the two `prime --persona` renderings, the op logs, and
// the dry worker's trigger stream — the last one because the persona path's whole point is that a
// session is STARTED, and two receipts can agree about a spawn that only one seam performed.
//
// The declarations are this test's own (`seed_surface_declarations`), written beside `ship.yaml`'s
// rather than folded into it, so every assertion in the section above keeps the fixture it was
// written against — the same reason the liveness differential declares its own workflow.

/// The persona the conversation is opened with, and the peer its address book names.
const PERSONA: &str = "coder";
const PEER: &str = "reviewer";

/// Two personas and a channel, with an address book on one of them — so `list` has something to
/// say in BOTH projections. A directory that came back empty on both seams would compare equal and
/// prove nothing, which is the vacuous pass the assertions below rule out explicitly.
fn seed_surface_declarations(dir: &Path) {
    let roles = dir.join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join(format!("{PERSONA}.yaml")),
        format!(
            "handle: {PERSONA}\njob_title: Implementer\njob_description: turns a work order into \
             code\nstage: junior\nsystem_prompt: You are the implementer.\n\
             address_book:\n  - to: {PEER}\n    why: get the work reviewed\n"
        ),
    )
    .unwrap();
    std::fs::write(
        roles.join(format!("{PEER}.yaml")),
        format!("handle: {PEER}\njob_title: Reviewer\nsystem_prompt: You are the reviewer.\n"),
    )
    .unwrap();
    // The channel DECLARES a description (nxf 6j6v.frek). Without one here the differential would
    // compare two `None`s and prove nothing about the field that was added precisely so the channel
    // half of a directory can say what it is for — the same vacuous-pass trap the doc above names.
    std::fs::write(
        roles.join("channels.yaml"),
        format!(
            "- name: standup\n  members: [{PERSONA}, {PEER}]\n  description: the daily sync of the \
             whole team\n"
        ),
    )
    .unwrap();
}

#[test]
fn surface_parity_engine_and_nxc_open_and_answer_one_thread_identically() {
    let _env = lock_env();
    deterministic_ids();
    let app = TempDir::new().unwrap();
    let cli = TempDir::new().unwrap();
    init(app.path());
    init(cli.path());
    seed_surface_declarations(app.path());
    seed_surface_declarations(cli.path());
    let app_log = app.path().join("dry.log");
    let cli_log = cli.path().join("dry.log");

    // ---- the app seam --------------------------------------------------------------------------
    let (app_full, app_book, app_opened, app_answered, app_back);
    let (app_prime_json, app_prime_human);
    let app_thread;
    let app_session;
    {
        let e = orch_engine(app.path(), &app_log);
        app_full = json_str(&e.directory(None).unwrap());
        app_book = json_str(&e.directory(Some(PERSONA)).unwrap());

        let opened = e
            .send_to(
                caller(KICKOFF, None),
                SendToRequest {
                    machine: None,
                    to: PERSONA,
                    body: WORK_ORDER,
                    refs: SendToRefs::ExplicitlyNone,
                },
            )
            .unwrap();
        app_thread = opened.thread_id.clone();
        app_session = opened
            .session
            .clone()
            .expect("a persona target starts exactly one session");
        app_opened = json_str(&opened);

        // The persona answers into the thread from its own session — which is what leaves a return
        // address for the turn after it.
        app_answered = json_str(
            &e.reply_thread(
                caller(PERSONA, Some(&app_session)),
                ReplyThreadRequest {
                    machine: None,
                    thread: &app_thread,
                    body: VERDICT,
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
            .unwrap(),
        );
        // …and the human answers back, which resumes that session through the thread's return
        // address — the one behaviour `reply --thread` adds over the verb underneath it.
        app_back = json_str(
            &e.reply_thread(
                caller(KICKOFF, None),
                ReplyThreadRequest {
                    machine: None,
                    thread: &app_thread,
                    body: "one more thing",
                    escalate: false,
                    needs_rework: false,
                    accept: false,
                },
            )
            .unwrap(),
        );

        // Read LAST, so the persona's own prime reports a conversation that has actually happened.
        let report = e
            .prime_as(
                &format!("{}/{PERSONA}", origin(app.path())),
                Some(PERSONA),
                NOW,
            )
            .unwrap();
        app_prime_json = report.to_value(false).to_string();
        app_prime_human = report.render_markdown(false);
    }

    // ---- the CLI seam: the identical script, one `nxc` process per step ------------------------
    let kickoff = || nxc_orch(cli.path(), &cli_log, KICKOFF, None);
    let cli_full = run_json(kickoff(), &["list"]);
    let cli_book = run_json(kickoff(), &["list", "--persona", PERSONA]);
    let cli_opened = run_json(
        kickoff(),
        &["send", "--to", PERSONA, WORK_ORDER, "--no-ref"],
    );
    let cli_thread = json_field(&cli_opened, &["thread_id"]);
    let cli_session = json_field(&cli_opened, &["session"]);
    assert_eq!(
        (&app_thread, &app_session),
        (&cli_thread, &cli_session),
        "deterministic ids: both seams minted the same thread and session"
    );
    let cli_answered = run_json(
        nxc_orch(cli.path(), &cli_log, PERSONA, Some(&cli_session)),
        &["reply", "--thread", &cli_thread, VERDICT],
    );
    let cli_back = run_json(
        kickoff(),
        &["reply", "--thread", &cli_thread, "one more thing"],
    );
    let mut prime_cmd = nxc_orch(cli.path(), &cli_log, PERSONA, Some(&cli_session));
    let cli_prime_human = String::from_utf8(
        prime_cmd
            .args(["prime", "--persona", PERSONA])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .expect("utf8");
    let cli_prime_json = run_json(
        nxc_orch(cli.path(), &cli_log, PERSONA, Some(&cli_session)),
        &["prime", "--persona", PERSONA],
    );

    // ---- (1) the reads: who can be addressed ---------------------------------------------------
    //
    // Compared with each side's own workspace root substituted out — see `without_workspace_root`
    // for why that keeps the differential intact rather than weakening it.
    let app_full = without_workspace_root(&app_full, app.path());
    let cli_full = without_workspace_root(&cli_full, cli.path());
    let app_book = without_workspace_root(&app_book, app.path());
    let cli_book = without_workspace_root(&cli_book, cli.path());
    assert_eq!(app_full, cli_full, "`list` parity: the whole declared team");
    assert_eq!(
        app_book, cli_book,
        "`list --persona` parity: one persona's own address book"
    );
    // The substitution really fired on both sides, and what it left behind is the declaration
    // source — otherwise this pair could pass by comparing two records that never carried it.
    assert!(
        app_full.contains("[WS]/.nxs-personas") && cli_full.contains("[WS]/.nxs-personas"),
        "both seams report where the catalogue came from:\napp: {app_full}\ncli: {cli_full}"
    );
    // Neither projection is empty — two empty directories would compare equal and prove nothing.
    assert!(
        app_full.contains(PERSONA) && app_full.contains(PEER) && app_full.contains("standup"),
        "the full catalogue really lists the team: {app_full}"
    );
    assert!(
        app_book.contains(PEER) && !app_book.contains("\"handle\":\"coder\""),
        "the book is the projection, not the catalogue: {app_book}"
    );
    // The channel's own words reach BOTH seams (nxf 6j6v.frek). This is what the field was added
    // for: an app rendering its directory has the same sentence to show as `nxc list` does, so the
    // two cannot describe the same channel differently.
    assert!(
        app_full.contains("the daily sync of the whole team"),
        "the declared channel description crosses the seam: {app_full}"
    );

    // ---- (2) the writes: the receipts, byte for byte -------------------------------------------
    // Serialized bytes rather than field by field, for the reason the orchestration section states:
    // the receipt type IS the JSON contract of both seams at once.
    assert_eq!(app_opened, cli_opened, "`send --to` receipt parity");
    assert_eq!(
        app_answered, cli_answered,
        "`reply --thread` receipt parity, the persona's turn"
    );
    assert_eq!(
        app_back, cli_back,
        "`reply --thread` receipt parity, the turn that resumes the persona"
    );
    // The script really exercised the two things this surface adds, rather than agreeing about
    // nothing: a persona target resolved as one, and the thread's return address woke it.
    assert!(
        app_opened.contains("\"target\":\"persona\""),
        "the target resolved as a persona: {app_opened}"
    );
    assert!(
        app_back.contains(&app_session),
        "the reply resumed the session the thread's return address named: {app_back}"
    );

    // ---- (3) the reads after the writes: the persona's own prime -------------------------------
    assert_eq!(
        without_workspace_root(&app_prime_json, app.path()),
        without_workspace_root(&cli_prime_json, cli.path()),
        "`prime --persona` --json parity"
    );
    assert_eq!(
        format!("{app_prime_human}\n"),
        cli_prime_human,
        "`prime --persona` human parity: the block a host injects IS the one the CLI prints"
    );
    assert!(
        app_prime_human.contains("## You are") && app_prime_human.contains(PEER),
        "the persona is told who it is and whom it may address: {app_prime_human}"
    );

    // ---- (4) the op logs and the trigger stream ------------------------------------------------
    let (app_ops, cli_ops) = (op_rows(app.path()), op_rows(cli.path()));
    assert_eq!(app_ops, cli_ops, "the two seams emit identical op logs");
    assert!(
        app_ops.iter().filter(|r| r.3 == "message").count() >= 3,
        "the three posts are in the log: {app_ops:?}"
    );
    let app_triggers = std::fs::read_to_string(&app_log).unwrap_or_default();
    let cli_triggers = std::fs::read_to_string(&cli_log).unwrap_or_default();
    assert_eq!(
        app_triggers, cli_triggers,
        "the two seams recorded identical trigger streams"
    );
    assert_eq!(
        app_triggers.matches("trigger role=").count(),
        2,
        "the send started the persona and the reply resumed it: {app_triggers}"
    );
}
