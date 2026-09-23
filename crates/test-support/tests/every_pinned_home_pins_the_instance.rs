//! **Nobody in this tree pins `$HOME` by hand** — a source gate (nxf 6j6v.9bjv).
//!
//! Pinning `$HOME` at a `TempDir` looks like the whole of test isolation and is not. Two other
//! variables out-vote it, and one of them is exported by this repository's own `.envrc`:
//! `NXS_SERVICE_INSTANCE=nexus-flow-dev`, deliberately, so that a development build here is not
//! the only nexus-flow clock on the machine (6j6v.gd9p). Every test subprocess inherits it, and
//! the resolution then reads `<the pinned home>/.nexusflow-dev` instead of the `.nexusflow` the
//! suite planted its registry, heartbeat and lock in.
//!
//! The damage is a FALSE RED, not a leak: nothing reaches the developer's real `~`. Measured on
//! 2026-09-01, ten tests across four binaries — `nexus-flow-cli --test sync_daemon` (6), `nxs
//! --test service_registration` (2), `--test mcp` (1), `--test multicall` (1) — failed on the
//! owner's machine and passed in CI, which loads no `direnv` and therefore never sets the
//! variable. A gate that is fail-open against exactly the environment the variable was built for
//! is worse than no gate: it teaches the developer to read their own red as noise.
//!
//! **Why a source gate and not a runtime one.** There is nothing to observe at runtime. Each of
//! those ten tests failed on its own honest assertion — an empty registry really does render no
//! rows — so no assertion, however careful, can tell "the product is wrong" from "the suite is
//! looking in the wrong place". The only place the mistake is visible is the source line that
//! names one variable and not the other.
//!
//! So the rule is a search rather than a judgement per call site: [`nxs_test_support::PinHome`] is
//! the one place `HOME` is spelled, and every other place says `pin_home`. That is the same
//! reasoning `crates/chat/tests/no_test_arms_the_real_scheduler.rs` states for `NXC_TIMER`, one
//! variable over, with the difference that this one admits no "say which you want" escape: a suite
//! whose SUBJECT is the instance pins the home first and then sets the variable it is about, which
//! is what `instance_isolation.rs` and `service_stays_awake.rs` do.
//!
//! **What that search does and does not guarantee** — stated, because the first version of this
//! file called the rule ABSOLUTE and it was not (review of PR #413, Test Quality #1). It reads
//! source text, so it holds for the spellings in [`needles`] and only those. Both live shapes in
//! this tree are covered — the builder call and `set_var`, the latter including any `…env(key, …)`
//! helper wrapped around it — and the two that are not covered do not occur here at all: a name
//! held in a variable, and a map handed to `.envs(…)`. That is a smaller promise than "nothing can
//! slip", and it is the true one.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The forbidden shapes: a call that HANDS the variable's name to something that sets it.
///
/// Assembled at runtime rather than written out, so this file can describe what it looks for
/// without matching itself — the self-exclusion `no_test_arms_the_real_scheduler.rs` needs (its
/// needles sit in its own message strings) is a hole a real suite can hide in, and there is no
/// reason to dig one here.
///
/// **There are two, and the second was missing** (review of PR #413, Test Quality #1). The first
/// version looked for `.env("HOME"` alone — one call SHAPE, not the action. `std::env::set_var`
/// contains no dot before `env(` and walked straight through, and it is this tree's dominant idiom
/// for setting a variable in a test process (fifteen files use it for other variables). Worse,
/// `crates/chat/tests/worker.rs` already owns a generic `with_env(key, value, f)` around exactly
/// that call: `with_env("HOME", tmp.path(), …)` would read as an entirely ordinary line and be
/// invisible. So the needles are chosen to end at `("HOME"` rather than begin at a dot, which
/// covers `.env(`, `with_env(` and any other `…env(` helper in one, and `set_var(` in the other.
///
/// **What they still do not cover, stated rather than implied away:** a name held in a variable
/// (`cmd.env(key, …)`), and a map built elsewhere and handed to `.envs(…)`. Neither exists in this
/// tree — checked, and `.envs(` appears nowhere at all — so the guarantee below is exact TODAY and
/// would need a new needle the day one appears. That is the honest shape of a text gate, and it is
/// why the module doc says what is checked instead of claiming that nothing can slip.
fn needles() -> [String; 2] {
    let q = '"';
    [format!("env({q}HOME{q}"), format!("set_var({q}HOME{q}")]
}

/// Call sites in the shape this tree carried BEFORE the gate, assembled the same way [`needle`] is
/// — but independently of it, which is the whole point: a needle that drifts by one character (a
/// stray `)`, a lost quote) stops matching these and reddens
/// [`the_gate_would_notice_a_hand_pinned_home_and_ignores_a_mere_read`] instead of silently
/// matching nothing for the rest of the repository's life. That is not hypothetical — the first
/// version of this file shipped exactly that typo, and only mutating a real call site back found
/// it.
fn hand_pinned_examples() -> [String; 4] {
    let q = '"';
    [
        format!("    c.current_dir(dir).env({q}HOME{q}, dir.join({q}.fake-home{q}));"),
        format!("        .env({q}HOME{q}, home.path())\n        .env_remove({q}XDG_DATA_HOME{q})"),
        // The two shapes the first version of this gate walked straight past.
        format!("    std::env::set_var({q}HOME{q}, tmp.path());"),
        format!("    with_env({q}HOME{q}, tmp.path(), || read_the_registry());"),
    ]
}

/// What CONSTRUCTING a `trycmd` corpus looks like — assembled for the same reason [`needle`] is,
/// and here it earns its keep twice over. `crates/chat/tests/no_test_arms_the_real_scheduler.rs`
/// counts the bare type name as one of ITS needles, in a string literal; a gate that matched the
/// name would flag that file for not pinning a home it never opens a corpus in. Matching the
/// constructor asks the question that matters — does this file RUN a corpus — and keeps this file
/// out of its own answer.
fn harness_marker() -> String {
    format!("TestCases{}new()", "::")
}

/// What RUNNING one of this suite's `init` verbs looks like, assembled the same way [`needles`]
/// is and for the same reason.
///
/// The shapes are the ones this tree actually uses: `.args(["init", …])`, `.args(["--json",
/// "init"])` (memory puts the global flag first), `.arg("init")`, and a slice handed to a local
/// helper — `run("nxm", dir, &["init"])`. What they have in common is the quoted literal beside an
/// argument-passing call, which is what tells them apart from the several places that merely SPEAK
/// of init: `help.contains("init")` and `forced.contains("init")` are assertions about output, not
/// invocations, and neither carries `arg(` or `&[`.
fn runs_init(body: &str) -> bool {
    let q = '"';
    let init = format!("{q}init{q}");
    body.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .any(|l| {
            l.contains(&init) && (l.contains("arg(") || l.contains("args(") || l.contains("&["))
        })
}

/// Does `body` reach a pinned `$HOME` at all — by any of the three sanctioned routes?
fn reaches_a_pinned_home(body: &str) -> bool {
    [
        "pin_home(",
        "nxs_test_support::cargo_bin",
        "pinned_home_env()",
    ]
    .iter()
    .any(|needle| calls(body, needle))
}

/// **The gate's whole question, in one place**: does `body` pin `$HOME` by hand?
///
/// The tree scan and the controls below both go through THIS, which is the point (review of
/// PR #413, Test Quality #2). The first version's control checked the needle against fixture
/// strings and never called the predicate the scan used, so a regression in the comment-stripping
/// half would have been caught only by luck, on the full-tree pass — a control that does not
/// exercise the thing it controls.
fn pins_home_by_hand(body: &str) -> bool {
    needles().iter().any(|needle| calls(body, needle))
}

/// Does `body` CALL something spelled `needle`, as opposed to talking about it?
///
/// Comment lines are dropped first, and that is not fussiness: the four golden harnesses each
/// explain in prose why they pin a home, naming the helper. A gate that accepted a mention would
/// have been satisfied by the explanation of the very thing that had been deleted — which is what
/// the first version of this file did, and what mutating a harness to remove the call revealed.
fn calls(body: &str, needle: &str) -> bool {
    body.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .any(|l| l.contains(needle))
}

/// The repo root, from this crate's manifest dir (`<root>/crates/test-support`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/test-support lives two levels below the repo root")
        .to_path_buf()
}

/// Every Rust source in the workspace: `crates/*/src`, `crates/*/tests` and `xtask/src`.
///
/// `src` is walked as well as `tests` because a helper that builds a subprocess is not confined to
/// a test target — `crates/chat/src/worker.rs` builds the sidecar's command in library code, and a
/// pinned home there would carry exactly the same miss.
fn sources(root: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![root.join("xtask").join("src")];
    for krate in std::fs::read_dir(root.join("crates"))
        .expect("crates/ is readable")
        .flatten()
    {
        roots.push(krate.path().join("src"));
        roots.push(krate.path().join("tests"));
    }

    let mut out: BTreeSet<PathBuf> = BTreeSet::new();
    let mut stack = roots;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.insert(path);
            }
        }
    }
    out.into_iter().collect()
}

#[test]
fn no_source_pins_home_without_pinning_the_service_instance() {
    let root = repo_root();
    let sources = sources(&root);
    // The floor is the gate's own premise. A walk that finds nothing reports "clean" forever,
    // which is the silence every gate in this crate exists to abolish.
    assert!(
        sources.len() > 300,
        "the gate has lost sight of the tree it watches — only {} sources found under \
         crates/*/{{src,tests}} and xtask/src",
        sources.len()
    );

    let findings: Vec<String> = sources
        .iter()
        .filter(|path| {
            pins_home_by_hand(
                &std::fs::read_to_string(path).expect("a workspace source is readable"),
            )
        })
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .collect();

    assert!(
        findings.is_empty(),
        "\n\nThese sources pin `$HOME` by hand:\n  {}\n\n  \
         A pinned home does not decide on its own where the product looks. `XDG_CONFIG_HOME` and \
         `XDG_DATA_HOME` answer BEFORE it on Linux, and `NXS_SERVICE_INSTANCE` — which this \
         repository's own `.envrc` exports (nxf 6j6v.gd9p) — sends the resolution to \
         `<the pinned home>/.nexusflow-dev`, a directory the suite never wrote to. That is how ten \
         tests came to be red on the owner's machine and green in CI, where the variable is unset \
         (nxf 6j6v.9bjv).\n\n  \
         Say `.pin_home(<dir>)` instead (`use nxs_test_support::PinHome;`), which carries all \
         three. A suite whose SUBJECT is one of them pins the home first and then sets the \
         variable it is about — `crates/nxs/tests/instance_isolation.rs` is the worked example.\n",
        findings.join("\n  ")
    );
}

#[test]
fn the_gate_would_notice_a_hand_pinned_home_and_ignores_a_mere_read() {
    // Positive control, through the REAL predicate: the detector fires on every shape it forbids.
    // Without this the test above passes just as well against a needle that matches nothing — the
    // failure mode a source gate has and a type-level one does not, and the one this file shipped
    // with in its first version (a stray `)` in the needle, found only by mutating a real call
    // site back).
    for offender in hand_pinned_examples() {
        assert!(
            pins_home_by_hand(&offender),
            "the detector must fire on a hand-pinned home: {offender}"
        );
    }

    // Negative control: it is about the CALL that sets the variable, not the word. Every line here
    // is a real shape from this tree, and each would turn the gate into one that cries wolf —
    // which is how a gate gets routed around rather than obeyed.
    for innocent in [
        // Reading the ambient home is ordinary: `crates/cli/tests/sync_daemon.rs` names the real
        // one in an assertion.
        "let real = std::env::var(\"HOME\").expect(\"a HOME\");",
        "std::env::var_os(\"HOME\").map(PathBuf::from)",
        // A passthrough LIST of variable names — `crates/chat/src/worker.rs`'s sidecar hurdle.
        "[\"PATH\", \"HOME\", \"USER\"]",
        // A fake environment MAP in a pure unit test, and an assertion on an error's text — both
        // from `crates/chat/src/worker.rs`, both carrying `(\"HOME\"` and neither setting anything.
        "cache_root(env_of(&[(\"HOME\", \"/home/u\")]))",
        "assert!(err.msg.contains(\"HOME\"), \"{}\", err.msg);",
    ] {
        assert!(
            !pins_home_by_hand(innocent),
            "this sets nothing and must not be flagged: {innocent}"
        );
    }

    // …and the comment half of the predicate, which nothing else exercises directly: a line that
    // TALKS about the forbidden shape is prose. Every one of the four golden harnesses explains in
    // prose why it pins a home, so a predicate that read mentions would be satisfied by the very
    // explanation of a call somebody had deleted.
    let [spoken, ..] = hand_pinned_examples();
    assert!(
        !pins_home_by_hand(&format!("    // like this: {spoken}")),
        "a comment naming the shape is prose, not a call"
    );
    assert!(
        pins_home_by_hand(&format!("    // like this:\n{spoken}")),
        "…but the line under it is still read"
    );
}

#[test]
fn the_four_suites_that_were_red_on_the_owners_machine_go_through_the_helper() {
    // The rule above is satisfied by a file that stopped pinning a home at all, so it cannot by
    // itself say the ten measured tests are fixed rather than merely rewritten. These four are the
    // binaries the ticket measured (nxf 6j6v.9bjv); each has to reach the helper that clears the
    // instance, and the paths double as proof that the walk really covers `crates/cli`,
    // `crates/nxs` and `crates/server` and not just the crate this gate lives in.
    let root = repo_root();
    for rel in [
        "crates/cli/tests/sync_daemon.rs",
        "crates/nxs/tests/service_registration.rs",
        "crates/nxs/tests/mcp.rs",
        "crates/nxs/tests/multicall.rs",
        "crates/server/tests/relay_e2e.rs",
    ] {
        let path = root.join(rel);
        assert!(
            sources(&root).contains(&path),
            "{rel} is not in the walked set — the gate is blind to the crate it lives in"
        );
        let body = std::fs::read_to_string(&path).expect("readable");
        assert!(
            calls(&body, ".pin_home("),
            "{rel} pins a `$HOME` for its subprocesses and must do it through `PinHome`, which \
             clears the service instance with it"
        );
    }
}

#[test]
fn every_trycmd_corpus_pins_a_home_too() {
    // The other half of the rule, and the one it cost a poisoned corpus to learn (nxf 6j6v.q6e3):
    // a `trycmd` harness runs the binary through its own runner, so it inherits NOTHING from
    // `nxs_test_support::cargo_bin` — not the pinned home, not the instance. It is also the worst
    // place to inherit the developer's: a golden asserts stderr byte for byte, so the service
    // report an invocation makes on a machine with a dangling program alias (nxf 6j6v.dcpk) fails
    // every case, and `TRYCMD=overwrite` then writes that machine's paths into the committed
    // goldens. `TestCases` has `env` and no `env_remove`, which is why the values come from
    // `pinned_home_env` rather than from `PinHome`.
    let root = repo_root();
    let mut findings: Vec<String> = Vec::new();
    let mut harnesses = 0usize;
    for path in sources(&root) {
        let body = std::fs::read_to_string(&path).expect("a workspace source is readable");
        if !calls(&body, &harness_marker()) {
            continue;
        }
        harnesses += 1;
        if !calls(&body, "pinned_home_env()") {
            findings.push(
                path.strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            );
        }
    }
    assert!(
        harnesses >= 4,
        "the gate found {harnesses} trycmd harnesses and this repo has four — it has gone blind"
    );
    assert!(
        findings.is_empty(),
        "\n\nThese golden harnesses run the binary under the developer's own `$HOME`:\n  {}\n\n  \
         Say `for (key, value) in nxs_test_support::pinned_home_env() {{ cases.env(key, value); }}` \
         before the corpus runs.\n",
        findings.join("\n  ")
    );
}

#[test]
fn every_test_that_runs_init_pins_a_home_first() {
    // **`init` is the one verb that WRITES to the machine** (nxf 6j6v.y12q, review of PR #421).
    //
    // Every other black-box call in this tree reads. Since y12q, `nxf init` and `nxm init` put the
    // workspace they create on the background service's list — so a test that runs one without a
    // pinned home writes a `TempDir` path, deleted moments later, into the developer's real
    // `~/.nexusflow/workspaces.toml`. That is the 98-stale-entries incident this crate's own
    // `PinHome` doc names, arriving from a new direction.
    //
    // It is not hypothetical and it is not one file. `nxs_test_support::cargo_bin` pins a home
    // structurally, but only for the tests that CALL it; three suites here built their commands
    // from a bare `assert_cmd` path instead (`multicall.rs`, `sigpipe.rs`, `mcp_sigpipe.rs`) and
    // silently gained the write. Measured on the owner's machine on 2026-09-03, before the fix:
    // 384 phantom entries in the development registry, 100 in the production one.
    //
    // The review that found this recommended a grep before merge. This is that grep, kept.
    //
    // **What it checks, stated rather than implied** — the same honest shape as the gate above: it
    // reads source text, so it holds for the spellings in [`runs_init`] and only those, and it
    // asks its question per FILE rather than per call site. A file that pins a home for one
    // subprocess and forgets another still passes; what it cannot do is run `init` while knowing
    // nothing about pinning at all, which is exactly how all three misses happened. `git(ws,
    // &["init", …])` in `crates/chat/tests/smoke_v3.rs` matches too — that is `git init`, not
    // ours — and costs nothing, because that suite pins anyway.
    let root = repo_root();
    let mut findings: Vec<String> = Vec::new();
    let mut runners = 0usize;
    for path in sources(&root) {
        // Test sources only: production code re-execs `nxs init` (the front doors' umbrella
        // hand-off) and must never pin a home.
        if !path.components().any(|c| c.as_os_str() == "tests") {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("a workspace source is readable");
        if !runs_init(&body) {
            continue;
        }
        runners += 1;
        if !reaches_a_pinned_home(&body) {
            findings.push(
                path.strip_prefix(&root)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            );
        }
    }
    assert!(
        runners >= 10,
        "the gate found {runners} suites running `init` and this repo has many more — it has gone \
         blind"
    );
    assert!(
        findings.is_empty(),
        "\n\nThese suites run an `init` verb without pinning a `$HOME` anywhere:\n  {}\n\n  \
         Since nxf 6j6v.y12q every `nxf init`/`nxm init` registers the workspace with this \
         machine's background service, so an unpinned one writes a soon-deleted `TempDir` path \
         into the developer's own `~/.nexusflow/workspaces.toml`.\n\n  \
         Build the command with `nxs_test_support::cargo_bin`, which pins one structurally, or say \
         `.pin_home(<dir>)` (`use nxs_test_support::PinHome;`) on the ones you build yourself.\n",
        findings.join("\n  ")
    );
}

#[test]
fn the_init_gate_tells_an_invocation_from_a_sentence_about_one() {
    // Positive control, through the REAL predicates — the failure mode a text gate has and the one
    // the first version of the gate above actually shipped with.
    for invocation in [
        r#"        .args(["init", "--plugin", "issue-tracker"])"#,
        r#"        .args(["--json", "init"])"#,
        r#"    bin("nxm", tmp.path()).arg("init").assert().success();"#,
        r#"    run("nxf", dir, &["init", "--quiet", "--plugin", "issue-tracker"]);"#,
    ] {
        assert!(runs_init(invocation), "this RUNS init: {invocation}");
    }

    // Negative control: every one of these is a real line from this tree that talks about init
    // without running it. A gate that flagged them would cry wolf, which is how a gate gets
    // routed around rather than obeyed.
    for innocent in [
        r#"        help.contains("init") && help.contains("prime"),"#,
        r#"        forced.contains("no workspace") || forced.contains("init"),"#,
        r#"    // .args(["init", "--plugin", "issue-tracker"]) — how it used to read"#,
    ] {
        assert!(!runs_init(innocent), "this runs nothing: {innocent}");
    }

    // And the other half: the three sanctioned routes to a pinned home all count, a file that
    // merely names the concept does not.
    assert!(reaches_a_pinned_home(
        "    let c = nxs_test_support::cargo_bin(\"nxf\");"
    ));
    assert!(reaches_a_pinned_home(
        "        .pin_home(tmp.path().join(\".fake-home\"))"
    ));
    assert!(reaches_a_pinned_home(
        "    for (k, v) in nxs_test_support::pinned_home_env() {}"
    ));
    assert!(!reaches_a_pinned_home(
        "// this suite needs a pinned home one day"
    ));
}
