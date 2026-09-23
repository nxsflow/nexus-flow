//! **What a session is handed at its start, and what it costs** (nxf 6j6v.xbnh).
//!
//! It replaces `the_session_start_budget.rs`, whose subject — the two-class replay of nxf 6j6v.waq9
//! and the byte budget on its always-present class — this item removed. That cut went the right way
//! and measurably did not clear the wall it was aimed at:
//!
//! ```text
//! 25.893 B   of hook output the host passes through unchanged
//! 32.000 B   the host files away, delivering a 2 KB preview and a path instead
//! ```
//!
//! Above that line the yield of `prime` is not smaller but NIL — and worse than nil, because the
//! session then reads the files by hand. Projected onto the real workspaces, the two-class split
//! left nexus-flow at ~61 KB and manufakt-io at ~47 KB while both were comfortably INSIDE the
//! budget it declared, because that budget was calibrated against token cost rather than against
//! the host's threshold.
//!
//! So there is no full-text channel any more. Every memory is one WRITTEN line, and this file pins
//! the four promises that follow:
//!
//! 1. the line is mandatory and bounded at the WRITE, with the measured length named;
//! 2. the block is an index — `## Memories (N)`, a sentence, one line per memory, no bodies;
//! 3. a memory with no line yet renders a NAMED gap, never a derived stand-in;
//! 4. the obligation on a `rules` line is where an agent reads it before writing.
//!
//! What the host truncates is measured against the ceiling in
//! `crates/nxs/tests/the_session_start_ceiling.rs`. This half could never see it.
//!
//! **Superseded 2026-08-29 (nxf 6j6v.5jm3): promise 2's `## Memories (N)` is one of two headings
//! now.** "One line per memory, no bodies" is unchanged and is still what this file pins; what
//! changed is that the block renders itself within a byte budget, so past it the heading reads
//! `## Memories (showing 34 of 80)` and a note under the index names `nxm index` for the rest. The
//! reason is the same one the paragraphs above are about, one turn further on: the fixed prose was
//! cut to its target and the INDEX went on growing a line per memory, which is the shape that puts
//! a workspace over the cliff now. The cases here work at sizes where nothing is cut, deliberately
//! — the bound's own arithmetic is pinned in `facade`'s tests, next to the constant.
//!
//! **Superseded 2026-08-28 (nxf n2m6 + a2a1 / qhgw).** That sentence used to name "the composed
//! `nxs prime` output — the thing the host actually truncates", which was exact while the
//! SessionStart hook was a single `nxs prime`. The host truncates per hook OUTPUT (10.240 B), and
//! the wiring is now one hook per active module, so what is measured there is each module's own
//! `prime` — `nxm prime` among them. The composed output is still produced and still tested, as
//! the verb a person runs by hand.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn nxm(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", "2026-06-20T10:00:00Z")
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    tmp
}

fn remember(dir: &Path, key: &str, body: &str, introduction: &str) {
    nxm(dir)
        .args([
            "remember",
            body,
            "--key",
            key,
            "--introduction",
            introduction,
        ])
        .assert()
        .success();
}

fn prime(dir: &Path) -> String {
    let out = nxm(dir)
        .arg("prime")
        .assert()
        .success()
        .get_output()
        .clone();
    String::from_utf8(out.stdout).expect("utf8")
}

fn stderr_of(dir: &Path, args: &[&str]) -> String {
    let out = nxm(dir).args(args).assert().failure().get_output().clone();
    String::from_utf8_lossy(&out.stderr).to_string()
}

/// **Promise 1a: a memory cannot be written without the line it is replayed by.**
///
/// The refusal is at the WRITE and not at the render, because the write is the one moment somebody
/// is deciding what the memory says. A memory stored without a line would reach every future
/// session as a placeholder, and nobody would ever be in a position to notice.
#[test]
fn a_memory_written_without_an_introduction_is_refused_by_the_verb_itself() {
    let tmp = workspace();
    // clap refuses it before anything is opened — the flag is required, not merely validated.
    let err = stderr_of(tmp.path(), &["remember", "a fact", "--key", "k"]);
    assert!(
        err.contains("--introduction"),
        "the refusal names the missing flag: {err}"
    );
    assert!(
        nxm(tmp.path())
            .args(["--json", "recall", "k"])
            .assert()
            .failure()
            .get_output()
            .status
            .code()
            != Some(0),
        "and nothing was written"
    );

    // …and an UPDATE is not exempt: the body changed, so whether the line still describes it is
    // exactly the question, and carrying the old one forward silently is how an index stops
    // matching what it indexes.
    remember(tmp.path(), "k", "a fact", "one line about k");
    let err = stderr_of(tmp.path(), &["remember", "a different fact", "--key", "k"]);
    assert!(
        err.contains("--introduction"),
        "an update restates the line too: {err}"
    );
}

/// **Promise 1b: over 200 characters is refused at the write, and the message names the MEASURED
/// length.** "Too long" with no number leaves the writer counting characters by hand.
#[test]
fn an_over_long_introduction_is_refused_with_its_measured_length() {
    let tmp = workspace();
    let too_long = "x".repeat(201);
    let err = stderr_of(
        tmp.path(),
        &[
            "remember",
            "a fact",
            "--key",
            "k",
            "--introduction",
            &too_long,
        ],
    );
    assert!(
        err.contains("201") && err.contains("200"),
        "the refusal names the measured length and the limit: {err}"
    );
    // Exactly at the limit is fine — the boundary is stated by arithmetic, not by a wide margin.
    let at_limit = "y".repeat(200);
    remember(tmp.path(), "k", "a fact", &at_limit);

    // Counted in CHARACTERS, not bytes: an umlaut is not two thirds of an allowance.
    let umlauts = "ü".repeat(200);
    remember(tmp.path(), "u", "another fact", &umlauts);

    // And a line break is refused too — one memory is one bullet, whatever its author types.
    let err = stderr_of(
        tmp.path(),
        &[
            "remember",
            "a fact",
            "--key",
            "two-liner",
            "--introduction",
            "first line\nsecond line",
        ],
    );
    assert!(err.contains("ONE line"), "{err}");
}

/// **Promise 2: the block is an INDEX.** `## Memories (N)`, one explanatory sentence, then exactly
/// one line per memory — and no body anywhere.
#[test]
fn the_block_is_one_written_line_per_memory_and_no_bodies_at_all() {
    let tmp = workspace();
    remember(
        tmp.path(),
        "no-concurrent-lean-build-during-tests",
        "Never run `cargo build -p nxs --no-default-features` at the same time as `cargo test`. \
         The lean build overwrites target/debug/nxs with an mcp-less binary and the suite asserts \
         against it. THE TAIL THAT ONLY RECALL SHOWS.",
        "Never run the lean `nxs` build while `cargo test` is running — it swaps the binary out.",
    );
    remember(
        tmp.path(),
        "slow-tests-check-target-deps-and-swap",
        "A full `cargo test` taking over an hour with each binary at ~0 CPU is disk wait. Check \
         target/ dependencies and swap pressure first.",
        "A `cargo test` that crawls with the CPUs idle is I/O: check target/ deps and swap first.",
    );

    let text = prime(tmp.path());
    assert!(text.contains("## Memories (2)"), "{text}");
    assert!(
        text.contains("**One line per memory — this is an index, not the memories.**"),
        "the sentence that says what this is:\n{text}"
    );
    assert!(
        text.contains(
            "- **no-concurrent-lean-build-during-tests**: Never run the lean `nxs` build while \
             `cargo test` is running — it swaps the binary out."
        ),
        "one line, exactly as written:\n{text}"
    );
    assert!(
        !text.contains("THE TAIL THAT ONLY RECALL SHOWS") && !text.contains("disk wait"),
        "…and NO body reaches the session start — the whole saving:\n{text}"
    );
    // The saving as a proportion rather than an anecdote: the block's size is a function of the
    // memory COUNT, not of what anyone wrote.
    let short = workspace();
    let long = workspace();
    for i in 0..10 {
        let key = format!("m{i}");
        remember(short.path(), &key, "x", "a short line");
        remember(long.path(), &key, &"x".repeat(4_000), "a short line");
    }
    assert_eq!(
        prime(short.path()).len(),
        prime(long.path()).len(),
        "40 KB of bodies cost the session start exactly nothing"
    );
}

/// **Promise 2b: the saving is a RENDERING decision, and the data is untouched.** A host reading
/// `nxm prime --json` still gets every memory in full, so an embedding surface can render its own
/// bootstrap from the same report.
#[test]
fn the_json_projection_still_carries_every_body_in_full() {
    let tmp = workspace();
    let body = format!(
        "How a release is cut, step by step. {}\nAND THE TAIL THAT ONLY RECALL SHOWS",
        "detail ".repeat(40)
    );
    remember(
        tmp.path(),
        "a-recipe",
        &body,
        "How a release is cut, step by step.",
    );

    let out = nxm(tmp.path())
        .args(["--json", "prime"])
        .assert()
        .success()
        .get_output()
        .clone();
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).expect("valid json");
    assert_eq!(v["count"], 1, "{v}");
    assert_eq!(v["memories"][0]["body"], body, "{v}");
    assert_eq!(
        v["memories"][0]["introduction"], "How a release is cut, step by step.",
        "{v}"
    );

    let block = prime(tmp.path());
    assert!(block.contains("- **a-recipe**: How a release is cut, step by step."));
    assert!(
        !block.contains("AND THE TAIL THAT ONLY RECALL SHOWS"),
        "…and only the index line:\n{block}"
    );
}

/// **Promise 3: a memory with no line yet renders a NAMED gap.**
///
/// This is the one question the item left open at hand-off, and the answer is the constraint it
/// stated: the fallback must make the lack VISIBLE. The mechanism it replaces derived the line from
/// the body's first line and cut it at 110 characters; on real data that rendered ``- `key` — -…``
/// for the memories an agent had written, and agents write nearly all of them. A fallback that
/// renders empty is worse than one that renders a gap, because it looks like an answer.
///
/// Reached the way the migration window actually reaches it: an `introduction` op the view cannot
/// hold, i.e. exactly what a memory written before the register looks like.
#[test]
fn a_memory_with_no_introduction_reads_as_a_named_gap_and_is_counted() {
    let tmp = workspace();
    remember(
        tmp.path(),
        "older-than-the-register",
        "a fact from before",
        "x",
    );
    remember(
        tmp.path(),
        "written-since",
        "a newer fact",
        "the newer line",
    );

    // Roll the first memory back to what a pre-6j6v.xbnh workspace holds: the body op, no
    // introduction. Deleting the register op is what the previous release's log genuinely lacks —
    // and fabricating that log means stepping around the append-only guard (6j6v.hehx), which
    // refuses every deletion. Dropped here, in a test, for this one statement; the next open
    // installs it again.
    let db = tmp.path().join(".nxs/db.sqlite");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("DROP TRIGGER ops_append_only_no_delete;")
        .unwrap();
    conn.execute(
        "DELETE FROM ops WHERE field='introduction' AND target_id='older-than-the-register'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE memories SET introduction=NULL WHERE key='older-than-the-register'",
        [],
    )
    .unwrap();
    drop(conn);

    let text = prime(tmp.path());
    assert!(
        text.contains("- **older-than-the-register**: _(no introduction written yet)_"),
        "the gap is NAMED, not filled:\n{text}"
    );
    assert!(
        text.contains("**1 of these 2 memories has no introduction yet**"),
        "…and counted, so a reader knows how much of the index is missing:\n{text}"
    );
    assert!(
        text.contains("nxm classify <key> --introduction"),
        "…with the command that closes it:\n{text}"
    );
    assert!(
        text.contains("- **written-since**: the newer line"),
        "and the memory that has one is unaffected:\n{text}"
    );

    // `classify` closes the gap without touching the body — the verb the one-off run over the
    // existing memories uses.
    nxm(tmp.path())
        .args([
            "classify",
            "older-than-the-register",
            "--introduction",
            "a fact from before, now with a line",
        ])
        .assert()
        .success();
    let text = prime(tmp.path());
    assert!(
        text.contains("- **older-than-the-register**: a fact from before, now with a line"),
        "{text}"
    );
    assert!(
        !text.contains("no introduction"),
        "…and the count is gone with it:\n{text}"
    );
    assert!(
        nxm(tmp.path())
            .args(["recall", "older-than-the-register"])
            .assert()
            .success()
            .get_output()
            .stdout
            .windows(18)
            .any(|w| w == b"a fact from before"),
        "the body was never touched"
    );
}

/// **The one deliberate exception to the mandatory field, pinned rather than left to be found.**
///
/// `nxm import` (and `import_beads_memories_at`, which backs `nxs`'s beads onboarding) writes
/// through the store rather than through `facade::remember`, so an imported memory arrives with NO
/// introduction. That is on purpose — an imported fact is somebody else's text and there is no line
/// to carry over, and DERIVING one is exactly the mechanism this item removed. Review of PR #381
/// (Integrity #1 / Test Quality #4) found the state real, ongoing and untested; this is the test.
///
/// What it must show is that the exception lands in the SAME visible state as a memory older than
/// the register — a named gap that is counted and closable — and not in a blank line or a
/// fabricated one.
#[test]
fn an_imported_memory_reads_as_a_named_gap_rather_than_a_blank_or_a_derived_line() {
    let tmp = workspace();
    let dir = tmp.path();
    // A Claude-host memory source, in the layout `nxm import` reads: an index and one fact file.
    // The body opens with a dash on purpose — that is the shape the retired derivation rendered as
    // an empty `— -…` entry, so this fixture would have reproduced the original defect.
    let src = dir.join("claude-memory");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("MEMORY.md"), "- [Auth](auth-jwt.md) — auth\n").unwrap();
    std::fs::write(
        src.join("auth-jwt.md"),
        "---\nname: auth-jwt\n---\n\n- auth uses JWT, not sessions\n",
    )
    .unwrap();

    nxm(dir)
        .args(["import"])
        .env("NXM_CLAUDE_MEMORY_DIR", "claude-memory")
        .assert()
        .success();

    let text = prime(dir);
    assert!(
        text.contains("- **auth-jwt**: _(no introduction written yet)_"),
        "an import arrives as a NAMED gap — not a blank line, and not a line derived from the \
         body (the body opens with a dash, which is exactly what the old derivation rendered as \
         `— -…`):\n{text}"
    );
    assert!(
        text.contains("**1 of these 1 memories has no introduction yet**"),
        "…and it is COUNTED, so the gap is closable rather than merely visible:\n{text}"
    );
    assert!(
        !text.contains("auth uses JWT"),
        "…and the body still does not reach the session start:\n{text}"
    );

    // The body itself is intact and the gap closes through the ordinary verb.
    nxm(dir)
        .args([
            "classify",
            "auth-jwt",
            "--introduction",
            "auth is JWT, not sessions",
        ])
        .assert()
        .success();
    let text = prime(dir);
    assert!(
        text.contains("- **auth-jwt**: auth is JWT, not sessions"),
        "{text}"
    );
    assert!(!text.contains("no introduction"), "{text}");
}

/// **Promise 4: the obligation on a `rules` line is where an agent reads it BEFORE it writes.**
///
/// This is what carries the prohibition class now that there is no full-text channel. A prohibition
/// is only worth something if it is present before the mistake and nobody goes looking for one — so
/// under a pure index the line itself has to do the forbidding. Stating it in three places is not
/// redundancy: the session start is what an agent has already read, `--help` is what it reaches for
/// while writing, and the guide is what it reads when it wants the argument.
#[test]
fn the_rules_obligation_is_stated_at_session_start_and_in_the_verbs_help() {
    let tmp = workspace();
    let block = prime(tmp.path());
    // **Superseded 2026-08-28 (nxf q065, task 4).** The session-start wording changed (shorter,
    // owner-edited target draft — `facade::PRIME_INTRODUCTION_HINT`, not
    // `facade::PRIME_INTRODUCTION_RULE`, which is unchanged and still rides `--json` verbatim): "Under"
    // became lowercase "under" and the em dashes around the example became parentheses. The
    // underlying claim this test pins — the `--category rules` obligation is stated at session
    // start — did not change, so the substrings below were updated to match, not the assertion.
    assert!(
        block.contains("`--category rules` it speaks the rule")
            && block.contains("(\"Never X, always Y\", not \"Rules about X\")"),
        "the session start states it:\n{block}"
    );

    let help = nxm(tmp.path())
        .args(["remember", "--help"])
        .assert()
        .success()
        .get_output()
        .clone();
    let help = String::from_utf8_lossy(&help.stdout).to_string();
    assert!(
        help.contains("SPEAKS the rule") && help.contains("200 characters"),
        "`nxm remember --help` states it:\n{help}"
    );

    let guide = nxm(tmp.path())
        .args(["guide", "core-concepts"])
        .assert()
        .success()
        .get_output()
        .clone();
    let guide = String::from_utf8_lossy(&guide.stdout).to_string();
    assert!(
        guide.contains("SPEAKS the rule"),
        "and the guide states it:\n{guide}"
    );
}
