//! `cargo xtask docs gen` / `docs check` (nexus-flow-4oa.1): the living-documentation
//! pipeline and its fail-closed drift gate. The foundation of Epic E7.
//!
//! The nxf clap command tree (`nexus_flow_cli::command()`) is the SINGLE source of truth
//! for the command reference — there is no hand-written second copy. `gen` renders that
//! tree into `docs/generated/`:
//!
//!   - `cli.md` — human Markdown reference (clap-markdown).
//!   - `man/nxf*.1` — one roff man-page per (sub)command (clap_mangen), deterministic
//!     tree-walk order.
//!   - `commands.json` — a machine-readable, deterministic command tree, versioned by the
//!     workspace version so a bump is visible in the diff.
//!
//! `check` regenerates into a temp dir and compares byte-for-byte against the committed
//! `docs/generated/`. ANY difference fails the build with the offending file named and the
//! fix spelled out — the gate is the reason `gen` output must be deterministic (stable key
//! order, sorted man-page walk). Committing the artifacts is what keeps CI green.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::version;

/// Directory (relative to the workspace root) that holds the committed generated docs.
const GENERATED_DIR: &str = "docs/generated";

/// Directory (relative to the workspace root) holding the golden trycmd corpus. Per-plugin
/// cases live in `<plugin>/` subdirs; plugin-agnostic cases sit directly inside.
const GOLDEN_DIR: &str = "crates/cli/tests/golden";

/// The two shipped declarative plugins. Coverage requires every non-exempt command to be
/// exercised under BOTH (E7 epic: "beide Plugin-Varianten"). Third-party plugins wait on E3.
const REQUIRED_PLUGINS: &[&str] = &["issue-tracker", "personal-todo"];

/// Commands deliberately exempt from the golden-example requirement: `setup claude` mutates host
/// config that persists outside the workspace with no reset step, so it cannot be reproduced in the
/// in-crate-dir golden harness. It is covered by hermetic integration tests instead; it still
/// carries a reference entry, and coverage prints it so the exemption is never a silent gap. (Sync
/// moved to the `nxs` umbrella in aye.2.1, and `self-update` became a hidden, deprecated alias in
/// nexus-flow-gel — both are gone from the documented `nxf` leaves, so neither needs a waiver.)
const EXEMPT: &[&str] = &["setup claude"];

/// A single argument in the machine-readable command tree. Field order here IS the JSON key
/// order (serde_json preserves struct order), so it is part of the stable contract.
#[derive(Serialize)]
struct ArgNode {
    name: String,
    /// `--long` flag, if any (without the leading dashes).
    long: Option<String>,
    /// `-s` short flag, if any.
    short: Option<String>,
    required: bool,
    takes_value: bool,
    help: Option<String>,
}

/// A command (or subcommand) in the machine-readable tree. Recursive.
#[derive(Serialize)]
struct CommandNode {
    name: String,
    about: Option<String>,
    args: Vec<ArgNode>,
    subcommands: Vec<CommandNode>,
}

/// The top-level JSON document. The `version` header couples the generated artifact to the
/// workspace version: a version bump shows up as a one-line diff here, so the drift gate
/// forces a regenerate-and-commit on every release.
#[derive(Serialize)]
struct CommandTreeDoc {
    /// Workspace version (`[workspace.package].version`) at generation time.
    version: String,
    command: CommandNode,
}

/// Build the recursive command node for `cmd`. Args are emitted in clap's declaration order
/// (deterministic); subcommands are sorted by name so the JSON is order-independent of how
/// clap happens to enumerate them.
fn command_node(cmd: &clap::Command) -> CommandNode {
    let args = cmd
        .get_arguments()
        // The auto-injected `--help` / `--version` carry no stable contract and clap may add
        // or reorder them across versions — exclude them so the JSON stays about real flags.
        .filter(|a| a.get_id() != "help" && a.get_id() != "version")
        .map(|a| ArgNode {
            name: a.get_id().as_str().to_string(),
            long: a.get_long().map(str::to_string),
            short: a.get_short().map(|c| c.to_string()),
            required: a.is_required_set(),
            // `get_action().takes_values()` is the robust signal (Set/Append → true; the
            // boolean SetTrue/SetFalse/Count flags → false). `get_num_args()` is `None` for
            // args whose range is still defaulted, so it would under-report value-taking flags.
            takes_value: a.get_action().takes_values(),
            help: a.get_help().map(|h| h.to_string()),
        })
        .collect();

    // Hidden subcommands (`#[command(hide = true)]`) carry no user docs — exclude them so the JSON
    // reference matches `cli.md` and the coverage gate, which already skip them (nexus-flow-fyr/gel).
    let mut subcommands: Vec<CommandNode> = cmd
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
        .map(command_node)
        .collect();
    subcommands.sort_by(|a, b| a.name.cmp(&b.name));

    CommandNode {
        name: cmd.get_name().to_string(),
        about: cmd.get_about().map(|s| s.to_string()),
        args,
        subcommands,
    }
}

/// Collect every (sub)command in the tree as `(file_stem, command)` pairs for man-page
/// generation. The file stem is the dash-joined command path (`nxf`, `nxf-init`,
/// `nxf-dep-add`, …). Output is sorted by stem so the on-disk set + content is deterministic.
/// Hidden subcommands (and their subtrees) are skipped — a `#[command(hide = true)]` verb carries
/// no user docs, so it gets no man page, matching `cli.md`/`commands.json` (nexus-flow-fyr/gel).
fn man_targets(cmd: &clap::Command) -> Vec<(String, clap::Command)> {
    fn walk(prefix: &str, cmd: &clap::Command, out: &mut Vec<(String, clap::Command)>) {
        let stem = if prefix.is_empty() {
            cmd.get_name().to_string()
        } else {
            format!("{prefix}-{}", cmd.get_name())
        };
        // Clone so clap_mangen can take ownership per page (it consumes the Command).
        out.push((stem.clone(), cmd.clone()));
        for sub in cmd.get_subcommands() {
            if sub.is_hide_set() {
                continue;
            }
            walk(&stem, sub, out);
        }
    }
    let mut out = Vec::new();
    walk("", cmd, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The full set of files `gen` produces, as (relative path under `GENERATED_DIR`, bytes).
/// `gen` and `check` both go through this single function so they can never diverge.
fn render_all(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let version = version::workspace_version(root).context("reading workspace version for docs")?;
    // Stamp the command tree (the man-page `.TH` / `--version` header) from the RUNTIME workspace
    // version, NOT the `CARGO_PKG_VERSION` baked into this already-built binary (aye.39). `version
    // set` bumps Cargo.toml then calls gen() in the same process, so without this override the man
    // pages keep the pre-bump version until xtask is rebuilt — exactly the drift that failed the
    // v0.6.0 release PR. commands.json already uses the runtime `version`; this aligns the rest.
    // clap interns the version as a `'static` string, so leak the (tiny, once-per-process) runtime
    // value to satisfy that bound — xtask is a short-lived CLI, so the leak is immaterial.
    let version_static: &'static str = Box::leak(version.clone().into_boxed_str());
    let cmd = nexus_flow_cli::command().version(version_static);

    let mut files: Vec<(PathBuf, Vec<u8>)> = Vec::new();

    // 1. Markdown reference.
    let md = clap_markdown::help_markdown_command(&cmd);
    files.push((PathBuf::from("cli.md"), md.into_bytes()));

    // 2. Machine-readable command tree, version-stamped. Serializing the `#[derive(Serialize)]`
    //    structs DIRECTLY (not via `serde_json::Value`) writes fields in declaration order, so
    //    struct field order == JSON key order (the stable contract) — no `preserve_order` feature
    //    needed (and it is deliberately NOT enabled; see xtask/Cargo.toml for why).
    let doc = CommandTreeDoc {
        version,
        command: command_node(&cmd),
    };
    let mut json = serde_json::to_vec_pretty(&doc).context("serializing commands.json")?;
    json.push(b'\n'); // trailing newline so the committed file is POSIX-clean
    files.push((PathBuf::from("commands.json"), json));

    // 3. Man-pages, one per (sub)command, in deterministic tree-walk order. Unlike commands.json
    //    (which filters out --help/--version), these pages carry clap's auto-managed --help and
    //    --version boilerplate verbatim — the clap tree is the single source of truth. So a future
    //    clap upgrade that rewords that text will surface as drift across all ~26 pages at once;
    //    that is intentional/expected (regenerate and commit), not a bug in this pipeline.
    for (stem, sub) in man_targets(&cmd) {
        let man = clap_mangen::Man::new(sub);
        let mut buf: Vec<u8> = Vec::new();
        man.render(&mut buf)
            .with_context(|| format!("rendering man-page for {stem}"))?;
        files.push((PathBuf::from("man").join(format!("{stem}.1")), buf));
    }

    Ok(files)
}

/// `cargo xtask docs gen`: (re)write `docs/generated/` from the clap command tree.
///
/// The `man/` subtree is wiped first so a removed subcommand cannot leave a stale page
/// behind (which `check` would otherwise never flag, since it only compares files `gen`
/// emits — see the orphan check there for the converse direction).
pub fn gen(root: &Path) -> Result<()> {
    let base = root.join(GENERATED_DIR);
    let man_dir = base.join("man");
    if man_dir.exists() {
        fs::remove_dir_all(&man_dir)
            .with_context(|| format!("clearing stale man dir {}", man_dir.display()))?;
    }

    let files = render_all(root)?;
    for (rel, bytes) in &files {
        let path = base.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// `cargo xtask docs check`: the fail-closed drift gate. Regenerate in memory and compare
/// byte-for-byte against the committed `docs/generated/`; bail (non-zero exit) on any
/// missing, extra, or differing file, naming the offender and the fix.
pub fn check(root: &Path) -> Result<()> {
    let base = root.join(GENERATED_DIR);
    if !base.exists() {
        bail!(
            "{} is missing — run `cargo xtask docs gen` and commit the result",
            base.display()
        );
    }

    let files = render_all(root)?;

    // Forward direction: every freshly-generated file must exist and match on disk.
    let mut expected_paths = BTreeSet::new();
    for (rel, bytes) in &files {
        expected_paths.insert(rel.clone());
        let path = base.join(rel);
        let on_disk = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => bail!(
                "docs drift: {} is missing — run `cargo xtask docs gen` and commit the result",
                path.display()
            ),
        };
        if &on_disk != bytes {
            bail!(
                "docs drift: {} is stale — run `cargo xtask docs gen` and commit the result",
                path.display()
            );
        }
    }

    // Reverse direction: a committed man-page with no generating command is an orphan (e.g. a
    // subcommand was deleted). `gen` clears man/ so this only bites a hand-edit, but the gate
    // must still catch it fail-closed rather than silently tolerate the extra file.
    let man_dir = base.join("man");
    if man_dir.exists() {
        for entry in
            fs::read_dir(&man_dir).with_context(|| format!("reading {}", man_dir.display()))?
        {
            let path = entry?.path();
            let rel = PathBuf::from("man").join(path.file_name().unwrap());
            if !expected_paths.contains(&rel) {
                bail!(
                    "docs drift: {} is an orphan (no such command) — run `cargo xtask docs gen` \
                     and commit the result",
                    path.display()
                );
            }
        }
    }

    Ok(())
}

// ---- docs coverage (nexus-flow-4oa.2) --------------------------------------
//
// The fail-closed coverage gate. Every non-hidden leaf subcommand must have (a) a reference
// entry in the generated `commands.json`, (b) at least one golden example, and (c) a golden
// example under BOTH shipped plugins. The clap tree is the single source of the command set,
// so a command added/renamed without a matching example turns this gate red.

/// Every non-hidden LEAF command path in the tree, space-joined (`init`, `dep add`, `note
/// list`, …). A leaf is a command with no subcommands; group parents (`dep`, `note`) are not
/// leaves. Hidden commands (and their subtrees) are excluded — they carry no user docs.
fn leaf_command_paths(root: &clap::Command) -> BTreeSet<String> {
    fn walk(cmd: &clap::Command, prefix: &[String], out: &mut BTreeSet<String>) {
        for sub in cmd.get_subcommands() {
            if sub.is_hide_set() {
                continue;
            }
            let mut path = prefix.to_vec();
            path.push(sub.get_name().to_string());
            if sub.get_subcommands().next().is_none() {
                out.insert(path.join(" "));
            } else {
                walk(sub, &path, out);
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(root, &[], &mut out);
    out
}

/// Names of the non-hidden top-level commands — the set used to find where a subcommand starts
/// in an example line (so a flag value can never be mistaken for the command).
fn top_level_names(root: &clap::Command) -> BTreeSet<String> {
    root.get_subcommands()
        .filter(|s| !s.is_hide_set())
        .map(|s| s.get_name().to_string())
        .collect()
}

/// Names of the non-hidden top-level commands that GROUP subcommands (`dep`, `mention`, `note`,
/// `sync`) — needed to resolve a two-token leaf path (`dep add`) when parsing example lines.
fn group_command_names(root: &clap::Command) -> BTreeSet<String> {
    root.get_subcommands()
        .filter(|s| !s.is_hide_set() && s.get_subcommands().next().is_some())
        .map(|s| s.get_name().to_string())
        .collect()
}

/// The leaf command paths invoked by `$ nxf …` prompt lines in a golden file. Only executable
/// prompt lines count (a bare prose mention of a command does not), matching exactly what
/// trycmd runs. Flags are skipped; a two-token nested command is resolved via `groups`.
fn invoked_leaves(
    content: &str,
    top_level: &BTreeSet<String>,
    groups: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in content.lines() {
        let Some(rest) = line.trim().strip_prefix("$ nxf ") else {
            continue;
        };
        // Non-flag tokens, in order. The subcommand is taken as the first one that NAMES a
        // top-level command — the "subcommand-leads" assumption, true for `nxf <cmd> …` and
        // `nxf --json <cmd> …`. Scanning for a recognized name (rather than taking token 0)
        // keeps a leading boolean global flag from being misread as the command. The one blind
        // spot is a value-taking global flag whose VALUE equals a command name (only `--db
        // <name>` qualifies) placed BEFORE the subcommand; the golden corpus uses neither `--db`
        // nor such values, so coverage can't be mis-credited here. If that ever changes, parse
        // the clap tree instead of this line heuristic.
        let nonflag: Vec<&str> = rest
            .split_whitespace()
            .filter(|t| !t.starts_with('-'))
            .collect();
        let Some(pos) = nonflag.iter().position(|t| top_level.contains(*t)) else {
            continue;
        };
        let head = nonflag[pos];
        if groups.contains(head) {
            if let Some(tail) = nonflag.get(pos + 1) {
                out.insert(format!("{head} {tail}"));
            }
        } else {
            out.insert(head.to_string());
        }
    }
    out
}

/// Human-readable coverage gaps: for every required (non-exempt) leaf, every required plugin
/// that has no golden example. Empty result ⇒ full coverage.
fn coverage_gaps(
    leaves: &BTreeSet<String>,
    coverage: &BTreeMap<String, BTreeSet<String>>,
    required_plugins: &[&str],
    exempt: &BTreeSet<String>,
) -> Vec<String> {
    let mut gaps = Vec::new();
    for leaf in leaves {
        if exempt.contains(leaf) {
            continue;
        }
        for plugin in required_plugins {
            let covered = coverage
                .get(leaf)
                .is_some_and(|plugins| plugins.contains(*plugin));
            if !covered {
                gaps.push(format!(
                    "`nxf {leaf}` has no golden example under the `{plugin}` plugin"
                ));
            }
        }
    }
    gaps
}

/// Leaf command paths present in the generated `commands.json` tree (used for the reference
/// check). Mirrors [`leaf_command_paths`] but reads the JSON document rather than the live tree.
fn json_leaf_paths(doc: &serde_json::Value) -> BTreeSet<String> {
    fn walk(node: &serde_json::Value, prefix: &[String], out: &mut BTreeSet<String>) {
        let subs = node.get("subcommands").and_then(|s| s.as_array());
        match subs {
            Some(arr) if !arr.is_empty() => {
                for sub in arr {
                    let name = sub.get("name").and_then(|n| n.as_str()).unwrap_or_default();
                    let mut path = prefix.to_vec();
                    path.push(name.to_string());
                    walk(sub, &path, out);
                }
            }
            // No subcommands ⇒ a leaf. The root (empty prefix) is not a command path.
            _ => {
                if !prefix.is_empty() {
                    out.insert(prefix.join(" "));
                }
            }
        }
    }
    let mut out = BTreeSet::new();
    if let Some(root) = doc.get("command") {
        walk(root, &[], &mut out);
    }
    out
}

/// `*.trycmd` files directly inside `dir` (non-recursive, sorted). A missing dir yields none.
fn trycmd_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) == Some("trycmd") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// `cargo xtask docs coverage`: the fail-closed living-docs coverage gate (nexus-flow-4oa.2).
///
/// For every non-hidden leaf command it asserts (a) a reference entry exists in
/// `commands.json`, and (b)+(c) a golden trycmd example exercises it under BOTH shipped
/// plugins. Plugin-agnostic cases (directly under the golden dir) count for both plugins;
/// per-plugin cases (in a `<plugin>/` subdir) count for that plugin. `EXEMPT` network/release
/// commands are waived from the example requirement and printed so the waiver is visible.
pub fn coverage(root: &Path) -> Result<()> {
    let cmd = nexus_flow_cli::command();
    let leaves = leaf_command_paths(&cmd);
    let top_level = top_level_names(&cmd);
    let groups = group_command_names(&cmd);
    let exempt: BTreeSet<String> = EXEMPT.iter().map(|s| s.to_string()).collect();

    // (a) Reference entry: every non-hidden leaf must appear in the generated command tree.
    let json_path = root.join(GENERATED_DIR).join("commands.json");
    let json_raw = fs::read_to_string(&json_path).with_context(|| {
        format!(
            "reading {} — run `cargo xtask docs gen` first",
            json_path.display()
        )
    })?;
    let doc: serde_json::Value =
        serde_json::from_str(&json_raw).context("parsing commands.json")?;
    let reference = json_leaf_paths(&doc);
    let missing_reference: Vec<&String> =
        leaves.iter().filter(|l| !reference.contains(*l)).collect();
    if !missing_reference.is_empty() {
        bail!(
            "docs coverage: no reference entry for {missing_reference:?} — run `cargo xtask docs gen` and commit"
        );
    }

    // (b)+(c) Golden examples, per plugin.
    let golden = root.join(GOLDEN_DIR);
    let mut coverage_map: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    // Plugin-agnostic cases count for every required plugin.
    for path in trycmd_files(&golden)? {
        let content =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        for leaf in invoked_leaves(&content, &top_level, &groups) {
            let plugins = coverage_map.entry(leaf).or_default();
            for plugin in REQUIRED_PLUGINS {
                plugins.insert(plugin.to_string());
            }
        }
    }
    // Per-plugin cases count for their own plugin.
    for plugin in REQUIRED_PLUGINS {
        for path in trycmd_files(&golden.join(plugin))? {
            let content =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            for leaf in invoked_leaves(&content, &top_level, &groups) {
                coverage_map
                    .entry(leaf)
                    .or_default()
                    .insert(plugin.to_string());
            }
        }
    }

    let gaps = coverage_gaps(&leaves, &coverage_map, REQUIRED_PLUGINS, &exempt);
    if !gaps.is_empty() {
        bail!(
            "docs coverage failed (fail-closed) — add the missing golden example(s):\n  - {}",
            gaps.join("\n  - ")
        );
    }

    // Make the exemptions visible — a waived command is never a silent coverage hole.
    for leaf in EXEMPT {
        println!(
            "docs coverage: `nxf {leaf}` exempt from the golden requirement \
             (network/release or host-config; covered by hermetic integration tests)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace root, derived from this crate's manifest dir (`<root>/xtask`).
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// `render_all` is a pure function of the clap tree + workspace version, so two calls must
    /// produce byte-identical output — the determinism the drift gate depends on.
    #[test]
    fn render_is_deterministic() {
        let root = repo_root();
        let a = render_all(&root).unwrap();
        let b = render_all(&root).unwrap();
        assert_eq!(a, b, "render_all must be byte-stable across calls");
    }

    /// aye.39: the man-page `.TH` version must come from the RUNTIME workspace version (Cargo.toml),
    /// not the version baked into this xtask binary at compile time. `cargo xtask version set` bumps
    /// Cargo.toml then calls `gen()` in the SAME process, so a compile-time stamp would leave the
    /// man pages a version behind (the drift that failed the v0.6.0 release PR). Stage a sentinel
    /// version distinct from this binary's and prove the generated man page carries it.
    #[test]
    fn man_page_version_tracks_the_runtime_workspace_version() {
        let dir = tempfile::tempdir().unwrap();
        let current = version::workspace_version(&repo_root()).unwrap();
        let sentinel = "9.9.99";
        assert_ne!(
            current, sentinel,
            "sentinel must differ from the real version"
        );
        let staged = fs::read_to_string(repo_root().join("Cargo.toml"))
            .unwrap()
            .replace(
                &format!("version = \"{current}\""),
                &format!("version = \"{sentinel}\""),
            );
        fs::write(dir.path().join("Cargo.toml"), staged).unwrap();

        gen(dir.path()).unwrap();
        let nxf_man =
            fs::read_to_string(dir.path().join(GENERATED_DIR).join("man").join("nxf.1")).unwrap();
        let th = nxf_man
            .lines()
            .find(|l| l.starts_with(".TH"))
            .unwrap_or("<no .TH>");
        assert!(
            th.contains(&format!("nxf {sentinel}")),
            "man .TH must carry the runtime workspace version: {th}"
        );
    }

    /// gen → check round-trips: a freshly-genned tree passes the gate.
    #[test]
    fn check_passes_on_freshly_genned_output() {
        let dir = tempfile::tempdir().unwrap();
        // Symlink/point gen at a temp `docs/generated` by using the temp dir as root, but the
        // command tree + version come from the real workspace via render_all → version. Stage
        // the same Cargo.toml the version reader needs.
        stage_root(dir.path());
        gen(dir.path()).expect("gen succeeds");
        check(dir.path()).expect("check passes on freshly-genned output");
    }

    /// A single mutated byte in any generated file fails the gate fail-closed.
    #[test]
    fn check_fails_on_a_mutated_file() {
        let dir = tempfile::tempdir().unwrap();
        stage_root(dir.path());
        gen(dir.path()).unwrap();

        let cli_md = dir.path().join(GENERATED_DIR).join("cli.md");
        let mut body = fs::read_to_string(&cli_md).unwrap();
        body.push_str("\n<!-- drift -->\n");
        fs::write(&cli_md, body).unwrap();

        let err = check(dir.path()).expect_err("a mutated file must fail check");
        assert!(
            err.to_string().contains("cli.md"),
            "error should name the offending file, got: {err}"
        );
    }

    /// A missing `docs/generated/` dir fails (not panics).
    #[test]
    fn check_fails_when_generated_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        stage_root(dir.path());
        let err = check(dir.path()).expect_err("missing generated dir must fail");
        assert!(err.to_string().contains("docs gen"));
    }

    /// An orphan man-page (no generating command) is caught fail-closed.
    #[test]
    fn check_fails_on_orphan_man_page() {
        let dir = tempfile::tempdir().unwrap();
        stage_root(dir.path());
        gen(dir.path()).unwrap();
        let orphan = dir
            .path()
            .join(GENERATED_DIR)
            .join("man")
            .join("nxf-ghost.1");
        fs::write(&orphan, b".TH ghost\n").unwrap();
        let err = check(dir.path()).expect_err("orphan man-page must fail check");
        assert!(err.to_string().contains("orphan"));
    }

    /// The live single-source assertion: the committed `docs/generated/` matches what the
    /// current clap tree generates. If this fails, someone changed an nxf command without
    /// running `cargo xtask docs gen` — exactly the drift the CI gate prevents.
    #[test]
    fn committed_docs_are_up_to_date() {
        check(&repo_root()).expect("committed docs/generated matches the clap tree");
    }

    /// Stage the minimal root the version reader needs (the real workspace Cargo.toml), so a
    /// temp-dir `gen`/`check` can read `[workspace.package].version` without the rest of the repo.
    fn stage_root(dst: &Path) {
        let src = repo_root().join("Cargo.toml");
        fs::copy(&src, dst.join("Cargo.toml")).unwrap();
    }

    // ---- docs coverage (nexus-flow-4oa.2) ----------------------------------

    /// A hidden subcommand (`#[command(hide = true)]`) carries no user docs, so it must be excluded
    /// from EVERY generated artifact — not just `cli.md`/coverage (which already skip it) but also
    /// `commands.json` and the man-page corpus (nexus-flow-fyr/gel). These two assert the parts that
    /// previously leaked hidden commands.
    #[test]
    fn command_node_excludes_hidden_subcommands() {
        let cmd = clap::Command::new("t")
            .subcommand(clap::Command::new("visible"))
            .subcommand(
                clap::Command::new("group")
                    .subcommand(clap::Command::new("secret").hide(true))
                    .subcommand(clap::Command::new("shown")),
            )
            .subcommand(clap::Command::new("hush").hide(true));
        let node = command_node(&cmd);
        let names: Vec<&str> = node.subcommands.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["group", "visible"],
            "hidden top-level command excluded from commands.json: {names:?}"
        );
        let group = node.subcommands.iter().find(|s| s.name == "group").unwrap();
        let group_subs: Vec<&str> = group.subcommands.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            group_subs,
            vec!["shown"],
            "hidden nested command excluded too: {group_subs:?}"
        );
    }

    #[test]
    fn man_targets_exclude_hidden_subcommands() {
        let cmd = clap::Command::new("t")
            .subcommand(clap::Command::new("visible"))
            .subcommand(clap::Command::new("secret").hide(true));
        let stems: Vec<String> = man_targets(&cmd).into_iter().map(|(s, _)| s).collect();
        assert!(
            stems.contains(&"t".to_string()),
            "root page kept: {stems:?}"
        );
        assert!(
            stems.contains(&"t-visible".to_string()),
            "visible command keeps its page: {stems:?}"
        );
        assert!(
            !stems.iter().any(|s| s.contains("secret")),
            "hidden command gets no man page: {stems:?}"
        );
    }

    /// Leaf enumeration walks the real clap tree: nested two-token leaves are included, group
    /// parents are not leaves, and `#[command(hide = true)]` commands are excluded entirely.
    #[test]
    fn leaf_paths_include_nested_and_exclude_hidden() {
        let cmd = nexus_flow_cli::command();
        let leaves = leaf_command_paths(&cmd);
        for expected in [
            "init",
            "create",
            "dep add",
            "dep remove",
            "note list",
            "mention add",
            "guide",
        ] {
            assert!(
                leaves.contains(expected),
                "missing leaf {expected}: {leaves:?}"
            );
        }
        // The hidden plumbing command is not a required leaf.
        assert!(
            !leaves.contains("verify-signature"),
            "hidden command must be excluded: {leaves:?}"
        );
        // Group parents are not themselves leaves (only their subcommands are).
        assert!(!leaves.contains("dep") && !leaves.contains("note"));
    }

    /// Invocation parsing reads `$ nxf …` prompt lines, resolves two-token nested commands,
    /// skips flags, and ignores prose / non-prompt mentions of a command.
    #[test]
    fn invoked_leaves_resolves_nested_and_ignores_prose() {
        let cmd = nexus_flow_cli::command();
        let top = top_level_names(&cmd);
        let groups = group_command_names(&cmd);
        let content = "\
Prose that casually mentions nxf create without a prompt is not a command line.

```console
$ nxf init --plugin issue-tracker --json
$ nxf dep add ab12.0001 ab12.0002 --json
$ nxf note list ab12.0001
```
";
        let found = invoked_leaves(content, &top, &groups);
        assert!(found.contains("init"));
        assert!(found.contains("dep add"));
        assert!(found.contains("note list"));
        // The bare prose mention of "nxf create" (no `$ ` prompt) is NOT counted.
        assert!(
            !found.contains("create"),
            "prose must not count as coverage: {found:?}"
        );
    }

    /// The gap report flags a command missing a plugin variant and honours the exempt set.
    #[test]
    fn coverage_gaps_flags_missing_plugin_and_respects_exemptions() {
        let leaves: BTreeSet<String> = ["create", "sync run"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut cov: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        // `create` is covered only under issue-tracker → a personal-todo gap.
        cov.insert(
            "create".into(),
            ["issue-tracker".to_string()].into_iter().collect(),
        );
        let exempt: BTreeSet<String> = ["sync run".to_string()].into_iter().collect();
        let gaps = coverage_gaps(&leaves, &cov, &["issue-tracker", "personal-todo"], &exempt);
        assert_eq!(gaps.len(), 1, "exactly one gap expected: {gaps:?}");
        assert!(gaps[0].contains("create") && gaps[0].contains("personal-todo"));
    }

    #[test]
    fn coverage_gaps_empty_when_all_covered() {
        let leaves: BTreeSet<String> = ["create"].iter().map(|s| s.to_string()).collect();
        let mut cov: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        cov.insert(
            "create".into(),
            ["issue-tracker".to_string(), "personal-todo".to_string()]
                .into_iter()
                .collect(),
        );
        let gaps = coverage_gaps(
            &leaves,
            &cov,
            &["issue-tracker", "personal-todo"],
            &BTreeSet::new(),
        );
        assert!(gaps.is_empty(), "fully covered → no gaps: {gaps:?}");
    }

    /// The live gate: the committed golden corpus covers every non-hidden, non-exempt command
    /// under BOTH shipped plugins, and every command has a reference entry. If this fails, a
    /// command was added/renamed without a golden example — exactly the gap the gate prevents.
    #[test]
    fn committed_corpus_fully_covers_every_command() {
        coverage(&repo_root())
            .expect("golden corpus covers every non-hidden command under both plugins");
    }
}
