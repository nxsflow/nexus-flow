//! **No test may arm this machine's real scheduler** (nxf 6j6v.74c0, and the reason this gate
//! exists rather than a comment: it was breached within the hour).
//!
//! `nxc` arms a declared channel `timeout:` for real, and the whole point of the clock is that it
//! WORKS: a `send --to <a channel with a timeout>` writes a deadline the running nexus-flow service
//! will act on, keyed on the thread.
//!
//! **What a forgotten `NXC_TIMER` costs changed shape with 6j6v.8see, and it did not get smaller.**
//! Under the per-deadline `launchd` backend it left one self-removing agent per channel in
//! `~/Library/LaunchAgents`, each due to `cd` into a `TempDir` that no longer exists — bounded, and
//! visible in a directory listing. Under the service, an UNPINNED timer resolves its workspace from
//! the working directory, and a test's working directory is this repository. So the same slip now
//! writes a deadline into the DEVELOPER'S OWN `.nxs/timers.json`, on a board id that came out of a
//! tempdir — and their running service dutifully starts `nxs chat tick --thread <id>` here for it.
//! Nothing about that is visible on the Linux runner where CI actually looks, which is why this is
//! a gate and not a comment (it was breached within the hour of the first version).
//!
//! **Why the default is not enough on its own.** `nxs_test_support::cargo_bin` sets
//! `NXC_TIMER=dry`, which covers every suite that goes through it — and that is nearly all of
//! them. It cannot cover a suite that builds its command another way, and there are real reasons
//! to: `working_tree_two_process_e2e.rs` needs a raw `std::process::Command` it can `spawn`
//! without waiting, which `assert_cmd::Command` cannot express, and a `trycmd` corpus runs the
//! binary through its own runner. Those are the places this gate is about.
//!
//! The rule is deliberately about SAYING SOMETHING rather than about saying `dry`: a live smoke
//! that genuinely wants the real scheduler passes by writing that down.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The repo root, from this crate's manifest dir (`<root>/crates/chat`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/chat lives two levels below the repo root")
        .to_path_buf()
}

/// Every `crates/*/tests/**.rs` file, sorted.
fn test_sources(root: &Path) -> Vec<PathBuf> {
    let mut out: BTreeSet<PathBuf> = BTreeSet::new();
    let crates = root.join("crates");
    for krate in std::fs::read_dir(&crates)
        .expect("crates/ is readable")
        .flatten()
    {
        let tests = krate.path().join("tests");
        if !tests.is_dir() {
            continue;
        }
        let mut stack = vec![tests];
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
    }
    out.into_iter().collect()
}

/// Does the `trycmd` corpus beside `harness` actually RUN `nxc`? Its `.trycmd` files are where the
/// binary is named, so a harness's own source cannot answer — and the string `nxc` appearing in a
/// corpus is not enough either: `crates/nxs/tests/golden/guide.trycmd` prints it inside `nxs guide`
/// OUTPUT without ever invoking it.
fn corpus_runs_nxc(harness: &Path) -> bool {
    let dir = harness.with_extension("");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        e.path().extension().is_some_and(|x| x == "trycmd")
            && std::fs::read_to_string(e.path())
                .unwrap_or_default()
                .lines()
                .any(|l| l.trim_start().starts_with("$ ") && l.contains("nxc "))
    })
}

#[test]
fn every_suite_that_builds_its_own_nxc_command_says_which_timer_it_wants() {
    let root = repo_root();
    let sources = test_sources(&root);
    assert!(
        sources.len() > 40,
        "the gate has lost sight of the tree it watches — only {} test sources found under \
         crates/*/tests",
        sources.len()
    );

    let mut findings: Vec<String> = Vec::new();
    let mut watched: Vec<String> = Vec::new();
    for path in &sources {
        // The gate must not watch ITSELF (review of this branch, Test Quality #7): its own needles
        // appear inside its own string literals, which would let a real suite drop out of detection
        // while the floor below still held.
        if path
            .file_stem()
            .is_some_and(|f| f == "no_test_arms_the_real_scheduler")
        {
            continue;
        }
        let body = std::fs::read_to_string(path).expect("a test source is readable");
        // Count the places that build an `nxc` command OUTSIDE `nxs_test_support::cargo_bin` —
        // which is the one place the `NXC_TIMER=dry` default lives. Every shape that reaches the
        // binary another way belongs here; the last two were added because they are already in use
        // elsewhere in this tree for `nxf`, so they are live shapes rather than hypotheticals
        // (Test Quality #6).
        let builders: usize = ["nxc_binary()", "Command::cargo_bin(", "CARGO_BIN_EXE_nxc"]
            .iter()
            .map(|needle| body.matches(needle).count())
            .sum::<usize>()
            // `cargo_bin("nxc")` counts ONLY when it is not the helper's own — that one is the
            // covered case, and counting it would flag every ordinary suite in the tree.
            + (body.matches("cargo_bin(\"nxc\")").count()
                - body.matches("nxs_test_support::cargo_bin(\"nxc\")").count())
            // A `trycmd` corpus runs the binary through its own runner and inherits nothing from
            // that helper at all. It names its binary in the `.trycmd` files rather than in the
            // `.rs`, so the HARNESS is what is counted — and every corpus is checked, not only
            // chat's, because a corpus in any crate can drive `nxc`.
            + body.matches("trycmd::TestCases").count();
        if builders == 0 {
            continue;
        }
        // …and it must be about `nxc`. A suite that only ever runs `nxf`/`nxm`/`nxs` schedules
        // nothing. For a trycmd harness the answer is in its corpus, not in its source.
        let names_nxc = body.contains("\"nxc\"")
            || body.contains("nxc_binary()")
            || body.contains("CARGO_BIN_EXE_nxc")
            || (body.contains("trycmd::TestCases") && corpus_runs_nxc(path));
        if !names_nxc {
            continue;
        }
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        watched.push(rel.clone());
        if !body.contains("NXC_TIMER") {
            findings.push(rel);
        }
    }

    // **The watched set itself is the assertion**, not a count (review of this branch, Test
    // Quality #6/#7). Presence of `NXC_TIMER` is what the gate checks per file, and that has one
    // named residual: a file with TWO builders where only one is pinned would satisfy it. Counting
    // builders instead does not work — `multicall.rs` funnels seventeen of them through one helper
    // that pins once, and `working_tree_two_process_e2e.rs` names its resolver in prose as well as
    // in code, so a count produces false positives, and a gate that cries wolf gets weakened.
    //
    // So the set is pinned by NAME. A new file that builds its own `nxc` command has to be added
    // here in the same commit, which is the moment somebody looks at whether it pins the timer —
    // and that look is the thing a counter was standing in for.
    let expected = [
        "crates/chat/tests/golden.rs",
        "crates/chat/tests/working_tree_two_process_e2e.rs",
        // nxf 6j6v.k8zq: `nxs prime --persona` spans all three modules, so its proof drives all
        // three binaries in one workspace — and it sends real messages to summon a persona, which
        // is exactly the shape that would arm a real deadline. It pins `NXC_TIMER=dry` at `bin()`.
        "crates/nxs/tests/a_persona_gets_the_suites_prime.rs",
        "crates/nxs/tests/e2e.rs",
        "crates/nxs/tests/multicall.rs",
        // nxf 6j6v.dcpk: a dead service program link has to be reported by EVERY persona, so this
        // suite runs all four — including `nxc`. It arms nothing (`--version` runs no verb) and it
        // pins `NXC_TIMER=dry` at `stderr_of()` anyway, because "it happens not to schedule today"
        // is exactly the reasoning this gate exists to stop relying on.
        "crates/nxs/tests/service_program.rs",
        // nxf 6j6v.xbnh: the gate on the COMPOSED session start measures `nxs prime` over a
        // workspace with all three modules active, so it runs `nxc init` too. It pins
        // `NXC_TIMER=dry` at `bin()`.
        "crates/nxs/tests/the_session_start_ceiling.rs",
    ];
    watched.sort();
    assert_eq!(
        watched, expected,
        "\n\nThe set of suites that build their own `nxc` command changed.\n\n  \
         Every file here runs the binary WITHOUT `nxs_test_support::cargo_bin`, so none of them \
         inherits its `NXC_TIMER=dry` default. Add the new one to this list in the same commit \
         that adds the suite — and while you are looking at it, check that it says which timer it \
         wants.\n"
    );

    assert!(
        findings.is_empty(),
        "\n\nThese suites build their own `nxc` command and never say which timer they want:\n  \
         {}\n\n  \
         On macOS an unset `NXC_TIMER` means the REAL `launchd` backend (nexus-flow-6j6v.74c0), so \
         every channel with a declared `timeout:` these open bootstraps a one-shot agent into the \
         developer's own login session — due to fire `nxc tick` in a `TempDir` that is gone by \
         then. `nxs_test_support::cargo_bin` sets `NXC_TIMER=dry` for every suite that goes \
         through it; a suite that builds its command another way has to say it itself.\n\n  \
         Add `.env(\"NXC_TIMER\", \"dry\")` — or, for a live smoke that genuinely wants the real \
         scheduler, write that down in the same file so this gate can see the decision.\n",
        findings.join("\n  ")
    );
}
