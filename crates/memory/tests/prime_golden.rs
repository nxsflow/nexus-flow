//! The byte-exact `nxm prime` contract (nxf 6j6v.wph0, epic 6j6v.fjrc).
//!
//! `nxm prime` is a SessionStart-hook output: `nxs prime` fans out to it and a host injects the
//! Markdown verbatim as session context. Its bytes are therefore a CONTRACT, not an implementation
//! detail — and lifting the assembly out of `cli.rs` into the shared facade layer is a MOVE, not a
//! reformulation. `contract.rs` asserts the *meaning* (the rule is stated, the commands are listed,
//! every memory is replayed); this file pins the exact stdout, so the move can be proven byte-for-
//! byte rather than argued.
//!
//! Written against the pre-lift implementation deliberately: if the shared-layer renderer changes so
//! much as a space, these fail.
//!
//! **Rewritten 2026-08-28 (nxf q065, task 4, q3fh session-start-budget).** The fixed head shrank
//! for the session-start byte budget (the host drops any single hook output above 10.240 B rather
//! than delivering it smaller): the Context Recovery blockquote and the `NEXUS_MEMORY.md`/
//! `CLAUDE.md` paragraph are gone from the human view (both survive in `--json`, see
//! `crates/memory/src/facade.rs::PrimeReport::render_markdown`'s doc comment for the full
//! accounting), "## Memory Commands" became "## How a memory is written" with three verbs instead
//! of six, and the open-migration note moved from the head to after the memories. This file's own
//! fixed `RULES`/`COMMANDS` constants and `head()` helper are rewritten to match, MEASURED against
//! this task's own build (`cargo test -p nexus-memory --test prime_golden`).

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

/// An `nxm` invocation with actor/now/ids pinned, so authors and `updated` stamps are stable.
fn nxm(dir: &Path) -> Command {
    let mut c = nxs_test_support::cargo_bin("nxm");
    c.current_dir(dir)
        .env("NXM_ACTOR", "alice")
        .env("NXM_NOW", "2026-06-20T10:00:00Z")
        .env("NXF_DETERMINISTIC_IDS", "1");
    c
}

/// The introduction a fixture memory carries — derived from its key so every line in the index is
/// DISTINCT, which is what makes a byte-exact index assertion say anything at all.
fn intro_for(key: &str) -> String {
    format!("the one line about {key}")
}

/// A memory workspace with the given `(key, body)` facts remembered, in order.
fn workspace(facts: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new().unwrap();
    nxm(tmp.path()).arg("init").assert().success();
    for (key, body) in facts {
        nxm(tmp.path())
            .args([
                "remember",
                body,
                "--key",
                key,
                "--introduction",
                &intro_for(key),
            ])
            .assert()
            .success();
    }
    tmp
}

fn stdout_of(dir: &Path, args: &[&str]) -> String {
    let out = nxm(dir).args(args).assert().success().get_output().clone();
    String::from_utf8(out.stdout).expect("utf8")
}

/// The invariant head of every `nxm prime` (nxf q065, task 4): title, the intro paragraph, the
/// correction rule, the "## How a memory is written" cheatsheet, and the introduction obligation.
/// Everything a session is handed regardless of the workspace's state — the migration note is NOT
/// part of it any more, see `head()` below.
const RULES: &str = "\
# nexus-memory — what this project knows

`nxm` carries durable project knowledge — conventions, gotchas, decisions — and replays it into \
every session. **Never write a `MEMORY.md`** or any other ad-hoc memory file: nothing reads it, so \
it is never replayed and the knowledge is silently lost. `nxm remember` is the one durable channel, \
and reusing a stable `--key` evolves a fact in place. `nxm --help` lists the commands, `nxm guide` \
the topics behind them.

**Keep them true.** Each memory below is named by its key. If you apply one and the project says \
otherwise, do not read past it: correct it in place with `nxm remember \"<the corrected fact>\" \
--key <key>`, or `nxm forget <key>` if it no longer holds at all. You are the best-informed \
corrector this memory will ever have.

## How a memory is written

    nxm remember \"<fact>\" --introduction \"<one line>\" --key <key>
    nxm recall <key>                     # read one memory in full
    nxm memories [<search>]              # list or search them

`--introduction` is ONE line, at most 200 characters, and it says what the memory SAYS — under \
`--category rules` it speaks the rule (\"Never X, always Y\", not \"Rules about X\"), because \
nobody looks a prohibition up before breaking it.
";

/// The paragraph that heads the index (nxf 6j6v.xbnh) — one written line per memory, and no
/// full-text channel at all.
const INDEX_RULE: &str = "\
**One line per memory — this is an index, not the memories.** Each line is the introduction its \
author wrote for it. Read one in full with `nxm recall <key>`, or search their bodies with \
`nxm memories <text>`.";

/// The migration blockquote for `unsorted` unfiled memories — or an empty string at `0`, without
/// it at all. **Moved 2026-08-28 (nxf q065, task 4): now the block's own TAIL, appended after the
/// memories, not part of `head()` any more** — it names a one-time, situational move, not a
/// standing fact every session needs before the memories themselves.
fn migration_tail(unsorted: usize) -> String {
    match unsorted {
        0 => String::new(),
        n => format!(
            "\n\n> **Open migration:** {n} {} still filed as `unsorted`, so this project's memory \
             document cannot read as an argument yet. Run `nxm migrate plan --with-judge --json > \
             migration.json`, read the plan, then `nxm migrate apply --plan migration.json`.",
            if n == 1 { "memory is" } else { "memories are" }
        ),
    }
}

#[test]
fn prime_with_no_memories_is_byte_exact() {
    // Nothing remembered means nothing unfiled, so the migration line is absent — the "silent when
    // none is open" half of the rule, pinned at the byte level.
    let tmp = workspace(&[]);
    assert_eq!(
        stdout_of(tmp.path(), &["prime"]),
        format!(
            "{RULES}\n## Memories (0)\n\n\
             _No memories yet — capture durable project knowledge with `nxm remember`._{}\n",
            migration_tail(0)
        )
    );
}

#[test]
fn prime_replays_memories_as_one_written_line_each_byte_exact() {
    // Two memories, the first multi-paragraph — and NEITHER body appears (nxf 6j6v.xbnh). That is
    // the whole saving, and it is what makes the block's size a function of the memory COUNT rather
    // than of what anyone wrote. Neither is filed, so the order is the reading order's own tail —
    // the order they were WRITTEN in (6j6v.643z), which is why `dolt-phantoms` leads the
    // alphabetically earlier `auth-jwt`.
    let tmp = workspace(&[
        (
            "dolt-phantoms",
            "Dolt phantoms.\n\nA second paragraph with a `code span`.",
        ),
        ("auth-jwt", "auth uses JWT, not sessions"),
    ]);
    let block = stdout_of(tmp.path(), &["prime"]);
    assert_eq!(
        block,
        format!(
            "{RULES}\n## Memories (2)\n\n\
             {INDEX_RULE}\n\n\
             - **dolt-phantoms**: the one line about dolt-phantoms\n\
             - **auth-jwt**: the one line about auth-jwt{}\n",
            migration_tail(2)
        )
    );
    assert!(
        !block.contains("A second paragraph"),
        "no body reaches the session start:\n{block}"
    );
}

#[test]
fn prime_replays_a_filed_bestand_as_its_argument_byte_exact() {
    // 6j6v.643z at the byte level, on the surface a session is actually handed: category first —
    // what the workspace IS, then the idea that carries it, then the hard rules, then the rest by
    // category name, and last whatever nobody filed — with the stored position deciding inside a
    // category. Every memory here is written in an order that is neither the alphabet nor the
    // result, so a regression to either shows up as a diff rather than as a coincidence.
    let tmp = workspace(&[]);
    for (key, body, category) in [
        ("zebra", "a rule", Some("rules")),
        ("crates", "the crate map", Some("architecture")),
        ("later", "not filed yet", None),
        ("what", "what this workspace is", Some("introduction")),
        ("alpha", "how a release is cut", Some("release-process")),
        ("branching", "always work on a branch", Some("rules")),
    ] {
        let introduction = intro_for(key);
        let mut args = vec![
            "remember",
            body,
            "--key",
            key,
            "--introduction",
            &introduction,
        ];
        if let Some(category) = category {
            args.extend(["--category", category]);
        }
        nxm(tmp.path()).args(args).assert().success();
    }
    // A position WITHIN `rules`: it moves `branching` ahead of `zebra` and moves nothing else.
    nxm(tmp.path())
        .args(["reorder", "branching"])
        .assert()
        .success();

    // The reading order is unchanged and still the point — and since nxf 6j6v.xbnh it is ONE
    // sequence again, because there are no longer two classes to run it through. Every memory is
    // one line, in the order the argument reads.
    assert_eq!(
        stdout_of(tmp.path(), &["prime"]),
        format!(
            "{RULES}\n## Memories (6)\n\n\
             {INDEX_RULE}\n\n\
             - **what**: the one line about what\n\
             - **crates**: the one line about crates\n\
             - **branching**: the one line about branching\n\
             - **zebra**: the one line about zebra\n\
             - **alpha**: the one line about alpha\n\
             - **later**: the one line about later{}\n",
            // One memory is still `unsorted`, so the migration line names exactly that one.
            migration_tail(1)
        )
    );
}

#[test]
fn prime_json_is_byte_exact() {
    // The `--json` projection: `serde_json::Value`'s own key order (a BTreeMap — no
    // `preserve_order` in the build graph), so the object keys are alphabetical and stable.
    //
    // Unlike the human view above, this one GREW under 6j6v.wph0 — additively. `count` and
    // `memories` are untouched; `context_recovery`, `memory_rule` and `commands` joined them
    // because the JSON is now a projection of the whole report rather than a second, thinner
    // assembly. That is the point of the lift: an agent reading `nxm prime --json` gets exactly what
    // an embedding host gets from `Engine::prime`, prose included, instead of having to know the
    // wording to reproduce the block.
    //
    // It grew a second time under 6j6v.e0z6, and again only additively: each replayed memory now
    // also reports how it is FILED (`category`, `scope`, `refs`, `ordinal`). An unclassified memory
    // reports the status quo — `unsorted`, reach `project`, nothing referenced, no explicit position
    // — which is precisely why the human block above did not have to change a byte.
    //
    // 6j6v.srpg added the one entry that DID change both views: `nxm classify` joined the command
    // reference. The reach decides where a memory surfaces, so the verb that sets it belongs in the
    // list an agent is handed at session start — a mechanism nobody is told about is one nobody uses.
    //
    // 6j6v.9yaj added two more, and both are additive again: `correction_rule` (how a memory stays
    // true) and `migration`, which is present ONLY while something is still unfiled — a consumer
    // that never migrates simply never sees the key, rather than a `null` it has to interpret.
    //
    // 6j6v.sebs added `context_document`: why `CLAUDE.md` does not import the generated
    // `NEXUS_MEMORY.md`. Additive again, and on BOTH views on purpose — an embedding host that
    // renders its own bootstrap has the same question to answer as a session reading the Markdown.
    //
    // 6j6v.xbnh added `introduction_rule` and, per memory, `introduction` — additive again on the
    // JSON side, while the HUMAN view above changed shape: the bodies left it. That asymmetry is
    // deliberate and is the reason this projection matters. The saving is a RENDERING decision, so
    // a host reading `--json` still gets every body in full and can render its own bootstrap.
    //
    // **Superseded 2026-08-28 (nxf q065, task 4): the JSON shape itself is UNCHANGED by this task —
    // every key below still appears, unmoved.** `context_recovery`, `memory_rule`,
    // `context_document`, `introduction_rule` and all six `commands` (including both `nxm classify`
    // entries) are exactly the strings that shipped before this task; only `render_markdown()`
    // (the human view pinned in the other three tests in this file) reads differently now. See
    // `crates/memory/src/facade.rs::PrimeReport::render_markdown`'s doc comment for the accounting.
    let tmp = workspace(&[("auth-jwt", "auth uses JWT, not sessions")]);
    assert_eq!(
        stdout_of(tmp.path(), &["--json", "prime"]),
        concat!(
            r#"{"commands":[{"invocations":["nxm remember \"<fact>\" "#,
            r#"--introduction \"<one line>\" [--key <key>]"],"summary":"add or update a fact; "#,
            r#"the introduction is the one line the index below replays, and reusing a stable "#,
            r#"`--key` evolves the fact in place"},"#,
            r#"{"invocations":["nxm recall <key>"],"summary":"read one memory's full text"},"#,
            r#"{"invocations":["nxm memories [<search>]"],"summary":"list or search memories"},"#,
            r#"{"invocations":["nxm classify <key> --scope item --refs <item-id>"],"#,
            r#""summary":"file a memory against board items; it then reads on `nxf show <item-id>` "#,
            r#"instead of here (reach `project`/`global` keeps reading here)"},"#,
            r#"{"invocations":["nxm classify <key> --introduction \"<one line>\""],"#,
            r#""summary":"rewrite a memory's line below, leaving its text alone"},"#,
            r#"{"invocations":["nxm forget <key>"],"summary":"remove a memory (reversible)"}],"#,
            r#""context_document":"**This block IS how the project's memory reaches you.** It "#,
            r#"arrived through this project's SessionStart hook (`nxm prime`). The generated "#,
            r#"`NEXUS_MEMORY.md` repeats the index below and adds every body beneath it, so "#,
            r#"importing that file from `CLAUDE.md` would hand you all of it again. The missing "#,
            r#"import is deliberate, not an oversight: nothing needs to be added to `CLAUDE.md` "#,
            r#"for this project's memory to reach you.","#,
            r#""context_recovery":"re-run `nxs prime` after a context compaction to reload these "#,
            r#"memories.","#,
            r#""correction_rule":"**Keep them true.** Each memory below is named by its key. If "#,
            r#"you apply one and the project says otherwise, do not read past it — correct it in "#,
            r#"place with `nxm remember \"<the corrected fact>\" --key <key>`, or "#,
            r#"`nxm forget <key>` if it no longer holds at all. You are the best-informed "#,
            r#"corrector this memory will ever have.","#,
            r#""count":1,"#,
            r#""introduction_rule":"**Writing one:** `--introduction` is ONE line, at most 200 "#,
            r#"characters, and it says what the memory SAYS. Under `--category rules` it speaks "#,
            r#"the rule — \"Never X, always Y\", not \"Rules about X\" — because nobody looks a "#,
            r#"prohibition up before breaking it.","#,
            r#""memories":[{"active":true,"author":"alice","#,
            r#""body":"auth uses JWT, not sessions","category":"unsorted","#,
            r#""introduction":"the one line about auth-jwt","key":"auth-jwt","#,
            r#""ordinal":null,"refs":[],"scope":"project","#,
            r#""updated":"2026-06-20T10:00:00Z"}],"#,
            r#""memory_rule":"**CRITICAL — Memory rule:** store durable project knowledge "#,
            r#"(conventions, gotchas, decisions) **only** with `nxm remember`. **NEVER create a "#,
            r#"`MEMORY.md`** or any other ad-hoc memory file: nxs never reads it, so it is "#,
            r#"**never replayed** at session start and that knowledge is silently lost. "#,
            r#"`nxm remember` is the one durable channel (reuse a stable `--key` to evolve a fact "#,
            r#"in place).","migration":{"unsorted":1}}"#,
            "\n"
        )
    );
}
