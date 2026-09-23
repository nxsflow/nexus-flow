//! The reads that OUTLIVED their verb (nexus-flow-6j6v.dvyq §3, owner decision of 2026-08-19).
//!
//! # Why this file exists at all
//!
//! `tests/parity.rs` is a DIFFERENTIAL: it drives one script through the `nxc` CLI and through
//! `Engine` and compares the two byte for byte. That is the right gate for a capability both
//! surfaces carry, and its core is untouched — `send --to`, `reply --thread`, `status`, `list`,
//! `search`, `transcript` all still run the same verb kernel on both sides and must still answer
//! identically.
//!
//! What §3 changes is that some reads now exist on ONE surface. `nxc channels` is gone — a channel
//! is a declaration, its membership is that file's `members:`, and `ensure_declared_channel`
//! materialises it on the first `send --to` — while `facade::channels` and
//! `facade::public_channels` stay, because they are what the reads are built out of.
//!
//! **The `Engine` half of that sentence has since gone** (nxf 6j6v.yr59, 2026-08-21): the handle
//! carries seven read verbs, and neither of these two is one of them — `Engine::public_channels`
//! folded into `Engine::directory` (front doors and all) and `Engine::channels` left as a named
//! loss on its unread half. The disposition record next door carries both decisions. That makes
//! THIS FILE the only gate left over these two derivations: no CLI half, no seam method, no parity
//! partner. It was already the answer to "a read with no verb stops being checked"; it is now the
//! answer to "a read with no verb AND no handle method" as well.
//!
//! A read with no CLI half has no parity partner. Left alone it would simply stop being checked,
//! and the removal of its verb would take its coverage with it — a coverage loss wearing the
//! clothes of a cleanup. So the owner's decision was the more expensive of the three on offer: the
//! differential keeps its core, the claim that both surfaces carry the same SET of verbs is given
//! up EXPLICITLY (`parity.rs`'s module doc says so), and the orphaned reads get a gate of their own.
//! This is that gate.
//!
//! # What it holds
//!
//! The properties the removed `nxc channels list` used to prove through the CLI: the read is
//! MEMBERSHIP-SCOPED, it carries the fields an app renders, and it flags a DEGRADED direct channel
//! — a `direct` channel whose resolved membership is not exactly two (spec §3.3). That last one is
//! the reason this is not simply covered by `is_degraded_dm`'s own unit test: the flag has to
//! survive the projection into `ChannelView`, which is what an app actually reads.
//!
//! `public_channels` — the cross-project discovery read, which loses its CLI half entirely — is
//! pinned beside its own subject matter, in `tests/public_channel.rs`.
//!
//! # A SECOND removal wrote into this file, and a THIRD took it back out (nxf 6j6v.1gm9, 6j6v.4d2z)
//!
//! `nxc inbox` and `nxc read` went the same way on 2026-08-27, for the reason that "people pull and
//! agents get pushed" — an agent's message arrives in the session that starts or resumes it, so a
//! pull verb for it duplicated a delivery that had already happened (both measured 0 uses across 66
//! role sessions), and a human reads a CONVERSATION rather than a flat unread list.
//!
//! Their compute halves stayed then — `facade::inbox` was what `prime` read, `facade::mark_read`
//! the ack an app issued — so their black-box coverage in `tests/verbs.rs` moved here rather than
//! going with the verbs, which is precisely the "coverage loss wearing the clothes of a cleanup"
//! this file exists to refuse.
//!
//! **nxf 6j6v.4d2z then removed those compute halves too**, and the two moved tests are deleted
//! with them — see the note where they stood. That is not the same event twice: the first time the
//! behaviour outlived its entrance, which is when coverage has to be rehomed; the second time the
//! behaviour itself was measured, decided and removed. `seam_disposition.rs` records both moves,
//! and its two rows read `Leaves … landed` now rather than `Stays`.

use nexus_chat::facade;
use nexus_chat::model::{CHANNEL_KIND_DIRECT, CHANNEL_KIND_GROUP};
use nexus_chat::store::ChatStore;

const ALICE: &str = "local/alice";
const BOB: &str = "local/bob";
const CAROL: &str = "local/carol";

#[test]
fn the_channel_list_is_membership_scoped_and_carries_what_an_app_renders() {
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("c-review", "name", "review", ALICE);
    s.set_channel_field("c-review", "kind", CHANNEL_KIND_GROUP, ALICE);
    s.add_member("c-review", ALICE, ALICE);
    // A channel alice is not in.
    s.set_channel_field("c-elsewhere", "name", "elsewhere", BOB);
    s.set_channel_field("c-elsewhere", "kind", CHANNEL_KIND_GROUP, BOB);
    s.add_member("c-elsewhere", BOB, BOB);

    let rows = facade::channels(&s, ALICE).expect("channels reads");
    assert_eq!(rows.len(), 1, "membership-scoped: {rows:?}");
    assert_eq!(rows[0].channel_id, "c-review");
    assert_eq!(rows[0].name.as_deref(), Some("review"));
    assert_eq!(rows[0].kind.as_deref(), Some(CHANNEL_KIND_GROUP));
    assert_eq!(rows[0].members, 1);
    assert!(!rows[0].degraded);
}

#[test]
fn a_direct_channel_with_a_third_member_is_flagged_degraded_in_the_view() {
    // `channels_list_flags_a_degraded_dm` used to prove this through `nxc channels list`. The verb
    // is gone (6j6v.dvyq §3); the flag is not, and it has to survive the projection into
    // `ChannelView` rather than only exist on `is_degraded_dm`, because the view is what an app
    // renders. A third member arrives the way one really can — another client, or a mistake —
    // which is the whole reason the read is defensive about "exactly 2" (spec §3.3).
    let mut s = ChatStore::open_in_memory(1);
    s.set_channel_field("dm:ab", "kind", CHANNEL_KIND_DIRECT, ALICE);
    s.add_member("dm:ab", ALICE, ALICE);
    s.add_member("dm:ab", BOB, ALICE);

    let healthy = facade::channels(&s, ALICE).expect("channels reads");
    assert_eq!(healthy.len(), 1);
    assert_eq!(healthy[0].members, 2);
    assert!(!healthy[0].degraded, "exactly two is healthy: {healthy:?}");

    s.add_member("dm:ab", CAROL, ALICE);
    let degraded = facade::channels(&s, ALICE).expect("channels reads");
    assert_eq!(degraded[0].members, 3);
    assert!(
        degraded[0].degraded,
        "≠2 members on a direct channel is degraded: {degraded:?}"
    );
}

// ---- the reads that outlived `nxc inbox` / `nxc read` — and then did not (nxf 6j6v.4d2z) ------
//
// Two tests stood here, moved off `tests/verbs.rs` when nxf 6j6v.1gm9 removed the two verbs so
// their black-box coverage would not go with them: the inbox's disposition partition plus the
// cursor advance, and the ack's refusal ORDER (a SEP byte is `validation` before any lookup, an
// unknown channel `not_found` before membership is consulted, a non-member `forbidden`, and a
// `through` naming no message in the channel `not_found` — with the case where the last two apply
// at once proving it was the ORDER and not merely the set).
//
// **They are deleted rather than moved again, because their subject is gone.** nxf 6j6v.4d2z
// removed `facade::inbox`, `facade::mark_read`, `read_cursors` and the `PrimeReport` fields they
// fed — the unread apparatus entire. This file exists to refuse a coverage loss wearing the clothes
// of a cleanup, and the distinction it turns on is whether the BEHAVIOUR survived its entrance:
// in 6j6v.1gm9 it did, so the coverage moved here; here it did not, so there is nothing left to
// cover. What survives of the ack's refusal order is the gate the READS share, `require_readable`,
// exercised by `messages`/`thread` above and in `tests/public_channel.rs`.
