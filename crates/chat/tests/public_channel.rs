//! The `public` channel kind (nxf 6j6v.bd6g): a third kind beside `group`/`direct` that is
//! READABLE and ADDRESSABLE without membership, and DISCOVERABLE by anyone in the workspace.
//!
//! The need comes from app-foundations (spec §8 D6.2, its own WAIT chore `41j0.hpvj`): every
//! project exposes its channel owner under one channel — the project's FRONT DOOR — so a PM can
//! report a bug straight to another project's PM. Addressing a front door already worked (`send`
//! has no membership gate); READING one did not, and that is the half this file pins.
//!
//! # What this suite holds, and why each half matters
//!
//! `public` is a READ opening. It makes no write gate wider and none narrower — so the tests come
//! in pairs: for every read that now lets a stranger through on a `public` channel there is a
//! `group` twin proving the refusal is untouched, and the write side is pinned in the one direction
//! it still has: a stranger may still post. (It was pinned in both until nxf 6j6v.4d2z — the other
//! direction was the read cursor a stranger could not advance, and there is no cursor now.)
//!
//! Both seams are driven on purpose. The products speak `Engine::*` and never spawn `nxc`
//! (project rule `engine-seam-test-rule`), while the CLI is what a human types — a suite that
//! drives only one of them proves the path the other does not walk.

mod common;

use std::path::Path;

use nexus_chat::channel::{ChannelDecl, ChannelKind, Visibility};
use nexus_chat::definitions::Definitions;
use nexus_chat::engine::{Engine, EngineConfig};
use nexus_chat::error::ErrorKind;
use nexus_chat::facade::{self, AskRequest, ReplyRequest, SendRequest};
use nexus_chat::model::{
    Disposition, MessageKind, Priority, Refs, CHANNEL_KIND_GROUP, CHANNEL_KIND_PUBLIC,
};
use nexus_chat::orchestration::Caller;
use nexus_chat::store::ChatStore;
use nexus_chat::surface::{SendToRefs, SendToRequest};
use nexus_chat::timer::TimerConfig;
use nexus_chat::worker::WorkerConfig;
use nexus_chat::workspace::{chat_config, setup, ChatWorkspaceExt, Workspace};
use tempfile::TempDir;

const NOW: &str = "2026-08-10T09:00:00Z";
/// The `<origin>` the FIXTURE mints under: this file's store-level cases build their own `Ctx` and
/// hand it this value, so it is the test's own string and stays a literal. It is NOT what the two
/// [`Engine`]-driven cases at the bottom get — a handle answers with its workspace's replica prefix
/// (nxf 6j6v.07me), so anything compared against what THEY wrote is derived from `e.origin()` at
/// the assertion rather than spelled here.
const ORIGIN: &str = "local";
/// The project's front door, and the handle that owns it.
const DOOR: &str = "decl:pm";
const OWNER: &str = "local/pm";
/// A member-only channel beside the door — every refusal this suite claims is still in force is
/// claimed against THIS one, so "the gate is open" can never quietly mean "the gate is gone".
const PRIVATE: &str = "c-private";
/// Somebody from another project: a member of nothing in this workspace.
const STRANGER: &str = "local/other-pm";

// ---- fixtures ---------------------------------------------------------------

/// A store with the public front door (owner joined) and a private group channel beside it.
fn seed() -> ChatStore {
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field(DOOR, "kind", CHANNEL_KIND_PUBLIC, OWNER);
    s.set_channel_field(DOOR, "name", "pm", OWNER);
    s.add_member(DOOR, OWNER, OWNER);
    s.set_channel_field(PRIVATE, "kind", CHANNEL_KIND_GROUP, OWNER);
    s.set_channel_field(PRIVATE, "name", "inner", OWNER);
    s.add_member(PRIVATE, OWNER, OWNER);
    s
}

/// Post `body` into `channel` as the door's owner, through the real `send` facade.
fn post(s: &mut ChatStore, channel: &str, body: &str) -> String {
    facade::send(
        s,
        SendRequest {
            now: NOW,
            origin: ORIGIN,
            actor: "pm",
            channel,
            body,
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs::default(),
        },
    )
    .expect("owner posts")
    .message_id
}

/// Open a board in `channel` as the owner, through the real `ask` facade.
fn open_board(s: &mut ChatStore, channel: &str) -> String {
    facade::ask(
        s,
        AskRequest {
            now: NOW,
            origin: ORIGIN,
            actor: "pm",
            channel,
            body: "what is the state of this project?",
            expect: &[OWNER.to_string()],
            deadline: None,
            kind: MessageKind::Question,
            priority: Priority::Normal,
            refs: Refs::default(),
        },
    )
    .expect("owner opens a board")
    .thread_id
}

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

/// Declare `defs` in the workspace's own `.nxs-personas/` folder, then open a handle over it — see
/// `embed_surface.rs`'s twin for why injection is no longer an option (nxf 6j6v.dvyq step 6).
fn engine(dir: &Path, defs: Definitions) -> Engine {
    common::write_declarations(dir, &defs);
    Engine::open_with(
        None,
        dir,
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine")
}

// ---- reading a front door without being in it -------------------------------

#[test]
fn a_stranger_reads_a_public_channels_history_and_is_still_refused_a_private_one() {
    let mut s = seed();
    post(&mut s, DOOR, "the door is open");
    post(&mut s, PRIVATE, "the inner room is not");

    let seen = facade::messages(&s, DOOR, STRANGER).expect("a stranger reads the front door");
    assert_eq!(
        seen.iter().map(|m| m.body.as_str()).collect::<Vec<_>>(),
        ["the door is open"],
        "the public channel's history comes back in full"
    );

    // The twin: the same stranger, the same read, a `group` channel — untouched.
    let refused = facade::messages(&s, PRIVATE, STRANGER).unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Forbidden);
}

#[test]
fn an_unknown_channel_is_still_not_found_rather_than_readable() {
    let s = seed();
    // The kind lookup must not turn "there is no such channel" into anything softer: a typo'd
    // front door has to say so, or a caller silently reads an empty room forever.
    let e = facade::messages(&s, "decl:nope", STRANGER).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NotFound);
}

#[test]
fn a_stranger_reads_a_thread_and_its_board_in_a_public_channel() {
    let mut s = seed();
    let thread = open_board(&mut s, DOOR);

    // BOTH readers take the channel's `visibility` since nxf 6j6v.yr59 — `thread` gained it with
    // the seam's one message reader, and it is passed here the way `thread_board` beside it always
    // took it: as an explicit capability, resolved by the adapter above.
    let t = facade::thread(&s, &thread, STRANGER, Visibility::RequesterOnly)
        .expect("a stranger reads the thread");
    assert_eq!(t.channel_id, DOOR);
    assert_eq!(t.messages.len(), 1, "the opening message is there");

    // `thread_board` opens too — the same question ("what is in this channel") must not be open
    // through one verb and shut through its neighbour. The visibility filter is UNCHANGED: a
    // stranger is by definition not the opener, so `requester_only` still hands over only the
    // opening message.
    let board = facade::thread_board(&s, &thread, NOW, STRANGER, Visibility::RequesterOnly)
        .expect("a stranger reads the board");
    assert_eq!(board.quorum.opener.as_deref(), Some(OWNER));
    assert_eq!(board.messages.len(), 1);
}

#[test]
fn a_stranger_is_still_refused_a_thread_and_board_in_a_private_channel() {
    let mut s = seed();
    let thread = open_board(&mut s, PRIVATE);

    assert_eq!(
        facade::thread(&s, &thread, STRANGER, Visibility::RequesterOnly)
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
    assert_eq!(
        facade::thread_board(&s, &thread, NOW, STRANGER, Visibility::RequesterOnly)
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden
    );
}

// ---- the write side: nothing got wider, nothing got narrower ----------------

#[test]
fn a_stranger_may_post_into_a_public_channel_and_the_owner_reads_it() {
    let mut s = seed();
    let receipt = facade::send(
        &mut s,
        SendRequest {
            now: NOW,
            origin: ORIGIN,
            actor: "other-pm",
            channel: DOOR,
            body: "your parser drops empty lines",
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread: None,
            refs: Refs::default(),
        },
    )
    .expect("a front door a stranger cannot write to is not a front door");
    assert!(!receipt.message_id.is_empty());

    let seen = facade::messages(&s, DOOR, OWNER).expect("the owner reads its own door");
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].sender, STRANGER);
}

#[test]
fn a_stranger_may_reply_in_a_public_channels_thread() {
    let mut s = seed();
    let thread = open_board(&mut s, DOOR);
    facade::reply(
        &mut s,
        ReplyRequest {
            now: NOW,
            origin: ORIGIN,
            actor: "other-pm",
            target: &thread,
            body: "we shipped it in 0.4.2",
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            refs: Refs::default(),
            if_unanswered: false,
        },
    )
    .expect("replying into an open door needs no membership either");

    let seen = facade::messages(&s, DOOR, STRANGER).unwrap();
    assert_eq!(seen.len(), 2, "opening message plus the stranger's reply");
}

// `a_stranger_still_cannot_advance_a_read_cursor_in_a_public_channel` stood here: the one WRITE the
// `public` kind deliberately did not open, because a non-member had no read state to advance and a
// cursor written for one would be a row nothing ever reads — permanent, syncing garbage in a
// grow-only register. The ack and the register both went with nxf 6j6v.4d2z, so the exception has
// nothing left to be an exception to. What the kind opens is still exactly the three reads, which
// is what this file's remaining tests hold.

// ---- discovery --------------------------------------------------------------

#[test]
fn discovery_lists_public_channels_for_a_stranger_and_leaves_the_private_ones_out() {
    let mut s = seed();
    post(&mut s, DOOR, "hello");

    let found = facade::public_channels(&s).expect("discovery needs no membership");
    assert_eq!(
        found
            .iter()
            .map(|c| c.channel_id.as_str())
            .collect::<Vec<_>>(),
        [DOOR],
        "only the public channel, and the private one is not in it"
    );
    let door = &found[0];
    assert_eq!(door.kind.as_deref(), Some(CHANNEL_KIND_PUBLIC));
    assert_eq!(door.name.as_deref(), Some("pm"));
    assert_eq!(door.members, 1, "the COUNT travels, the handles do not");
    assert!(!door.degraded);
}

// `discovery_reports_a_members_real_unread_in_the_same_public_channel` stood here: the other half
// of the same claim — a MEMBER of a public channel saw its genuine unread count on the discovery
// read, so the `0` above was the true answer for a stranger rather than an inherited accident.
// `ChannelView` carries no unread at all since nxf 6j6v.4d2z, so both halves are gone and the
// discovery read is now about addressability alone, which is what `persona::FrontDoor` always said
// it should be.

#[test]
fn the_membership_scoped_reads_do_not_change_their_result_set() {
    let mut s = seed();
    post(&mut s, DOOR, "findable only from inside");

    // `channels` still means "the channels I am in" — a public channel I never joined is not one.
    assert!(
        facade::channels(&s, STRANGER).unwrap().is_empty(),
        "discovery is its own read, not a widening of this one"
    );
    // ... and search stays scoped too: a body-substring read that silently swept in every public
    // channel would change every existing caller's hit list.
    assert!(facade::search(&s, STRANGER, "findable", &[])
        .unwrap()
        .is_empty());
    // The owner, being a member, sees both as before.
    assert_eq!(facade::channels(&s, OWNER).unwrap().len(), 2);
    assert_eq!(facade::search(&s, OWNER, "findable", &[]).unwrap().len(), 1);
}

// ---- the two ways a public channel comes into being -------------------------

/// Discovery, on the seam it now lives on alone.
///
/// This was `the_cli_creates_a_public_channel_and_lists_it_for_a_non_member`, and both halves of it
/// lost their command line to 6j6v.dvyq §3: `channels create --public` minted the door, and
/// `channels public` listed it. A door is a DECLARATION now (`kind: public`, pinned by
/// `a_declared_channel_carries_its_declared_kind_into_the_store` below), and cross-project
/// discovery LEAVES the agent surface without a successor there — the owner's decision of
/// 2026-08-19, recorded in `tests/seam_disposition.rs`: `Engine::public_channels` stays and an app
/// renders it.
///
/// Which is exactly why this test exists in this shape. A read whose CLI half is gone has no parity
/// partner left, so without a gate of its own it would simply stop being checked — a coverage loss
/// wearing the clothes of a cleanup.
#[test]
fn a_non_member_discovers_a_public_channel_through_the_seam() {
    let s = seed();
    let rows = facade::public_channels(&s).expect("discovery needs no membership");
    assert_eq!(rows.len(), 1, "only the public one: {rows:?}");
    assert_eq!(rows[0].channel_id, DOOR);
    assert_eq!(rows[0].kind.as_deref(), Some(CHANNEL_KIND_PUBLIC));

    // And the membership-scoped read is NOT widened by it — the two are different questions.
    assert!(
        facade::channels(&s, STRANGER)
            .expect("channels reads")
            .is_empty(),
        "discovery is its own read"
    );
}

#[test]
fn a_declared_channel_carries_its_declared_kind_into_the_store() {
    let tmp = workspace();
    let defs = Definitions::new(
        vec![
            serde_yaml::from_str("handle: pm\nsystem_prompt: You own the door.\n").unwrap(),
            serde_yaml::from_str("handle: dev\nsystem_prompt: You build things.\n").unwrap(),
        ],
        vec![
            serde_yaml::from_str::<ChannelDecl>("name: pm\nmembers: [pm, dev]\nkind: public\n")
                .unwrap(),
        ],
    )
    .expect("definitions");
    let e = engine(tmp.path(), defs);

    // Opening a board on the declared channel is what materialises it; the declared kind is what
    // it materialises AS — that is what lets a cross-project directory select by kind instead of
    // by an agreed-upon name (app-foundations 41j0.3sq0).
    // Through `send_to`, which is the ONE entrance a declared channel has since 6j6v.dvyq §3 —
    // `Engine::channel_open` was its second door and is gone. Same mechanism underneath
    // (`open_declared_channel_and_fan_out`), reached by naming the channel as a target.
    e.send_to(
        Caller {
            session: None,
            actor: Some("pm"),
            now: Some(NOW),
        },
        SendToRequest {
            machine: None,
            to: "pm",
            body: "opening the door",
            refs: SendToRefs::ExplicitlyNone,
        },
    )
    .expect("open the declared channel");

    // Discovery through the seam is `directory` since nxf 6j6v.yr59 — `Engine::public_channels`
    // folded into it, so an app learns the front doors from the same call that names the personas
    // and the declared channels.
    let found = e
        .directory(None)
        .expect("the seam an app links offers discovery too")
        .public_channels;
    assert_eq!(
        found
            .iter()
            .map(|c| c.channel_id.as_str())
            .collect::<Vec<_>>(),
        [DOOR]
    );
    assert_eq!(found[0].name.as_deref(), Some("pm"));

    // And the point of the whole exercise: a stranger reads what is behind that door. TWO messages
    // since nxf 6j6v.pf6j — the requester's own into the channel thread, and the supervisor's task
    // into the one member thread it opened below it. What the assertion is about is unchanged: a
    // declared public channel reads at all, without membership.
    // Read off the store: the channel-wide read left the handle with `Engine::messages` (nxf
    // 6j6v.yr59), and what this asserts is what the fan-out WROTE into the channel — two threads'
    // worth — rather than a seam capability. The seam half of "a stranger reads a public channel"
    // is `Engine::thread` below.
    let seen = common::channel_messages(tmp.path(), DOOR, STRANGER)
        .expect("a declared public channel reads without membership");
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0].body, "opening the door");
    assert_eq!(
        seen[1].sender,
        format!("{}/__channel__", e.origin()),
        "{seen:?}"
    );
}

#[test]
fn a_declaration_without_a_kind_is_still_a_group_channel() {
    // Every `channels.yaml` written before this field existed must mean exactly what it meant.
    let decl: ChannelDecl = serde_yaml::from_str("name: review\nmembers: [a, b]\n").unwrap();
    assert_eq!(decl.kind, ChannelKind::Group);
    assert_eq!(decl.kind.as_str(), CHANNEL_KIND_GROUP);

    let public: ChannelDecl =
        serde_yaml::from_str("name: pm\nmembers: [pm]\nkind: public\n").unwrap();
    assert_eq!(public.kind, ChannelKind::Public);
    assert_eq!(public.kind.as_str(), CHANNEL_KIND_PUBLIC);
}

#[test]
fn a_declared_kind_that_names_nothing_is_a_loud_parse_error() {
    // `direct` is minted, never declared, and a typo must not silently become a group channel.
    let e = serde_yaml::from_str::<ChannelDecl>("name: pm\nmembers: [pm]\nkind: direct\n");
    assert!(e.is_err(), "an undeclarable kind is rejected: {e:?}");
}

#[test]
fn the_engine_seam_opens_a_thread_and_its_board_on_a_declared_public_channel() {
    // The seam-level twin of `a_stranger_reads_a_thread_and_its_board_in_a_public_channel`
    // (independent review, Test Quality #1, High). `Engine::thread` has NO CLI verb at all, so
    // `Engine::*` is the ONLY way manufakt.io / nexflow.it reach it — the project's
    // `engine-seam-test-rule` exists because covering the facade and calling that done is the
    // omission that has bitten three reviews in a row.
    //
    // Declarations go on DISK here, and the definition source is the default one, so this walks
    // the same path a real embedder walks: `channels.yaml` → materialised channel → the read.
    // `Engine::thread_board` resolves `visibility` from that same file, which the supplied-defs
    // route would leave unresolvable.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\nsystem_prompt: You own the door.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("dev.yaml"),
        "handle: dev\nsystem_prompt: You build things.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: pm\n  members: [pm, dev]\n  kind: public\n- name: inner\n  members: [pm, dev]\n",
    )
    .unwrap();
    let e = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    let caller = Caller {
        session: None,
        actor: Some("pm"),
        now: Some(NOW),
    };
    let open = |to| SendToRequest {
        machine: None,
        to,
        body: "opening",
        refs: SendToRefs::ExplicitlyNone,
    };
    let public_thread = e
        .send_to(caller, open("pm"))
        .expect("open the public channel")
        .thread_id;
    let private_thread = e
        .send_to(caller, open("inner"))
        .expect("open the group channel")
        .thread_id;

    // The opening: both reads answer a stranger on the PUBLIC channel.
    let t = e
        .thread(&public_thread, STRANGER)
        .expect("Engine::thread reads a public channel without membership");
    assert_eq!(t.channel_id, DOOR);
    // `visibility` is UNCHANGED by the opened gate, and since nxf 6j6v.yr59 the ONE message reader
    // is where it is applied: the declaration defaults to `requester_only` and a stranger is by
    // definition not the opener, so exactly the opening message travels.
    assert_eq!(t.messages.len(), 1, "{:?}", t.messages);

    // The twin: the same seam call, a `group` channel — the refusal is untouched.
    assert_eq!(
        e.thread(&private_thread, STRANGER).unwrap_err().kind,
        ErrorKind::Forbidden
    );
}

// ---- the substrate read underneath ------------------------------------------

#[test]
fn a_channel_row_whose_kind_was_never_written_reads_as_no_kind_rather_than_a_db_fault() {
    // `channels.kind` is nullable (`crates/chat/src/schema.rs`), and a row with the name folded and
    // the kind not yet folded is a REAL state — ops arrive one at a time, so any interleaved sync
    // fold passes through it. The gate runs on every channel read, so it has to survive it.
    //
    // This is a disclosed behaviour change (independent review, Code Quality #1, High): the old
    // `is_degraded_dm` read the column as a non-nullable `String` and returned `io` — "the database
    // is broken" — for a schema-legal row. It now answers the question it was asked: a channel with
    // no kind is not a degraded DM, so `false`. The changelog fragment says so.
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("half-folded", "name", "half", OWNER);
    s.add_member("half-folded", OWNER, OWNER);

    assert_eq!(s.channel_kind("half-folded").unwrap(), None);
    assert!(
        !s.is_degraded_dm("half-folded").unwrap(),
        "a channel with no kind is not a degraded DM — and not a db fault either"
    );
    assert_eq!(
        s.channel_kind("no-such-channel").unwrap(),
        None,
        "and an absent channel is the same `None`, not an error"
    );

    // The load-bearing half: the read gate must fall through to the membership check rather than
    // blow up — a member still reads, a stranger is still refused.
    facade::messages(&s, "half-folded", OWNER).expect("a member reads a kind-less channel");
    assert_eq!(
        facade::messages(&s, "half-folded", STRANGER)
            .unwrap_err()
            .kind,
        ErrorKind::Forbidden,
        "no kind is not `public` — the absence must not open the door"
    );
}

#[test]
fn the_public_kind_converges_across_two_sites_like_any_other_channel_field() {
    // `kind` is a per-field LWW register in the shared substrate, and `public` is the first new
    // value it has carried since M1 (independent review, Test Quality #3). The fold logic is
    // untouched, so this pins the claim rather than a suspicion: the new value rides the ordinary
    // op path, and a replica that learns about a front door treats it as one.
    let mut a = ChatStore::open_in_memory(1);
    a.set_channel_field(DOOR, "kind", CHANNEL_KIND_PUBLIC, OWNER);
    a.set_channel_field(DOOR, "name", "pm", OWNER);
    post(&mut a, DOOR, "knock knock");

    let mut b = ChatStore::open_in_memory(2);
    b.apply(&a.export());

    assert_eq!(
        b.channel_kind(DOOR).unwrap().as_deref(),
        Some(CHANNEL_KIND_PUBLIC)
    );
    assert_eq!(
        facade::public_channels(&b).unwrap().len(),
        1,
        "the second site discovers the front door it was told about"
    );
    assert_eq!(
        facade::messages(&b, DOOR, STRANGER).unwrap().len(),
        1,
        "and reads it without membership, exactly as the first site does"
    );

    // Re-exporting from B and folding back into A converges rather than duplicating — the ordinary
    // idempotence of the op log, asserted here because a new register value is the case where a
    // fold bug would show up first.
    a.apply(&b.export());
    assert_eq!(
        a.channel_kind(DOOR).unwrap().as_deref(),
        Some(CHANNEL_KIND_PUBLIC)
    );
    assert_eq!(facade::public_channels(&a).unwrap().len(), 1);
}

#[test]
fn a_degraded_dm_is_unaffected_by_the_new_kind() {
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("dm-1", "kind", "direct", OWNER);
    s.add_member("dm-1", OWNER, OWNER);
    assert!(
        s.is_degraded_dm("dm-1").unwrap(),
        "a one-member DM is still degraded"
    );
    assert!(
        facade::public_channels(&s).unwrap().is_empty(),
        "and a DM is not discoverable"
    );
}

#[test]
fn a_public_channel_with_no_members_is_still_discoverable_and_readable() {
    // The front door is materialised by whoever declares it; nobody has to JOIN it for it to work,
    // which is the whole difference this kind makes. `members: 0` is a legitimate state, not a
    // degraded one.
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("decl:open", "kind", CHANNEL_KIND_PUBLIC, OWNER);
    s.set_channel_field("decl:open", "name", "open", OWNER);
    post(&mut s, "decl:open", "anybody there?");

    let found = facade::public_channels(&s).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].members, 0);
    assert!(!found[0].degraded);
    assert_eq!(
        facade::messages(&s, "decl:open", STRANGER).unwrap().len(),
        1
    );
}

// ---- discovery folded into the directory (nxf 6j6v.yr59) --------------------

#[test]
fn the_directory_names_everything_addressable_including_a_door_no_declaration_here_names() {
    // yr59's acceptance point: `directory` names personas AND channels SAMT ZWECK, and a caller
    // learns everything addressable from it WITHOUT a second verb. Owner, 2026-08-21: "`directory`
    // zeigt uebrigens auch die Kanaele an und wofuer sie da sind."
    //
    // The half that needed a second verb was DISCOVERY. `Engine::public_channels` was the
    // workspace's front-door read (6j6v.bd6g) and it is the one thing `list` genuinely could not
    // answer, because a door reaching this workspace by SYNC carries no declaration here to read
    // — which is exactly why the owner's decision of 2026-08-19 kept it as a read of its own.
    // yr59 overrules that: the doors ride on the directory now, so "what is there" is ONE call.
    //
    // So the fixture carries both kinds at once: two declared channels with their purposes, and a
    // public channel that NOTHING here declares, folded straight into the store the way a sync
    // would fold it.
    let tmp = workspace();
    let roles = tmp.path().join(".nxs-personas");
    std::fs::create_dir_all(&roles).unwrap();
    std::fs::write(
        roles.join("pm.yaml"),
        "handle: pm\njob_title: Product Manager\njob_description: Owns the roadmap.\n\
         system_prompt: You own the door.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("dev.yaml"),
        "handle: dev\njob_title: Engineer\njob_description: Builds it.\n\
         system_prompt: You build things.\n",
    )
    .unwrap();
    std::fs::write(
        roles.join("channels.yaml"),
        "- name: pm\n  members: [pm, dev]\n  kind: public\n  description: The project's front door.\n\
         - name: review\n  members: [pm, dev]\n  description: Where a change is looked at.\n",
    )
    .unwrap();

    // A front door from ANOTHER project, arrived by sync: a `public` channel row with no
    // declaration in this workspace at all.
    {
        let mut s = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        s.set_wall_clock(NOW);
        s.set_channel_field("c-elsewhere", "kind", CHANNEL_KIND_PUBLIC, STRANGER);
        s.set_channel_field("c-elsewhere", "name", "other-pm", STRANGER);
        s.add_member("c-elsewhere", STRANGER, STRANGER);
    }

    let e = Engine::open_with(
        None,
        tmp.path(),
        EngineConfig {
            worker: WorkerConfig::Dry { log: None },
            timer: TimerConfig::Disabled,
            ..EngineConfig::default()
        },
    )
    .expect("open engine");

    // ONE call. Everything below comes out of this single value.
    let dir = e.directory(None).expect("the directory reads");

    // WHO — every declared persona, with what they are for.
    assert_eq!(
        dir.personas
            .iter()
            .map(|p| (p.handle.as_str(), p.job_description.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("dev", Some("Builds it.")),
            ("pm", Some("Owns the roadmap.")),
        ]
    );

    // WHAT — every declared channel, with what it is FOR and who is in it.
    assert_eq!(
        dir.channels
            .iter()
            .map(|c| (c.name.as_str(), c.description.as_deref(), c.members.len()))
            .collect::<Vec<_>>(),
        [
            ("pm", Some("The project's front door."), 2),
            ("review", Some("Where a change is looked at."), 2),
        ]
    );

    // AND the doors — including the one no declaration here names, which is the whole of what the
    // second verb used to be for.
    assert_eq!(
        dir.public_channels
            .iter()
            .map(|d| (d.channel_id.as_str(), d.name.as_deref()))
            .collect::<Vec<_>>(),
        [("c-elsewhere", Some("other-pm"))],
        "the synced front door is in the directory: {:?}",
        dir.public_channels
    );

    // "Without a second verb", said as a comparison rather than as a claim: the directory carries
    // every door the discovery read carries.
    let by_the_old_road: Vec<String> = {
        let s = Workspace::resolve(None, tmp.path())
            .unwrap()
            .open_chat_store()
            .unwrap();
        facade::public_channels(&s)
            .unwrap()
            .into_iter()
            .map(|c| c.channel_id)
            .collect()
    };
    assert_eq!(
        dir.public_channels
            .iter()
            .map(|d| d.channel_id.clone())
            .collect::<Vec<_>>(),
        by_the_old_road
    );

    // And the names it hands out are the names the WRITE takes: a persona and a channel, both
    // addressed straight out of the directory with nothing looked up in between.
    let caller = Caller {
        session: None,
        actor: Some("pm"),
        now: Some(NOW),
    };
    for target in [dir.personas[0].handle.clone(), dir.channels[1].name.clone()] {
        e.send_to(
            caller,
            SendToRequest {
                machine: None,
                to: &target,
                body: "addressed straight out of the directory",
                refs: SendToRefs::ExplicitlyNone,
            },
        )
        .unwrap_or_else(|e| {
            panic!("`{target}` is named by the directory and refused by send_to: {e:?}")
        });
    }
}
