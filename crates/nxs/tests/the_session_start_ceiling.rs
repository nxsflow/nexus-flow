//! **The gate on the COMPOSED session start** (nxf 6j6v.xbnh).
//!
//! The host that runs the SessionStart hook truncates a large output. **Superseded 2026-08-28 (nxf
//! hrz7, task 1):** the 2026-08-27 single-probe measurement this paragraph used to cite (25.893
//! bytes pass through, 32.000 are filed away) was wrong by a factor of 2.5 — see
//! [`nxs::prime::SESSION_START_CEILING_BYTES`] for the corrected, telemetry-backed measurement
//! (10.164 B pass / 10.203 B filed, from 1.114 hook events across 947 transcripts) and the full
//! reasoning; it is not repeated here. Above the line the yield of `nxs prime` is not smaller but
//! NIL — and worse, because the session then reads files by hand and spends more context than
//! `prime` would have cost.
//!
//! **What makes this file necessary is where the previous gate sat.** `nxm`'s own budget
//! (nxf 6j6v.waq9) measured the memory half against 48 KiB and was green in every workspace that
//! was over the cliff, for two reasons the item names: it was calibrated against token cost rather
//! than the host's threshold, and it was anchored on ONE module while the cliff hits the composed
//! fan-out — `nxf` (9–11 KB), `nxc` and `nxm` land in the same context together. A per-module gate
//! is structurally unable to see that. This one runs the real `nxs prime` over a real workspace with
//! all three modules active and measures the bytes a session would actually be handed.
//!
//! **Superseded 2026-08-28 (nxf qhgw, task 6): this file measures PER HOOK OUTPUT now.** The
//! paragraph above was right, and its warning is worth keeping straight rather than deleting,
//! because what replaces it looks superficially like the mistake it was written against.
//!
//! What made 6j6v.waq9's gate wrong was not that it measured one module. It was that it measured
//! one module while the HOST's limit applied to something else — the whole session start, which
//! three modules shared. Measuring a part against a whole's budget is green exactly when it should
//! be red. The 2026-08-28 telemetry (1.114 SessionStart events, 947 transcripts) established that
//! the limit is per hook OUTPUT, at 10.240 B, and that three hooks of ~8 KB each arrive whole
//! (24.127 B together); nxf n2m6 + a2a1 then wired one hook per active module. So each module's
//! block is now, by itself, one host output with one budget — and measuring it alone is measuring
//! a whole against its own whole budget. Same arithmetic shape as the old bug, opposite meaning,
//! because the thing being truncated moved.
//!
//! The composed `nxs prime` is still measured here, but for what it now is: the size of the verb a
//! person runs BY HAND. No host output is that size any more, so it is no longer compared to the
//! ceiling.

use assert_cmd::Command;
use nxs::prime::{ceiling_warning, SESSION_START_CEILING_BYTES};
use nxs_test_support::PinHome;
use std::path::Path;
use tempfile::TempDir;

const NOW: &str = "2026-08-27T12:00:00Z";

/// The introduction length the gate assumes: the MAXIMUM a write can store
/// ([`nexus_memory::model::INTRODUCTION_MAX_CHARS`]). The point of measuring at the maximum is that
/// the result is then a bound and not a sample — no future memory can push the index past what this
/// test already paid for, because the write path refuses a longer line.
fn worst_case_introduction(i: usize) -> String {
    let head = format!("memory {i:03}: ");
    format!(
        "{head}{}",
        "x".repeat(nexus_memory::model::INTRODUCTION_MAX_CHARS - head.chars().count())
    )
}

fn bin(name: &str, dir: &Path) -> Command {
    let mut c = Command::cargo_bin(name).unwrap_or_else(|_| panic!("{name} binary built"));
    // Never this machine's real `~` (nxf 6j6v.npf9): `nxs init` writes to the background service's
    // workspace registry, and an unpinned suite leaves a dead entry per case in the developer's
    // own `~/.nexusflow/workspaces.toml`.
    c.pin_home(nxs_test_support::pinned_home())
        .current_dir(dir)
        .env("NXF_ACTOR", "alice")
        .env("NXM_ACTOR", "alice")
        .env("NXC_ACTOR", "alice")
        .env("NXF_NOW", NOW)
        .env("NXM_NOW", NOW)
        .env("NXS_NOW", NOW)
        .env("NXF_DETERMINISTIC_IDS", "1")
        // This suite builds its own `nxc` command rather than going through
        // `nxs_test_support::cargo_bin`, so it does not inherit that helper's default: say which
        // timer it wants, or `nxc init` here would arm the real scheduler.
        .env("NXC_TIMER", "dry")
        // …and never a real model for a thread name (nxf 6j6v.e76c) — see `cargo_bin`, which this
        // suite deliberately does not go through.
        .env("NXC_NAMER", "dry");
    c
}

fn run(name: &str, dir: &Path, args: &[&str]) -> String {
    let out = bin(name, dir)
        .args(args)
        .assert()
        .success()
        .get_output()
        .clone();
    String::from_utf8(out.stdout).expect("utf8 stdout")
}

/// **What the host actually runs at session start, since nxf n2m6 + a2a1: one hook per active
/// module.** Each entry is one host output with its own [`SESSION_START_CEILING_BYTES`] budget.
///
/// This is the shape the gate measures. It is deliberately built by running the module binaries the
/// way the wired hooks do — `<binary> prime` — rather than by splitting the composed `nxs prime`
/// on its block headings: the composed verb and the three hooks are different processes producing
/// different bytes (the composed one can carry the ceiling warning, for instance), and a gate that
/// measured a reconstruction would be measuring something no session is ever handed.
fn hook_outputs(dir: &Path) -> Vec<(&'static str, String)> {
    ["nxf", "nxc", "nxm"]
        .into_iter()
        .map(|binary| (binary, run(binary, dir, &["prime"])))
        .collect()
}

/// The chat history the workspace that was actually over the cliff carried (nxf 6j6v.1gm9): **24
/// threads the caller opened, six of them complete**, each complete one replaying its whole
/// conversation — the request plus the reply that closed the quorum.
///
/// The bodies are sized from that measurement rather than invented: the six complete boards there
/// were 46.487 bytes between them, so ~7.7 KB per board, which is what whole work orders and whole
/// reports actually weigh. They must cost the session start NOTHING, and a board-sized regression
/// is what this catches. Written straight to the store, exactly as `chat`'s own goldens seed one:
/// what is under test is what `prime` RENDERS, so the traffic has to exist and does not have to be
/// typed.
fn a_chat_history_of_finished_commissions(dir: &Path) {
    use nexus_chat::model::{
        Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot,
    };
    use nexus_chat::workspace::{ChatWorkspaceExt, Workspace};

    let ws = Workspace::resolve(None, dir).expect("resolve workspace");
    // The OPENER has to be the handle `nxs prime` will run as, or the requester wake is empty and
    // this whole fixture measures nothing (it did, in the first draft: the counter-probe below
    // stayed green). `nxc` derives it as `<replica prefix>/<NXC_ACTOR>` — the workspace's own
    // answer since nxf 6j6v.07me, not the constant `local` — so it is read from the workspace here
    // rather than spelled.
    let origin = nexus_chat::workspace::origin_of(&ws).to_string();
    let mut store = ws.open_chat_store().expect("open chat store");
    store.set_wall_clock(NOW);
    let (opener, replier) = (format!("{origin}/alice"), format!("{origin}/bob"));
    let (opener, replier) = (opener.as_str(), replier.as_str());
    let channel = "decl:planning";
    store.set_channel_field(channel, "name", "planning", opener);
    store.set_channel_field(channel, "kind", "group", opener);
    store.set_channel_field(channel, "origin", "local", opener);
    store.add_member(channel, opener, opener);
    store.add_member(channel, replier, opener);

    let post =
        |store: &mut nexus_chat::store::ChatStore, thread: &str, sender: &str, body: &str| {
            store.post_message(&MessageEnvelope {
                origin: origin.clone(),
                channel_id: channel.into(),
                sender: sender.into(),
                kind: MessageKind::Report,
                priority: Priority::Normal,
                disposition: Disposition::InTurn,
                thread_id: Some(thread.into()),
                refs: Refs::default(),
                body: body.into(),
            });
        };
    for i in 0..24 {
        let thread = format!("m-thread-{i:03}");
        store.open_thread(
            &thread,
            &ThreadRoot {
                origin: origin.clone(),
                channel_id: channel.into(),
                opener: opener.into(),
                created: NOW.into(),
                parent: None,
            },
            opener,
        );
        store.set_expects_reply_from(&thread, &format!("[\"{replier}\"]"), opener);
        // The commission itself — a whole work order, with its own headings, which is what made
        // these show up in a session start as sections of their own.
        post(
            &mut store,
            &thread,
            opener,
            &format!("## Der Auftrag {i}\n\n{}", "order ".repeat(640)),
        );
        // Six of the 24 are answered, and an answered board is what the removed block replayed.
        if i % 4 == 0 {
            post(
                &mut store,
                &thread,
                replier,
                &format!("## Was ich getan habe {i}\n\n{}", "report ".repeat(560)),
            );
        }
    }
}

/// A workspace at the scale this project actually runs at: all three modules active, a board with
/// enough open items to fill `nxf next`, 55 memories — one more than the largest real workspace on
/// the machine this was measured on (manufakt-io, 53) — and the chat history above.
fn a_workspace_at_real_scale() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    run(
        "nxf",
        dir,
        &["init", "--quiet", "--plugin", "issue-tracker"],
    );
    run("nxm", dir, &["init"]);
    run("nxc", dir, &["init"]);

    for i in 0..20 {
        run(
            "nxf",
            dir,
            &[
                "create",
                "--type",
                "feature",
                "--title",
                &format!(
                    "An open item with a title of the length these titles actually have, no. {i}"
                ),
                "--description",
                "A description of the length these descriptions actually have, so the board block \
                 is the size a board block really is.",
                "--priority",
                "2",
            ],
        );
    }
    for i in 0..55 {
        // Bodies at the measured average (1.614 B per memory across 300 real memories) — they must
        // cost the session start NOTHING, and a body-sized regression is what this catches.
        run(
            "nxm",
            dir,
            &[
                "remember",
                &"body ".repeat(320),
                "--key",
                &format!("memory-{i:03}"),
                "--introduction",
                &worst_case_introduction(i),
                "--category",
                if i % 3 == 0 { "rules" } else { "architecture" },
            ],
        );
    }
    a_chat_history_of_finished_commissions(dir);
    tmp
}

/// **The gate: every module's hook output, each against its own ceiling** (nxf qhgw, task 6).
///
/// **Superseded 2026-08-28 (nxf qhgw, task 6), and the superseded text is kept because a reader has
/// to be able to tell these two states apart.** This test used to be a KNOWN, TRACKED GAP rather
/// than a passing check: it asserted that the COMPOSED `nxs prime` of a real-scale workspace was
/// ABOVE the host's cut-off, deliberately inverted by nxf hrz7 task 1 rather than deleted or
/// fudged, because task 1 corrected `SESSION_START_CEILING_BYTES` from a wrongly-believed 25 KiB to
/// its true 10.240 B without reworking what `prime` renders. Its note instructed whoever shrank the
/// render enough to flip the operator back to `<=` and restore the name
/// `the_composed_session_start_stays_under_the_hosts_cut_off`.
///
/// **That note is discharged here, and NOT by restoring that name — deliberately.** Between task 1
/// and this task, nxf n2m6 + a2a1 made the host run one hook per active module. The composed
/// `nxs prime` output is therefore no longer a host output at all: it is what a person gets when
/// they run the verb by hand, and nothing truncates it at 10.240 B. A test called
/// `the_composed_session_start_stays_under_the_hosts_cut_off` would assert a relationship between
/// two quantities that no longer meet — the exact "false map" the task-1 review objected to when it
/// found the inverted body under the un-inverted name. What the host truncates is each MODULE's
/// output, so that is what this measures and what the name says.
///
/// **Where it stands, measured (see `hook_outputs`).** All three clear the line, and all three are
/// asserted to — `OVER_THE_LINE` is empty:
///
/// - `nxf prime` — **3.453 B** at real scale, 1.799 B on an empty workspace. Under.
/// - `nxc prime` — **1.616 B**, unchanged by workspace size (its data sections cost nothing at
///   session start, which is what nxf 6j6v.1gm9 bought). Under. It read 1.008 B when nxf qhgw
///   measured it on 2026-08-28, and **that figure was already stale before nxf 6j6v.aqqa touched
///   anything** — see the paragraph below, which is the correction the review of PR #468 earned.
/// - `nxm prime` — **9.087 B** at real scale, 1.346 B on an empty workspace. Under, **since nxf
///   6j6v.5jm3**; it measured 13.528 B before.
///
/// **The memory entry is what this task closed, and the shape of the fix is worth keeping.** The
/// reason nxm was over was never the fixed prose (nxf q065 task 4 cut that to 1.346 B and it stayed
/// there). It was the INDEX: one line per memory, bounded by nothing, so this fixture's 55 memories
/// at the 200-character maximum a write can store made the index alone ~12 KB. It is bounded now,
/// the way `nxf prime` already bounds the board (`## Next (showing 15 of 153)`, leaving the rest to
/// `nxf show`) — `nexus_memory::facade::PRIME_BLOCK_BUDGET_BYTES`, with the cut named in the block
/// and the whole index served by `nxm index`.
///
/// So the entry above no longer sits at a ceiling it happens to clear: it sits at a budget the
/// renderer fills on purpose, which is why it is ~9.1 KB and not ~3 KB. That budget is 90 % of this
/// ceiling and is asserted against it directly
/// (`the_memory_blocks_own_budget_stays_under_the_hosts_cut_off`), because the two constants live
/// in crates that cannot see each other.
///
/// Counter-probed by removing the fix, TWICE, once per cliff this fixture carries. Both figures
/// predate the hook split and are COMPOSED sizes; they are kept as they were measured rather than
/// restated per hook, because what they demonstrate — that the two removed renderers put two
/// orders of magnitude of payload into a session start — does not depend on how it is split up.
///
/// - **Memory (2026-08-27, nxf 6j6v.xbnh):** with `render_memory_index` swapped back for the
///   full-text renderer, this workspace's `nxs prime` measures **101.350 bytes** against that day's
///   ceiling of 25.600 (superseded 2026-08-28, nxf hrz7 task 1 — the ceiling is 10.240 now, see
///   `SESSION_START_CEILING_BYTES`; 101.350 bytes goes red against either one) and this test goes
///   red on exactly that.
/// - **Chat (2026-08-27, nxf 6j6v.1gm9):** with `render_opener_wake` put back into
///   `PrimeReport::render_markdown`, it measures **71.695** — the 24 boards below, six of them
///   complete, replayed in full, which is the 46 KB that `watch-bundestag` was actually carrying.
///
/// **"Worst case" is a statement about the LINES, not about the workspace** (review of PR #381).
/// A workspace with more memories, a longer board or a busier chat block costs more than this
/// fixture, and nothing at run time refuses that — which is why `nxs prime` carries its own
/// tripwire (`the_composed_block_warns_near_the_cut_off_and_is_silent_well_below_it`) and why
/// `6j6v.hqq6` tracks the rest. An introduction that arrived from a sync peer without passing this
/// build's write path is bounded at render instead (`facade::render_memory_index`), so the
/// `memories × 200` arithmetic holds for foreign data too.
#[test]
fn each_modules_hook_output_stays_under_the_hosts_cut_off() {
    let tmp = a_workspace_at_real_scale();
    let outputs = hook_outputs(tmp.path());

    // The modules whose hook output is still ABOVE the line, and why — see the doc comment. A table
    // rather than an `if` so that the exception is a named list a reader can diff.
    //
    // **Empty since nxf 6j6v.5jm3**, which bounded the memory index — the one entry it ever held.
    // It is kept rather than deleted with its last entry: the inverted-assertion shape is how this
    // suite pins a known gap without either fudging the ceiling or dropping the coverage (task 1
    // did it for the ceiling constant, task 6 for the memory index), and an empty list says "no
    // module is over" in the same breath as showing the next author where to say otherwise.
    const OVER_THE_LINE: &[&str] = &[];

    for (binary, block) in &outputs {
        if OVER_THE_LINE.contains(binary) {
            assert!(
                block.len() > SESSION_START_CEILING_BYTES,
                "`{binary} prime` is {} bytes against a ceiling of \
                 {SESSION_START_CEILING_BYTES}; it was expected to still be ABOVE the line — see \
                 the paragraph in this test's doc comment that says why. If it is now at or under \
                 the line, the fix landed: remove {binary:?} from OVER_THE_LINE, delete that \
                 paragraph, and re-measure MEASURED below.\n{block}",
                block.len()
            );
        } else {
            assert!(
                block.len() <= SESSION_START_CEILING_BYTES,
                "`{binary} prime` is {} bytes and the host delivers at most \
                 {SESSION_START_CEILING_BYTES} per hook output — above that a session is handed \
                 NOTHING from this module, not less. Shrink what it renders; do not raise the \
                 ceiling, which is the host's and not ours.\n{block}",
                block.len()
            );
        }
    }

    // **And the SIZE ITSELF is pinned, per block, independent of the ceiling** (review of PR #381,
    // Test Quality #5). The ceiling comparison alone cannot catch an accidental change to a
    // render's size — it has headroom on two of the three blocks and is permanently failing one
    // way on the third. This band is what does. When it trips, re-measure and move it deliberately.
    //
    // **Re-set per block 2026-08-28 (nxf qhgw, task 6).** It used to be ONE band over the composed
    // fan-out (`MEASURED = 18_377 ± 600`, itself moved by tasks 2, 3 and 4 as each shrank its
    // module's fixed prose). One number over three modules could say that the total moved but never
    // which module moved it — tasks 2-4 each had to argue that from the size of their own cut. Per
    // block, the fixture names the module directly.
    //
    // MEASURED with `cargo test -p nxs --test the_session_start_ceiling -- --nocapture` against a
    // temporary `eprintln!` of each block's length, then removed — not derived from the composed
    // total or from any module's own cut.
    const MEASURED: &[(&str, usize)] = &[("nxf", 3_453), ("nxc", 1_616), ("nxm", 9_087)];
    // Unchanged from the composed band, and deliberately NOT widened per block even though each
    // band now covers less: a wider band is exactly what would have hidden the 939 B and 1.218 B
    // shifts that tasks 3 and 4 recorded on purpose.
    const TOLERANCE: usize = 600;
    for (binary, expected) in MEASURED {
        let block = &outputs
            .iter()
            .find(|(b, _)| b == binary)
            .unwrap_or_else(|| panic!("{binary} is one of the wired hooks"))
            .1;
        assert!(
            block.len().abs_diff(*expected) <= TOLERANCE,
            "`{binary} prime` measured {} bytes; it was {expected} ± {TOLERANCE} when this was \
             written. Growing is what eats the headroom before the ceiling ever notices — \
             re-measure, decide whether the growth is worth it, and move the constant on purpose.",
            block.len()
        );
    }

    // The composed verb still has to carry all three blocks — it is what a person runs by hand, and
    // what a persona's prompt is composed from. Measured for CONTENT here, not against the ceiling:
    // no host output is this size any more.
    let composed = run("nxs", tmp.path(), &["prime"]);
    for module in ["# nexus-flow", "# nexus-chat", "# nexus-memory"] {
        assert!(
            composed.contains(module),
            "{module} is in the fan-out:\n{composed}"
        );
    }
    // And no memory body is, in ANY of the four outputs: 55 × ~1.6 KB is the 90 % of the payload
    // this item removed, and it must stay out of the per-hook path as well as the composed one.
    for (label, block) in std::iter::once(("nxs", composed.clone())).chain(outputs.clone()) {
        assert!(
            !block.contains("body body body"),
            "no memory body reaches `{label} prime`:\n{block}"
        );
        // …nor any MESSAGE body, which is the other half of the same payload (nxf 6j6v.1gm9): 24
        // boards with six finished commissions in them, and not one of their words reaches a
        // session.
        for gone in [
            "Threads you opened",
            "Der Auftrag",
            "order order",
            "Was ich getan habe",
        ] {
            assert!(
                !block.contains(gone),
                "no message body reaches `{label} prime`, and {gone:?} does:\n{block}"
            );
        }
    }
}

/// **The one place that can see both constants** (nxf 6j6v.5jm3).
///
/// The host's cut-off lives in `nxs` and the budget the memory block renders itself within lives in
/// `nexus-memory`, and that split is not an accident: `nxs` depends on the module crates, so a
/// module cannot name the umbrella's constant without inverting the dependency. What it can do is
/// carry its own number and be held to the relationship — here, in the crate that links both.
///
/// The relationship is `<`, not `<=`, and the margin is the point. `nxm prime`'s block is FILLED to
/// its budget on purpose (that is what makes the index as long as it can be rather than
/// arbitrarily short), so a budget equal to the ceiling would put every large workspace's output
/// flush against an edge that was inferred from a 39-byte band rather than read off a spec — and
/// leave nothing for whatever the host puts around a hook's stdout.
///
/// This is deliberately NOT a check that the two numbers stand in some exact ratio. What matters is
/// that the module's self-imposed bound cannot drift above the host's real one; how much room it
/// leaves below is the module's own call, argued at
/// [`nexus_memory::facade::PRIME_BLOCK_BUDGET_BYTES`].
#[test]
// Both sides ARE compile-time constants, and that is the point rather than a mistake: what this
// pins is a relationship between two `const`s that live in crates which cannot see each other, so
// nothing but a third crate linking both can state it at all. `assertions_on_constants` exists to
// catch `assert!(true)`; this is the case it cannot tell apart from one.
#[allow(clippy::assertions_on_constants)]
fn the_memory_blocks_own_budget_stays_under_the_hosts_cut_off() {
    assert!(
        nexus_memory::facade::PRIME_BLOCK_BUDGET_BYTES < SESSION_START_CEILING_BYTES,
        "`nxm prime` renders itself within {} bytes while the host delivers at most \
         {SESSION_START_CEILING_BYTES} — a module that fills a budget bigger than the ceiling \
         fills it with nothing, because above the line a session is handed NOTHING from that \
         module. Lower the budget; the ceiling is the host's and not ours.",
        nexus_memory::facade::PRIME_BLOCK_BUDGET_BYTES,
    );
}

/// **The gate discriminates per BLOCK, not on the total** (nxf qhgw, task 6, Definition of Done).
///
/// The two cases the DoD names, as fixtures over the pure comparison the gate makes, so the
/// property is pinned without building three real workspaces to provoke it:
///
/// - one block at 10.300 B fails, even though nothing else is near the line;
/// - three blocks totalling 24 KB pass, as long as each one is under 10 KiB — which is precisely
///   the case the OLD composed gate got wrong, and the whole reason the hook was split.
#[test]
fn the_ceiling_is_applied_per_block_so_24_kb_of_small_blocks_passes_and_one_big_block_does_not() {
    // The comparison the gate makes, extracted so both DoD cases can be stated as data.
    let over = |block_bytes: usize| block_bytes > SESSION_START_CEILING_BYTES;

    // A single module block at 10.300 B — 60 B past the line — is a failure on its own.
    assert!(
        over(10_300),
        "one block of 10.300 B is over the 10.240 B per-hook ceiling and must fall through"
    );

    // Three blocks of ~8 KB each: 24.000 B together, none of them over. This is the measured
    // reality behind the split (three ~8 KB hooks arrived whole, 24.127 B) and it must PASS.
    let three = [8_000usize, 8_000, 8_000];
    assert_eq!(three.iter().sum::<usize>(), 24_000, "the fixture is 24 KB");
    assert!(
        !three.iter().copied().any(over),
        "three blocks under 10 KiB each pass however large their total — the composed sum is not a \
         host output and is not what the ceiling governs"
    );

    // …and the boundary itself, so "under" is not off by one in either direction.
    assert!(
        !over(SESSION_START_CEILING_BYTES),
        "exactly at the ceiling still arrives"
    );
    assert!(
        over(SESSION_START_CEILING_BYTES + 1),
        "one byte past it does not"
    );
}

/// **The live tripwire**, and the reason it is not only a CI gate (review of PR #381, Integrity #2).
///
/// The gate above measures ONE synthetic workspace at build time. A real one keeps growing after
/// that, and nxf 6j6v.waq9's per-write budget — the only thing that used to watch the size at run
/// time — was removed together with the full-text channel it governed. So a workspace can walk back
/// over the cliff with nothing live to notice, which is precisely the failure this item exists to
/// end. `nxs prime` therefore weighs its own composed output and says so before it crosses.
///
/// Asserted on both sides of the threshold, because a warning that always fires is noise and one
/// that never fires is decoration.
///
/// **Superseded 2026-08-28 (nxf hrz7, task 1), and superseded AGAIN the same day (nxf hrz7, task
/// 2).** Task 1's paragraph (kept below, itself marked superseded rather than deleted) said the
/// "silent" half no longer had a real fixture to stand on: a freshly initialized, empty workspace
/// measured 10.747 B, already past the corrected 10.240 B ceiling, so even the emptiest workspace
/// warned. That was true against `nxf prime`'s PRE-task-2 render. Task 2 shrank `nxf prime`'s fixed
/// prose from 5.570 B to 1.635 B (a ~3.9 KB cut that applies whether or not the board holds any
/// items — it is the STATIC prose, not the data sections). The quiet fixture below is a bare,
/// empty workspace, so it eats that full cut — MEASURED directly (printed `block.len()` under
/// `--nocapture`, not computed from the cut alone, because the composed total also drops the
/// ~390-byte warning banner itself once it stops firing): it now measures **6.310 B**, back under
/// 90 % of the ceiling (9.216 B) — the "silent" case is demonstrable with a real fixture again,
/// and the assertion below is flipped back to expecting silence. The "LOUD" half is UNCHANGED by
/// this: a workspace at real scale (`a_workspace_at_real_scale`) carries real
/// `next`/`blocked`/`recently closed` content nxf 2's brief explicitly left untouched, so it
/// still measures ~20.5 KB (was 24.469 B; the composed band that pinned it is gone — see the
/// task-6 paragraph below) — still far past the ceiling, so
/// it still warns. Reworking `prime` enough to clear the ceiling ITSELF for a workspace at real
/// scale remains later work (nxf hrz7's task 6), not task 2's — this paragraph is only about the
/// WARNING threshold on the EMPTY fixture, a narrower claim the task-2 brief did not anticipate
/// but that fell out of its own honest re-measurement.
///
/// **Superseded again 2026-08-28 (nxf h4d3, task 3).** Task 3 cut `nxc prime`'s fixed prose from
/// 1.938 B to 720 B (a 1.218 B fall — see `crates/chat/src/facade.rs`'s
/// `PrimeReport::render_markdown`); the quiet fixture below eats that cut too, and now measures
/// 5.092 B, MEASURED the same way task 2's 6.310 B was. Still well under 90 % of the ceiling
/// (9.216 B), so the assertion below is unchanged — it was already expecting silence, and a
/// smaller quiet fixture only makes that easier to hold. The "LOUD" half is unaffected by this
/// paragraph specifically (it moved from 20.534 B to 19.316 B for its own task-3 reason; the
/// composed band that pinned those numbers is gone — see the task-6 paragraph below) — still far
/// past the ceiling, so it still warns.
///
/// **Superseded again 2026-08-28 (nxf q065, task 4).** Task 4 cut `nxm prime`'s fixed prose from
/// 2.285 B to 1.346 B (a 939 B fall — see `crates/memory/src/facade.rs`'s
/// `PrimeReport::render_markdown`); the quiet fixture below eats that cut too, and now measures
/// **4.153 B**, MEASURED the same way (a temporary `eprintln!("{}", block.len())` run under
/// `--nocapture`, then removed). Still well under 90 % of the ceiling (9.216 B), so the assertion
/// below is unchanged. The "LOUD" half is unaffected by this paragraph specifically (it moved from
/// 19.316 B to 18.377 B for its own task-4 reason) — still far past the ceiling, so it still warns.
///
/// **Superseded again 2026-08-28 (nxf qhgw, task 6): the composed band those two paragraphs cite
/// no longer exists, and this test's subject changed under it.** Tasks 3 and 4 above point at a
/// `MEASURED`/`TOLERANCE` pair "above" that pinned the COMPOSED fan-out at 18.377 B ± 600. Task 6
/// replaced it with a per-block table in
/// [`each_modules_hook_output_stays_under_the_hosts_cut_off`], so those citations are answered here
/// rather than left pointing at a constant that now holds different numbers: the composed figures
/// they record (24.469 → 20.534 → 19.316 → 18.377 B) were correct when written and are kept for the
/// history of the cuts, but nothing pins the composed size any more, because no host output is that
/// size.
///
/// **What this test still measures, and why it is still worth having.** `ceiling_warning` is
/// computed by `nxs prime` over its COMPOSED output — that has not changed, and after the hook
/// split the composed output is the verb a person runs by hand, not what the host truncates. So
/// this is no longer a tripwire on the session start; it is a tripwire on the manual verb, and its
/// "LOUD" half fires at real scale because the composed size genuinely is past 10.240 B (18.377 B)
/// even though every individual hook output the host receives is a separate, smaller thing. Giving
/// each module's own `prime` its own tripwire is the honest follow-up and belongs to no task on
/// this branch; the doc comment on `ceiling_warning` in `crates/nxs/src/prime.rs` records the same
/// gap from the other side.
///
/// **Superseded 2026-08-28 (nxf hrz7, task 1).** This used to say the real-scale fixture sits at
/// 24.081 bytes against a 25.600 ceiling, ~94 %, so a workspace of this project's own size is
/// already inside the window where it should be told — which was true against the ceiling in place
/// when it was written. With [`SESSION_START_CEILING_BYTES`] corrected to its true 10.240 B, the
/// same 24.081 bytes is no longer *inside the window*, it is past the CLIFF itself (~235 % of the
/// ceiling) — the fixture the "LOUD" half below exercises is not a near-miss any more, it is the
/// exact failure this whole item exists to name. And the "silent" half below no longer has a real
/// fixture to stand on at all: a freshly initialized, empty workspace already measures 10.747 B —
/// see `CEILING_WARNING_FRACTION`'s doc comment in `crates/nxs/src/prime.rs` — so it warns too now.
/// "Silent below the threshold" is demonstrated with the pure function directly instead (below,
/// alongside the other arithmetic assertions), which is the only place left where a genuinely small
/// composed size can still be produced on demand.
#[test]
fn the_composed_block_warns_near_the_cut_off_and_is_silent_well_below_it() {
    // **Silent again — nxf hrz7, task 2.** Task 1 (see the doc comment above) found that even an
    // empty workspace warned, because `nxf prime`'s fixed prose alone (10.747 B, pre-task-2)
    // already exceeded the corrected 10.240 B ceiling. Task 2 cut that fixed prose to 1.635 B (see
    // `crates/cli/src/commands/mod.rs::prime`'s human-view rendering); a bare, empty workspace has
    // no `next`/`blocked`/`recently_closed` content to offset that cut, so it then measured
    // 6.310 B (MEASURED via a temporary `eprintln!("{}", block.len())` run under `--nocapture`,
    // not derived from the cut alone — the composed total also drops the ~390-byte warning banner
    // itself once it stops firing) — back under 90 % of the corrected ceiling (9.216 B) — and this
    // assertion is flipped back to expecting silence. This does NOT mean the composed ceiling
    // itself is cleared: the "LOUD" half just below, run against a workspace at real scale, still
    // warns (task 6 is what closes that gap, not task 2).
    //
    // **Superseded 2026-08-28 (nxf qhgw, task 6): task 6 did NOT close that gap, and could not.**
    // The sentence above expected task 6 to shrink the composed render under the ceiling. What task
    // 6 actually established — after nxf n2m6 + a2a1 split the hook — is that the composed render
    // is not measured against the ceiling at all any more, because it is no longer a host output.
    // The "LOUD" half below still warns, and now that is correct rather than a pending defect: an
    // 18.377 B composed output IS past the point where the host would cut a single hook, which is
    // exactly what a person running `nxs prime` by hand should be told.
    //
    // **Superseded again 2026-08-28 (nxf h4d3, task 3).** `nxc prime`'s own fixed-prose cut (see
    // `PrimeReport::render_markdown`'s doc comment in `crates/chat/src/facade.rs`) moved this
    // fixture's measurement again, to 5.092 B — MEASURED the same way, same temporary
    // `eprintln!` + `--nocapture`. Still well under the 9.216 B silence threshold, so the assertion
    // below needed no change.
    //
    // **Superseded again 2026-08-28 (nxf q065, task 4).** `nxm prime`'s own fixed-prose cut (see
    // `PrimeReport::render_markdown`'s doc comment in `crates/memory/src/facade.rs`) moved this
    // fixture's measurement again, to **4.153 B** — MEASURED the same way, same temporary
    // `eprintln!` + `--nocapture`. Still well under the 9.216 B silence threshold, so the assertion
    // below needed no change.
    //
    // **Unchanged by nxf qhgw, task 6.** The split gave each module its own host output but did not
    // touch what any of them RENDERS, so this composed figure is still 4.153 B. Its three parts,
    // measured the same way for the per-block table in the gate above: `nxf` 1.799 B, `nxc`
    // 1.008 B, `nxm` 1.346 B — each far under the ceiling on an empty workspace, which is the
    // "silent" case this half exists to demonstrate.
    //
    // **Superseded 2026-09-12 (nxf 6j6v.aqqa).** `nxc prime`'s cheatsheet grew by one line and one
    // paragraph, so this fixture measures **4.761 B** — MEASURED the same way, same temporary
    // `eprintln!` + `--nocapture`, then removed. Its parts: `nxf` 1.799 B (unchanged), `nxc`
    // **1.616 B**, `nxm` 1.346 B (unchanged). Still well under the 9.216 B silence threshold, so
    // the assertion below needed no change; the per-block band in the gate above DID, and was
    // moved rather than left to ride inside its tolerance.
    //
    // **And the first draft of this paragraph got the ATTRIBUTION wrong, which is worth keeping**
    // (review of PR #468, Code Quality #2 / Test Quality #1, Medium). It read the 608 B between the
    // old band and the new measurement as the cost of 6j6v.aqqa. It is not. This change's own
    // contribution is **429 B**, measured by rendering the block through the real binary twice in
    // ONE fixed workspace, once with the old constant and once with the new (1.415 B -> 1.844 B
    // there; those absolutes differ from this fixture's because the Declarations section prints the
    // workspace path, which is why the DELTA is the transferable number and the absolutes are not).
    // The other ~179 B was already in the block: nxf 6j6v.9w08 added the `writing-declarations`
    // pointer to it on 2026-09-12 and did not move the band. So 1.616 - 429 = 1.187 B is where the
    // band should have stood before this change ever arrived.
    //
    // The lesson is the one this file already states and the first draft still managed to break:
    // a number that is DERIVED from two others is not MEASURED, however arithmetically sound it
    // looks. The 600 B tolerance would have swallowed the error silently — a reader caught it.
    let quiet = TempDir::new().unwrap();
    run(
        "nxf",
        quiet.path(),
        &["init", "--quiet", "--plugin", "issue-tracker"],
    );
    run("nxm", quiet.path(), &["init"]);
    run("nxc", quiet.path(), &["init"]);
    let block = run("nxs", quiet.path(), &["prime"]);
    assert!(
        !block.contains("the host stops delivering one"),
        "an empty workspace's composed size (nxf prime's fixed prose alone) dropped back under \
         90% of the corrected ceiling once nxf hrz7 task 2 shrank nxf prime's static prose — if \
         this now fails, the render grew again (or task 6 changed nxc/nxm's own fixed prose): \
         re-measure and decide whether to flip this back:\n{block}"
    );

    // Loud for the real-scale one. This is the review's measurement as an assertion: the worst case
    // of a workspace the size of this project's own is already past 90 % of the ceiling.
    let crowded = a_workspace_at_real_scale();
    let block = run("nxs", crowded.path(), &["prime"]);
    assert!(
        block.contains("the host stops delivering one") && block.contains("nxm forget <key>"),
        "…and a workspace at real scale IS told, with a way to shrink it:\n{block}"
    );
    // The number it reports is the size of the block WITHOUT the warning — a warning that counted
    // itself would report a figure no other measurement agrees with. Pinned as a band rather than
    // an equality, because the exact width of the warning is prose and prose gets edited.
    let reported: usize = block
        .split("This session start is ")
        .nth(1)
        .and_then(|t| t.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("the warning names a byte count:\n{block}"));
    assert!(
        reported < block.len() && reported + 600 > block.len(),
        "the reported {reported} is the composed size, just under the printed {} — it does not \
         count itself, and it is not some other quantity",
        block.len()
    );

    // **Genuinely silent, well below** — the case no real fixture can produce any more now that the
    // fixed floor itself is past the ceiling (nxf hrz7, task 1; see the doc comment above). The pure
    // function still has to be silent for a SMALL composed size, and this is the only place left
    // that can demonstrate it.
    assert!(
        ceiling_warning(100).is_none(),
        "a composed size far below the ceiling gets no warning"
    );

    // The threshold itself, stated as arithmetic rather than by growing a fixture until it trips:
    // the pure function the command calls is what decides, so it is what is pinned.
    let threshold = SESSION_START_CEILING_BYTES * 9 / 10;
    assert!(
        ceiling_warning(threshold).is_none(),
        "at 90 % exactly there is still room, so nothing is said"
    );
    let warning = ceiling_warning(threshold + 1).expect("a byte past 90 % warns");
    assert!(
        warning.contains(&(threshold + 1).to_string())
            && warning.contains(&SESSION_START_CEILING_BYTES.to_string()),
        "it names the measurement AND the ceiling, or a reader cannot tell how much room is \
         left: {warning}"
    );
    // Above the cliff it still fires — the host will not deliver it, but a person running
    // `nxs prime` by hand is exactly who needs to see it then.
    assert!(ceiling_warning(SESSION_START_CEILING_BYTES * 2).is_some());
}

/// **The order is a safety property, not cosmetics** (6j6v.xbnh): `nxf` → `nxc` → `nxm`.
///
/// If the host truncates anyway — a workspace bigger than any measured here, a host with a lower
/// cut-off — what falls off the end has to be the block a session can fetch afterwards. `nxm recall`
/// and `nxm memories` can get a memory back; nothing gets the board, the core rules or the command
/// vocabulary back, because they are written down nowhere else. So memory goes last, and the worst
/// case becomes benign instead of catastrophic.
#[test]
fn the_fan_out_puts_the_recoverable_block_last() {
    let tmp = a_workspace_at_real_scale();
    let block = run("nxs", tmp.path(), &["prime"]);
    let at = |needle: &str| {
        block
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} is in the fan-out:\n{block}"))
    };
    let (flow, chat, memory) = (at("# nexus-flow"), at("# nexus-chat"), at("# nexus-memory"));
    assert!(
        flow < chat && chat < memory,
        "flow, then chat, then memory — memory is the only one a session can fetch back: \
         {flow}/{chat}/{memory}\n{block}"
    );
}

/// **The two module-order tables must not drift** (review of PR #381, Code Quality #1 / Integrity
/// #5, found independently by both reviewers).
///
/// `ModuleInit::order` sequences the `nxs prime` fan-out; `nexus_chat::facade::PRIME_MODULE_ORDER`
/// sequences a persona's composed prompt. They say the same thing and are maintained by hand,
/// because chat does not depend on the registry crate and so cannot read the ordinals. This item
/// had to hand-sync them, and nothing would have caught a half-done edit — the fan-out would take
/// one order and a spawned persona the other, which is precisely the kind of divergence nobody
/// notices until a truncated session start eats the wrong block.
///
/// This crate links both, so this is the one place the comparison can be made at all.
#[test]
fn the_two_module_order_tables_agree() {
    let roster: Vec<&str> = nxs::registry::roster().iter().map(|m| m.key).collect();
    assert_eq!(
        roster,
        nexus_chat::facade::PRIME_MODULE_ORDER,
        "the fan-out order and the persona-prompt order have diverged — change both, or a \
         persona reads its blocks in a different sequence from every other session"
    );
    // …and it really is the safety order, not just two copies of the same accident.
    assert_eq!(
        roster.last(),
        Some(&"memory"),
        "memory goes last: it is the only block a session can fetch back afterwards"
    );
}

/// The order does NOT depend on which module a workspace happened to initialize first.
///
/// It used to: the fan-out ran in `active_modules` order, which is the order `init` appended keys to
/// `config.toml`. A safety property that varies per workspace is not a safety property, and every
/// workspace written before chat existed lists memory ahead of it.
#[test]
fn a_workspace_that_added_memory_first_still_fans_out_in_the_same_order() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    run(
        "nxf",
        dir,
        &["init", "--quiet", "--plugin", "issue-tracker"],
    );
    run("nxm", dir, &["init"]);
    run("nxc", dir, &["init"]);

    // Rewrite `active_modules` into the order a workspace whose memory was set up first carries.
    // Done on the file rather than by re-ordering the `init` calls because `init` refuses to run
    // inside an existing workspace — and it is the FILE that the fan-out used to read its order
    // from, so this is the state under test rather than a rehearsal of it.
    let path = dir.join(".nxs/config.toml");
    let config = std::fs::read_to_string(&path).unwrap();
    let reordered = config.replace(
        r#"active_modules = ["flow", "memory", "chat"]"#,
        r#"active_modules = ["memory", "chat", "flow"]"#,
    );
    assert_ne!(
        reordered, config,
        "the premise: the order really is in the file:\n{config}"
    );
    std::fs::write(&path, &reordered).unwrap();

    let block = run("nxs", dir, &["prime"]);
    let at = |needle: &str| block.find(needle).expect(needle);
    assert!(
        at("# nexus-flow") < at("# nexus-chat") && at("# nexus-chat") < at("# nexus-memory"),
        "the fan-out order is the roster's, not the workspace's own history:\n{block}"
    );
}
