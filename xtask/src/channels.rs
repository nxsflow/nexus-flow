//! `cargo xtask channels check` (nexus-flow-85y.27): enforce a single source of truth for the
//! three promotion-ring channels.
//!
//! `release/channels` is canonical. The ring (`stable|beta|alpha`) was duplicated across the CLI
//! self-update module, the clap `--channel` arg parser and install.sh's `NXF_CHANNEL` doc comment
//! — each independently maintained. This check reads the canonical file and asserts every copy
//! agrees: `selfupdate.rs`'s `const CHANNELS = [...]` array, the clap `value_parser = [...]` for
//! `--channel`, and install.sh's `(stable|beta|alpha)` doc token alternation must each equal the
//! canonical set. Drift fails CI fail-closed, naming every disagreeing consumer.
//!
//! A FIFTH CONSUMER USED TO BE CHECKED HERE AND IS NOT ANY MORE (nexus-flow-6j6v.sgfr): the
//! updater Lambda's `download/(stable|beta|alpha)/…` path regex, read out of
//! `infra/lambda/updater/decide.ts`. That tree left this repo with the delivery move, and the
//! copy this check was reading had not been the SHIPPED one since — so the assertion had become
//! a green tick on a file nobody deploys. The shipped updater now lives in
//! `nxsflow-landing-page` and is outside anything this repo can see; `release/channels`' own
//! header says so, rather than leaving a reader to infer that the ring is guarded end to end.
//!
//! This is the exact channel analogue of `platforms.rs` (nexus-flow-4sr).

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

/// Parse the canonical `release/channels` body: non-blank, non-comment lines, trimmed.
pub fn parse_channels(contents: &str) -> Vec<String> {
    contents
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Extract the elements of the first bracketed list-of-quoted-strings that appears after
/// `marker`: e.g. for marker `CHANNELS` in `const CHANNELS: [&str; 3] = ["stable", "beta",
/// "alpha"];` returns `[stable, beta, alpha]` (the `[&str; 3]` type bracket carries no quoted
/// strings and is skipped), and for marker `value_parser` in `value_parser = ["stable", ...]`
/// returns the same. Returns `None` if `marker` is absent or no `[...]` of quoted strings
/// follows it.
///
/// The marker is anchored to the actual *declaration*: a `marker` mention inside a comment
/// line (Rust `//`/`///`/`*` doc, shell `#`) is skipped, so a prose reference like
/// `// the value_parser below lists ["x"]` before the real declaration can't redirect the
/// parse. We locate the marker on the first non-comment line that contains it, then read the
/// first quoted-string bracket from there on.
pub fn bracketed_string_list(contents: &str, marker: &str) -> Option<Vec<String>> {
    let marker_at = marker_offset_skipping_comments(contents, marker)?;
    let mut rest = &contents[marker_at + marker.len()..];
    while let Some(open) = rest.find('[') {
        let after_open = &rest[open + 1..];
        let close = after_open.find(']')?;
        let elems = quoted_strings(&after_open[..close]);
        if !elems.is_empty() {
            return Some(elems);
        }
        rest = &after_open[close + 1..];
    }
    None
}

/// Byte offset of `marker` on the first line that contains it *and* is not a comment. We treat
/// a line whose first non-whitespace runs with `//`, `#`, or `*` (continuation of a Rust block
/// comment / doc comment) as a comment. This keeps a prose mention of the marker from anchoring
/// the parse ahead of the genuine declaration line.
fn marker_offset_skipping_comments(contents: &str, marker: &str) -> Option<usize> {
    let mut line_start = 0usize;
    for line in contents.split_inclusive('\n') {
        if let Some(col) = line.find(marker) {
            let lead = line.trim_start();
            // Comment leads: Rust line/doc (`//`, `///`), Rust block continuation (`*`), shell
            // (`#`). A Rust *attribute* leads with `#[`/`#!` — that is NOT a comment (it's where
            // the real `value_parser = [..]` declaration lives), so exclude it explicitly.
            let is_shell_comment =
                lead.starts_with('#') && !lead.starts_with("#[") && !lead.starts_with("#!");
            let is_comment = lead.starts_with("//") || lead.starts_with('*') || is_shell_comment;
            if !is_comment {
                return Some(line_start + col);
            }
        }
        line_start += line.len();
    }
    None
}

/// Collect every double-quoted string within `s`, in order (no escape handling needed: channel
/// names are plain identifiers).
fn quoted_strings(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else { break };
        out.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    out
}

/// Extract a `(a|b|c)` alternation, in order, from the first parenthesised pipe-list that
/// appears *after* `anchor` in `contents`. Today's one caller is install.sh's `NXF_CHANNEL` doc
/// token; the function stays anchor-parameterised because that is what makes it safe, not
/// because a second caller is expected back (the other one was decide.ts's download-path regex,
/// removed with the infra teardown, nexus-flow-6j6v.sgfr).
///
/// `anchor` locks the scan onto the *real* consumer construct: a coincidental earlier
/// `(stable|beta|alpha)` (e.g. in an unrelated doc comment or regex) would otherwise be matched
/// and let genuine drift in the real construct pass. Returns `None` if `anchor` is absent or no
/// channel-shaped group follows it.
pub fn pipe_alternation(contents: &str, anchor: &str) -> Option<Vec<String>> {
    let anchor_at = contents.find(anchor)?;
    let scan_from = anchor_at + anchor.len();
    for (rel, _) in contents[scan_from..].match_indices('(') {
        let i = scan_from + rel;
        let after = &contents[i + 1..];
        let Some(close) = after.find(')') else {
            continue;
        };
        let inner = &after[..close];
        if !inner.contains('|') {
            continue;
        }
        let parts: Vec<String> = inner.split('|').map(|p| p.trim().to_string()).collect();
        // Only accept a group whose parts are all plain identifiers (the channel shape); skip
        // regex groups like `(?:...)` or character-class fragments.
        if parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()))
        {
            return Some(parts);
        }
    }
    None
}

// ---- orchestration (reads the real repo files) --------------------------------------------

fn read(root: &Path, rel: &str) -> Result<String> {
    let path = root.join(rel);
    std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
}

/// First duplicated element in `items`, if any. The channel list is a promotion ring: a repeat
/// is never legitimate (it would silently widen the ring or hide a typo), so we reject it
/// outright rather than dedup-and-compare.
fn first_duplicate(items: &[String]) -> Option<&str> {
    for (i, a) in items.iter().enumerate() {
        if items[..i].iter().any(|b| b == a) {
            return Some(a);
        }
    }
    None
}

fn load_canonical(root: &Path) -> Result<Vec<String>> {
    let path = root.join("release/channels");
    let body = std::fs::read_to_string(&path)
        .with_context(|| format!("reading canonical channel list {}", path.display()))?;
    let channels = parse_channels(&body);
    if channels.is_empty() {
        bail!("{} lists no channels", path.display());
    }
    if let Some(dup) = first_duplicate(&channels) {
        bail!("{} lists channel '{dup}' more than once", path.display());
    }
    Ok(channels)
}

/// Compare one consumer's channel list against the canonical ring. Order is load-bearing (the
/// list is a promotion ring, most-stable first), so this is an ORDER-PRESERVING `Vec` equality,
/// not a set comparison — `["stable","beta","alpha"]` and `["beta","stable","alpha"]` are NOT
/// equal. A duplicate in the consumer is rejected outright (always a smell). `consumer` names
/// the file and `what` the construct, both surfaced in the bail message.
fn assert_consumer_matches(
    consumer: &str,
    what: &str,
    found: &[String],
    canon: &[String],
) -> Result<()> {
    if let Some(dup) = first_duplicate(found) {
        bail!("{consumer}: {what} lists channel '{dup}' more than once");
    }
    if found != canon {
        bail!("{consumer}: {what} disagrees with release/channels (order-sensitive)");
    }
    Ok(())
}

/// Extract every consumer's channel list, paired with a label, so `check` and the drift tests
/// can iterate the same set of consumers. Returns an error if any consumer's construct can't be
/// located (a missing construct is itself drift). The returned order is the orchestration order.
fn extract_consumers(root: &Path) -> Result<Vec<(&'static str, &'static str, Vec<String>)>> {
    let mut out = Vec::new();

    // 1. The CLI self-update module's `CHANNELS` array.
    let selfupdate = read(root, "crates/cli/src/selfupdate.rs")?;
    let cli_channels = bracketed_string_list(&selfupdate, "CHANNELS")
        .ok_or_else(|| anyhow!("crates/cli/src/selfupdate.rs: no `CHANNELS = [..]` array found"))?;
    out.push((
        "crates/cli/src/selfupdate.rs",
        "CHANNELS array",
        cli_channels,
    ));

    // 2. The clap `--channel` value_parser list. Lives in lib.rs since the cli lib/bin split
    //    (nexus-flow-4oa.1) moved the parser definition out of main.rs.
    let lib_rs = read(root, "crates/cli/src/lib.rs")?;
    let value_parser = bracketed_string_list(&lib_rs, "value_parser").ok_or_else(|| {
        anyhow!("crates/cli/src/lib.rs: no `value_parser = [..]` list for --channel found")
    })?;
    out.push((
        "crates/cli/src/lib.rs",
        "--channel value_parser",
        value_parser,
    ));

    // 3. install.sh's `(stable|beta|alpha)` doc token — anchored to `NXF_CHANNEL` so an unrelated
    //    earlier alternation can't be mistaken for it.
    let install_sh = read(root, "install.sh")?;
    let install_alt = pipe_alternation(&install_sh, "NXF_CHANNEL")
        .ok_or_else(|| anyhow!("install.sh: no `(a|b|c)` channel alternation after NXF_CHANNEL"))?;
    out.push(("install.sh", "NXF_CHANNEL doc token", install_alt));

    Ok(out)
}

/// Run the full single-source check against the repo at `root`. Returns an error naming the
/// first consumer that disagrees with `release/channels`.
pub fn check(root: &Path) -> Result<()> {
    let canon = load_canonical(root)?;
    for (consumer, what, found) in extract_consumers(root)? {
        assert_consumer_matches(consumer, what, &found, &canon)?;
    }
    // The npx runner shim (npm/mcp) does not enumerate the ring — it accepts any NXF_CHANNEL and
    // hardcodes only the DEFAULT_CHANNEL. Guard that default against the canonical most-stable ring
    // (release/channels is most-stable-first), so a rename of the top channel can't leave the shim
    // defaulting to a channel that no longer exists.
    let shim = read(root, "npm/mcp/lib/constants.js")?;
    let expected = format!("const DEFAULT_CHANNEL = '{}';", canon[0]);
    if !shim.contains(&expected) {
        bail!(
            "npm/mcp/lib/constants.js: DEFAULT_CHANNEL must be the canonical most-stable channel '{}'",
            canon[0]
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The workspace root, derived from this crate's manifest dir (`<root>/xtask`).
    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf()
    }

    /// Write a minimal but COMPLETE fixture repo where every consumer lists exactly `channels`.
    fn write_fixture(root: &Path, channels: &[&str]) {
        let w = |rel: &str, body: String| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        let quoted = channels
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let alt = channels.join("|");
        w(
            "release/channels",
            format!("# canon\n{}\n", channels.join("\n")),
        );
        w(
            "crates/cli/src/selfupdate.rs",
            format!("const CHANNELS: [&str; {}] = [{quoted}];\n", channels.len()),
        );
        w(
            "crates/cli/src/lib.rs",
            format!("        #[arg(long, value_parser = [{quoted}])]\n"),
        );
        w("install.sh", format!("#   NXF_CHANNEL  ring ({alt})\n"));
        // The npx shim: only the DEFAULT_CHANNEL (the canonical most-stable, i.e. channels[0]).
        w(
            "npm/mcp/lib/constants.js",
            format!("const DEFAULT_CHANNEL = '{}';\n", channels[0]),
        );
    }

    /// Re-write a single consumer file in an otherwise-consistent fixture so it drops a channel
    /// (drifts from the canonical ring). Keyed by the consumer's repo-relative path.
    fn drift_one_consumer(root: &Path, rel: &str) {
        let w = |body: String| std::fs::write(root.join(rel), body).unwrap();
        match rel {
            "crates/cli/src/selfupdate.rs" => {
                w("const CHANNELS: [&str; 2] = [\"stable\", \"beta\"];\n".to_string())
            }
            "crates/cli/src/lib.rs" => {
                w("        #[arg(long, value_parser = [\"stable\", \"beta\"])]\n".to_string())
            }
            "install.sh" => w("#   NXF_CHANNEL  ring (stable|beta)\n".to_string()),
            other => panic!("unknown consumer {other}"),
        }
    }

    #[test]
    fn drift_in_any_consumer_is_caught() {
        let three = ["stable", "beta", "alpha"];

        // The canon-only case: a channel present only in release/channels drifts EVERY consumer.
        {
            let dir = tempfile::tempdir().unwrap();
            write_fixture(dir.path(), &three);
            check(dir.path()).expect("consistent fixture passes");
            let mut four = three.to_vec();
            four.push("nightly");
            std::fs::write(
                dir.path().join("release/channels"),
                format!("{}\n", four.join("\n")),
            )
            .unwrap();
            assert!(
                check(dir.path()).is_err(),
                "a channel present only in release/channels must fail the check"
            );
        }

        // Each consumer INDEPENDENTLY: perturb exactly one, leave the rest canonical, and assert
        // the check fails AND names that specific consumer. This gives install.sh and the
        // value_parser their own drift coverage instead of riding on selfupdate.rs.
        for consumer in [
            "crates/cli/src/selfupdate.rs",
            "crates/cli/src/lib.rs",
            "install.sh",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_fixture(dir.path(), &three);
            check(dir.path()).expect("consistent fixture passes before perturbation");
            drift_one_consumer(dir.path(), consumer);
            let err = check(dir.path())
                .expect_err(&format!("drifting {consumer} alone must fail the check"));
            let msg = format!("{err:#}");
            assert!(
                msg.contains(consumer),
                "drift error must name the offending consumer {consumer}, got: {msg}"
            );
        }
    }

    #[test]
    fn ordered_comparison_rejects_reordered_or_duplicated_consumer() {
        let three = ["stable", "beta", "alpha"];

        // Reordering a consumer's ring (set-equal but order differs) must FAIL: the list is a
        // promotion ring, most-stable first, so order is load-bearing.
        {
            let dir = tempfile::tempdir().unwrap();
            write_fixture(dir.path(), &three);
            std::fs::write(
                dir.path().join("crates/cli/src/selfupdate.rs"),
                "const CHANNELS: [&str; 3] = [\"beta\", \"stable\", \"alpha\"];\n",
            )
            .unwrap();
            assert!(
                check(dir.path()).is_err(),
                "a reordered ring must fail under order-preserving comparison"
            );
        }

        // A duplicate in a consumer (`stable, stable, beta`) must FAIL even though, deduped, it is
        // a subset — a repeat in a ring is always a smell.
        {
            let dir = tempfile::tempdir().unwrap();
            write_fixture(dir.path(), &three);
            std::fs::write(
                dir.path().join("crates/cli/src/selfupdate.rs"),
                "const CHANNELS: [&str; 3] = [\"stable\", \"stable\", \"beta\"];\n",
            )
            .unwrap();
            assert!(
                check(dir.path()).is_err(),
                "a duplicated channel must fail the check"
            );
        }
    }

    #[test]
    fn install_sh_decoy_alternation_above_real_drift_is_caught() {
        // A decoy `(stable|beta|alpha)` in unrelated prose ABOVE the real NXF_CHANNEL doc token,
        // which dropped `alpha`. The `NXF_CHANNEL` anchor must read the real (drifted) token, not
        // the decoy.
        //
        // This case had a TWIN over decide.ts's `download\/(…)` group until 6j6v.sgfr took that
        // consumer out. `pipe_alternation`'s anchoring is what both were about, and it is still
        // exercised here — the property kept its guard when the second caller went.
        let three = ["stable", "beta", "alpha"];
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path(), &three);
        std::fs::write(
            dir.path().join("install.sh"),
            "#   Historically the rings were (stable|beta|alpha).\n\
             #   NXF_CHANNEL  ring (stable|beta)\n",
        )
        .unwrap();
        assert!(
            check(dir.path()).is_err(),
            "a decoy alternation above a drifted NXF_CHANNEL token must NOT pass"
        );
    }

    #[test]
    fn bracketed_marker_in_a_comment_does_not_redirect_the_parse() {
        // A doc comment mentioning the marker with a DECOY list, BEFORE the real declaration that
        // has drifted (dropped `alpha`). The comment-skipping marker lookup must anchor on the
        // real declaration and catch the drift, not read the decoy and pass.
        let three = ["stable", "beta", "alpha"];
        let dir = tempfile::tempdir().unwrap();
        write_fixture(dir.path(), &three);
        std::fs::write(
            dir.path().join("crates/cli/src/selfupdate.rs"),
            "// CHANNELS used to be [\"stable\", \"beta\", \"alpha\"] in the old layout.\n\
             const CHANNELS: [&str; 2] = [\"stable\", \"beta\"];\n",
        )
        .unwrap();
        assert!(
            check(dir.path()).is_err(),
            "a decoy CHANNELS list in a comment above a drifted declaration must NOT pass"
        );
    }

    #[test]
    fn every_consumer_agrees_with_the_canonical_list() {
        // The live single-source assertion: selfupdate.rs's CHANNELS, the clap --channel
        // value_parser and install.sh's NXF_CHANNEL doc token all match release/channels. If this
        // fails, a channel was added/renamed without updating every consumer — exactly what
        // 85y.27 prevents.
        check(&repo_root()).expect("all consumers agree with release/channels");
    }

    #[test]
    fn parse_channels_drops_comments_and_blanks() {
        let body = "# header\n\nstable\n  beta  \n\n# trailing\n";
        assert_eq!(parse_channels(body), vec!["stable", "beta"]);
    }

    #[test]
    fn bracketed_string_list_reads_a_rust_array_literal() {
        let body = "const CHANNELS: [&str; 3] = [\"stable\", \"beta\", \"alpha\"];\n";
        assert_eq!(
            bracketed_string_list(body, "CHANNELS"),
            Some(vec![
                "stable".to_string(),
                "beta".to_string(),
                "alpha".to_string()
            ])
        );
        assert_eq!(bracketed_string_list(body, "MISSING"), None);
    }

    #[test]
    fn bracketed_string_list_reads_a_clap_value_parser() {
        let body = "        #[arg(long, value_parser = [\"stable\", \"beta\", \"alpha\"])]\n";
        assert_eq!(
            bracketed_string_list(body, "value_parser"),
            Some(vec![
                "stable".to_string(),
                "beta".to_string(),
                "alpha".to_string()
            ])
        );
    }

    #[test]
    fn pipe_alternation_extracts_a_paren_group_after_its_anchor() {
        let install = "#   NXF_CHANNEL=stable  ring (stable|beta|alpha)\n";
        assert_eq!(
            pipe_alternation(install, "NXF_CHANNEL"),
            Some(vec![
                "stable".to_string(),
                "beta".to_string(),
                "alpha".to_string()
            ])
        );
        let regex = "/^download\\/(stable|beta|alpha)\\/[A-Za-z0-9._\\-/]+\\.tar\\.gz$/";
        assert_eq!(
            pipe_alternation(regex, "download"),
            Some(vec![
                "stable".to_string(),
                "beta".to_string(),
                "alpha".to_string()
            ])
        );
        // No alternation after the anchor ⇒ None.
        assert_eq!(pipe_alternation("just (one) group, no pipe", "just"), None);
        // Anchor absent ⇒ None, even though a group exists.
        assert_eq!(pipe_alternation("(stable|beta|alpha)", "MISSING"), None);
    }

    #[test]
    fn pipe_alternation_ignores_groups_before_the_anchor() {
        // A decoy group precedes the anchor; only the post-anchor group is read.
        let body = "decoy (stable|beta|alpha) ... NXF_CHANNEL ring (stable|beta)\n";
        assert_eq!(
            pipe_alternation(body, "NXF_CHANNEL"),
            Some(vec!["stable".to_string(), "beta".to_string()])
        );
    }

    #[test]
    fn bracketed_string_list_skips_a_marker_mention_in_a_comment() {
        // The marker appears first in a comment (with a decoy list), then in the real
        // declaration; the real declaration's list is the one returned.
        let body = "// CHANNELS was [\"a\", \"b\", \"c\"]\n\
                    const CHANNELS: [&str; 2] = [\"stable\", \"beta\"];\n";
        assert_eq!(
            bracketed_string_list(body, "CHANNELS"),
            Some(vec!["stable".to_string(), "beta".to_string()])
        );
    }
}
