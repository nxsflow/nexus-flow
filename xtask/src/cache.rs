//! `cargo xtask cache sweep` (nexus-flow-6j6v.gapx): decide which GitHub Actions cache entries
//! no future run can reach, so the repo stops evicting the ones it still needs.
//!
//! WHY THIS EXISTS. A repository gets 10 GB of Actions cache and GitHub evicts by least-recent
//! use when that fills. Measured on 2026-09-04 this repo sat at 10.83 GB in 32 entries — over
//! the limit, so eviction was running continuously — and 6.30 GB of that (58 %) belonged to refs
//! no workflow can ever produce again: five entries under branches that were deleted, eight
//! under the merge refs of PRs that were merged, eight under release tags. The entries being
//! evicted to make room for that were the live ones, and the first to go was the coldest live
//! lane: the macOS cache, which exactly one job reads. That is the original bug report — a job
//! that could not find its own, fully uploaded, bit-identical cache 47 minutes later.
//!
//! WHAT THE RULE IS: an entry is deleted exactly when NO FUTURE RUN CAN READ IT. That is not a
//! judgement about age or size; it is a property of the entry's `ref`, which is the visibility
//! scope the cache service stores it under. A run reads its own ref's entries plus the default
//! branch's, and nothing else — so a ref that can no longer be checked out is a ref whose
//! entries are unreachable by construction.
//!
//! Three things make that sentence worth a module rather than three lines of YAML:
//!
//! 1. REACHABILITY IS A FACT ABOUT THE REPO, NOT ABOUT THE ENTRY. Whether `refs/heads/feat/x` is
//!    dead depends on the branch list; whether `refs/pull/428/merge` is dead depends on the PR
//!    list. So the decision is a pure function of (entries, what the repo can still reach), and
//!    keeping it pure is what makes the interesting cases testable — a deleted branch literally
//!    named `refs/tags/v1`, a merge ref with no number, an empty world. None of those are shapes
//!    anyone would manufacture in CI to watch a workflow handle them.
//!
//! 2. UNCERTAINTY FALLS TO KEEP — WITH TWO NAMED EXCEPTIONS, and pretending otherwise would be
//!    the more dangerous documentation. An unknown ref shape, a merge ref whose number will not
//!    parse, a tag when no newest tag was supplied: all kept. The expensive mistake is throwing
//!    away a live `main` cache — that costs every branch a cold build. The cheap mistake is
//!    leaving a dead entry lying around one more day, which is the state this module is fixing.
//!
//!    THE FIRST EXCEPTION: the branch arm does not KEEP on doubt, it DELETES ON ABSENCE. A
//!    `refs/heads/<b>` entry is swept because `<b>` is not in the branch list — and absence has
//!    two causes, "the branch is gone" and "the list did not mention it". So this one arm rests
//!    on the branch list being COMPLETE, not merely non-empty, and that is a property only the
//!    caller can supply. `cache-sweep.yml` therefore reads it from `git ls-remote --heads` — one
//!    unpaginated ref advertisement carrying every `refs/heads/*` the server has, with no page to
//!    lose and no namespace the API might choose not to surface (the ephemeral
//!    `gh-readonly-queue/...` refs of a merge queue are the case that raised this). The review of
//!    #431 found this arm described as if it kept on doubt; it never did.
//!
//!    THE SECOND EXCEPTION is the tag rule, and it is named where it is implemented.
//!
//! 3. AN EMPTY WORLD IS AN ERROR, NOT AN EMPTY ANSWER. If the branch list arrives empty, the
//!    honest reading is that the query failed, not that the repo has no branches — and treating
//!    it as fact would delete every branch-scoped entry including the default branch's. So
//!    [`Reachable::new`] refuses a branch list that does not contain the default branch. That
//!    catches a list that failed ENTIRELY; it cannot catch one that is merely short, which is
//!    why the completeness of the source in point 2 carries the weight rather than this check.
//!
//! WHAT THIS DOES NOT DO. It does not touch `save-if`/`cache-targets` on the CI lanes: the
//! measured hit rate leaves nothing to fix there, and suppressing saves would trade a working
//! cache for space this sweep recovers for free. The counts behind that decision live once, in
//! the header of `.github/workflows/cache-sweep.yml`, rather than a second time here — a number
//! restated in three files is three places to forget on the next measurement (review of #431).
//! The one save that *was* worth suppressing is `release.yml`'s per-tag zigbuild cache, and its
//! own count sits at that line.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// The ref every run can read regardless of its own scope. GitHub scopes the base-branch
/// fallback to the repository's default branch, which is `main` here.
pub const DEFAULT_BRANCH: &str = "main";

/// One entry as `GET /repos/{owner}/{repo}/actions/caches` reports it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct CacheEntry {
    pub id: u64,
    pub key: String,
    /// The visibility scope. `refs/heads/<branch>`, `refs/pull/<n>/merge`, and — for a
    /// tag-triggered run — the doubled shape `refs/heads/refs/tags/<tag>` the API really emits.
    #[serde(rename = "ref")]
    pub git_ref: String,
    pub size_in_bytes: u64,
}

/// What the repository can still check out. Supplied by the caller (the workflow queries the
/// API); this module never reaches the network.
#[derive(Debug, Clone)]
pub struct Reachable {
    branches: BTreeSet<String>,
    open_prs: BTreeSet<u64>,
    keep_tag: Option<String>,
}

impl Reachable {
    /// `branches` are bare names (`main`, `feat/x`), `open_prs` the numbers of PRs still open,
    /// `keep_tag` the newest release tag — the one release a re-run could still plausibly want.
    ///
    /// Refuses a branch list without [`DEFAULT_BRANCH`] — see the module header, point 3.
    ///
    /// What that refusal proves and what it does not: it catches a branch list that failed
    /// COMPLETELY (empty, or some other repo's), because every repo has its default branch. It
    /// cannot catch a list that is merely SHORT — one that lost a page or a namespace and still
    /// happens to carry `main` passes here, and every live branch missing from it would then be
    /// swept as "gone". No check inside a pure function can see that; completeness is a property
    /// of the query that produced the list, which is why `cache-sweep.yml` takes it from
    /// `git ls-remote --heads` (unpaginated, no server-side filtering) rather than from a
    /// paginated REST listing.
    pub fn new(
        branches: impl IntoIterator<Item = String>,
        open_prs: impl IntoIterator<Item = u64>,
        keep_tag: Option<String>,
    ) -> Result<Self> {
        let branches: BTreeSet<String> = branches.into_iter().collect();
        if !branches.contains(DEFAULT_BRANCH) {
            bail!(
                "the branch list does not contain `{DEFAULT_BRANCH}`, so it cannot be the real \
                 branch list — refusing to sweep rather than treat a failed query as proof that \
                 every branch is gone"
            );
        }
        Ok(Self {
            branches,
            open_prs: open_prs.into_iter().collect(),
            keep_tag,
        })
    }
}

/// What the sweep decided about one entry, and why. The reason is carried, not derived later:
/// it is what the dry run prints and what makes a deletion answerable afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub id: u64,
    pub key: String,
    pub git_ref: String,
    pub size_in_bytes: u64,
    pub delete: bool,
    pub reason: &'static str,
}

/// The whole rule. Pure: same inputs, same answer, no clock, no network, no filesystem.
pub fn sweep(entries: &[CacheEntry], world: &Reachable) -> Vec<Decision> {
    entries
        .iter()
        .map(|e| {
            let (delete, reason) = verdict(&e.git_ref, world);
            Decision {
                id: e.id,
                key: e.key.clone(),
                git_ref: e.git_ref.clone(),
                size_in_bytes: e.size_in_bytes,
                delete,
                reason,
            }
        })
        .collect()
}

/// The decision table, one arm per row of the header's rule.
///
/// The `refs/heads/` arm is ONE arm, not two, and the liveness test comes before the tag reading
/// inside it. That ordering is the whole subtlety: a tag-triggered run stores its cache under the
/// doubled shape `refs/heads/refs/tags/<tag>`, and `refs/tags/<tag>` is also a legal branch name.
/// Reading the tag first would sweep a live branch that happens to be called that; asking whether
/// the ref names something that exists first cannot, because a branch that exists is reachable
/// whatever it is named.
fn verdict(git_ref: &str, world: &Reachable) -> (bool, &'static str) {
    if let Some(name) = git_ref.strip_prefix("refs/heads/") {
        if name == DEFAULT_BRANCH {
            return (false, "the default branch — every branch reads from here");
        }
        if world.branches.contains(name) {
            return (false, "the branch still exists");
        }
        // Not a live branch. Now the doubled tag shape can be read without ambiguity.
        //
        // THIS ARM IS THE ONE ACCEPTED RISK IN THE TABLE, and it is not an instance of the
        // module's keep-on-doubt rule — calling it one would overstate its safety (review of
        // #431). Every other deletion here rests on a structural fact: a deleted branch cannot be
        // checked out, a closed PR's merge ref is not rebuilt. A superseded TAG is different —
        // `refs/tags/v0.82.0` is never deleted and stays checkout-able forever, so a re-run of an
        // old release (`gh run rerun`, or a dispatch against an old tag for a backport) is
        // structurally possible and merely improbable. The bet is taken deliberately, because
        // getting it wrong costs one cold rebuild on a rare re-run, while NOT taking it brings
        // back the 1.29 GB in 8 tag-scoped entries that were part of the original problem and
        // grow with every release. Cheap either way — but a bet, not a guarantee.
        if let Some(tag) = name.strip_prefix("refs/tags/") {
            return match world.keep_tag.as_deref() {
                None => (false, "tag scope, and no newest tag was named — keeping"),
                Some(newest) if newest == tag => {
                    (false, "the newest release tag — a re-run could read it")
                }
                Some(_) => (true, "a superseded release tag; a tag is built once"),
            };
        }
        return (
            true,
            "the branch is gone, so no run can carry this ref again",
        );
    }
    if let Some(rest) = git_ref.strip_prefix("refs/pull/") {
        let Some(number) = rest.strip_suffix("/merge") else {
            return (false, "a pull-request ref of an unfamiliar shape — keeping");
        };
        let Ok(number) = number.parse::<u64>() else {
            return (
                false,
                "a pull-request ref with no readable number — keeping",
            );
        };
        return if world.open_prs.contains(&number) {
            (false, "the pull request is still open")
        } else {
            (
                true,
                "the pull request is closed, so its merge ref is not built again",
            )
        };
    }
    (false, "an unfamiliar ref shape — keeping")
}

/// Bytes the deletions in `decisions` would free.
pub fn freed_bytes(decisions: &[Decision]) -> u64 {
    decisions
        .iter()
        .filter(|d| d.delete)
        .map(|d| d.size_in_bytes)
        .sum()
}

// ---------------------------------------------------------------------------------------------
// The impure edge. Everything above is a pure decision; this reads three files and prints. The
// network stays in the workflow — `gh api` fetches, `gh api -X DELETE` deletes — so nothing here
// can be surprised by a rate limit or an auth scope, and the rule stays testable without one.
// ---------------------------------------------------------------------------------------------

/// The two halves of the output are deliberately on different streams: stdout carries ONLY the
/// ids to delete, one per line, so the workflow can pipe it straight into a delete loop; stderr
/// carries the reasoning, so a dry run and a live run print exactly the same report and the dry
/// run is a real preview rather than a second implementation of the rule.
pub fn run_sweep(
    caches: &Path,
    branches: &Path,
    open_prs: &Path,
    keep_tag: Option<String>,
) -> Result<()> {
    let entries = read_entries(caches)?;
    let world = Reachable::new(
        read_lines(branches)?,
        read_lines(open_prs)?
            .into_iter()
            .map(|l| {
                l.parse::<u64>()
                    .with_context(|| format!("`{l}` is not a pull-request number"))
            })
            .collect::<Result<Vec<_>>>()?,
        keep_tag.filter(|t| !t.is_empty()),
    )?;

    let decisions = sweep(&entries, &world);
    let total: u64 = entries.iter().map(|e| e.size_in_bytes).sum();
    let freed = freed_bytes(&decisions);

    for d in &decisions {
        eprintln!(
            "{} {:>8}  {:<38} {}  ({})",
            if d.delete { "DELETE" } else { "keep  " },
            human(d.size_in_bytes),
            d.git_ref,
            d.key,
            d.reason
        );
    }
    eprintln!(
        "\n{} of {} entries unreachable — {} of {} would be freed, leaving {}",
        decisions.iter().filter(|d| d.delete).count(),
        decisions.len(),
        human(freed),
        human(total),
        human(total - freed)
    );

    for d in decisions.iter().filter(|d| d.delete) {
        println!("{}", d.id);
    }
    Ok(())
}

/// NDJSON, one entry per line — the shape `gh api --paginate ... -q '.actions_caches[]'` emits,
/// which is the only one that survives more than one page of results. `-` reads stdin.
fn read_entries(path: &Path) -> Result<Vec<CacheEntry>> {
    let raw = if path == Path::new("-") {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)?;
        buf
    } else {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    };
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str::<CacheEntry>(l).with_context(|| format!("parsing `{l}`")))
        .collect()
}

fn read_lines(path: &Path) -> Result<Vec<String>> {
    Ok(std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect())
}

/// Decimal GB/MB, because that is the unit GitHub states the 10 GB limit in and the unit
/// `actions/cache/usage` answers in.
///
/// The sub-megabyte arm is not hypothetical: this repo really holds a 403 KB npm cache, and
/// integer division alone printed it as `0 MB` — a line that reads as "this entry is empty"
/// next to a deletion verdict (review of #431).
fn human(bytes: u64) -> String {
    match bytes {
        b if b >= 1_000_000_000 => format!("{:.2} GB", b as f64 / 1e9),
        b if b >= 1_000_000 => format!("{} MB", b / 1_000_000),
        0 => "0 B".to_string(),
        b => format!("{} KB", b.div_ceil(1_000)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, git_ref: &str, size: u64) -> CacheEntry {
        CacheEntry {
            id,
            key: format!("v0-rust-job-{id}"),
            git_ref: git_ref.to_string(),
            size_in_bytes: size,
        }
    }

    /// A world with `main` plus whatever else is named.
    fn world(branches: &[&str], open_prs: &[u64], keep_tag: Option<&str>) -> Reachable {
        let mut all = vec![DEFAULT_BRANCH.to_string()];
        all.extend(branches.iter().map(|s| s.to_string()));
        Reachable::new(all, open_prs.iter().copied(), keep_tag.map(str::to_string)).unwrap()
    }

    fn deleted(entries: &[CacheEntry], world: &Reachable) -> Vec<u64> {
        sweep(entries, world)
            .into_iter()
            .filter(|d| d.delete)
            .map(|d| d.id)
            .collect()
    }

    /// The one entry whose loss is expensive: every branch restores from the default branch, so
    /// a sweep that touches it makes every later run cold. Named as its own test because it is
    /// the failure this module must never produce, not merely one row of the table.
    #[test]
    fn the_default_branch_is_never_swept() {
        let e = [entry(1, "refs/heads/main", 783 * 1024 * 1024)];
        assert!(deleted(&e, &world(&[], &[], None)).is_empty());
        // …not even when the branch list is somehow reduced to nothing but itself and the entry
        // is by far the largest thing in the repo.
        assert!(deleted(&e, &world(&[], &[7, 8, 9], Some("v9.9.9"))).is_empty());
    }

    #[test]
    fn a_branch_that_still_exists_keeps_its_cache() {
        let e = [entry(1, "refs/heads/feat/nxc-surface-recut", 100)];
        assert!(deleted(&e, &world(&["feat/nxc-surface-recut"], &[], None)).is_empty());
    }

    /// 2,89 GB of the measured 10,83 GB sat here: `feat/wwrh-docs-blocks-feed`, merged and
    /// deleted, its whole eight-job cache set still billed against the limit.
    #[test]
    fn a_deleted_branch_loses_its_cache() {
        let e = [entry(
            1,
            "refs/heads/feat/wwrh-docs-blocks-feed",
            782 * 1024 * 1024,
        )];
        assert_eq!(deleted(&e, &world(&["feat/other"], &[], None)), vec![1]);
    }

    #[test]
    fn an_open_pull_request_keeps_its_merge_ref_cache() {
        let e = [entry(1, "refs/pull/431/merge", 234)];
        assert!(deleted(&e, &world(&[], &[431], None)).is_empty());
    }

    /// The recurring source: `changelog-check.yml` runs only on `pull_request`, so it writes a
    /// 234 MB entry per PR that nothing outside that PR can read. Eight of them were lying
    /// around from eight merged PRs.
    #[test]
    fn a_closed_pull_request_loses_its_merge_ref_cache() {
        let e = [entry(1, "refs/pull/428/merge", 234 * 1024 * 1024)];
        assert_eq!(deleted(&e, &world(&[], &[431], None)), vec![1]);
    }

    /// The shape the API really emits for a tag-triggered run — `refs/heads/` and then the tag
    /// ref again. A reader that only saw the `refs/heads/` prefix would take `refs/tags/v0.83.0`
    /// for a branch name; that it is not one is the point of the arm being ahead of the branch arm.
    #[test]
    fn a_superseded_release_tag_loses_its_cache() {
        let e = [entry(1, "refs/heads/refs/tags/v0.82.0", 277 * 1024 * 1024)];
        assert_eq!(deleted(&e, &world(&[], &[], Some("v0.83.0"))), vec![1]);
    }

    #[test]
    fn the_newest_release_tag_keeps_its_cache() {
        let e = [entry(1, "refs/heads/refs/tags/v0.83.0", 277 * 1024 * 1024)];
        assert!(deleted(&e, &world(&[], &[], Some("v0.83.0"))).is_empty());
    }

    /// `refs/tags/v0.82.0` is a legal branch name, and a branch that exists is reachable whatever
    /// it is called — so the liveness test has to come before the tag reading, not after it.
    /// Nothing in this repo is named that; the rule holds anyway, which is the point.
    #[test]
    fn a_live_branch_named_like_a_tag_ref_outranks_the_tag_reading() {
        let e = [entry(1, "refs/heads/refs/tags/v0.82.0", 100)];
        let live = world(&["refs/tags/v0.82.0"], &[], Some("v0.83.0"));
        assert!(deleted(&e, &live).is_empty());
        // …and with no such branch it is read as the superseded tag it looks like.
        assert_eq!(deleted(&e, &world(&[], &[], Some("v0.83.0"))), vec![1]);
    }

    /// Fail-safe: with no newest tag named, the sweep cannot tell a superseded release from the
    /// current one, so it touches neither.
    #[test]
    fn without_a_newest_tag_no_tag_scope_is_swept() {
        let e = [
            entry(1, "refs/heads/refs/tags/v0.82.0", 100),
            entry(2, "refs/heads/refs/tags/v0.83.0", 100),
        ];
        assert!(deleted(&e, &world(&[], &[], None)).is_empty());
    }

    /// Fail-safe: refs this rule has no arm for are kept. A cache the sweep does not understand
    /// is not evidence that it is dead.
    ///
    /// One case per assertion rather than five behind one `is_empty()`: bundled, a regression
    /// says only "something in that group broke" and the next reader has to bisect the array by
    /// hand to learn which shape stopped being safe (review of #431).
    #[test]
    fn an_unfamiliar_ref_shape_is_kept() {
        let w = world(&[], &[], Some("v0.83.0"));
        for (git_ref, what) in [
            (
                "refs/tags/v0.83.0",
                "a bare tag ref, not the doubled shape a cache carries",
            ),
            ("refs/pull/428/head", "the PR's head ref, not its merge ref"),
            (
                "refs/pull/not-a-number/merge",
                "a merge ref whose number will not parse",
            ),
            ("", "no ref at all"),
            ("refs/remotes/origin/main", "a remote-tracking ref"),
        ] {
            assert!(
                deleted(&[entry(1, git_ref, 100)], &w).is_empty(),
                "`{git_ref}` ({what}) must be kept, not swept"
            );
        }
    }

    /// The shape a GitHub merge queue gives its ephemeral branches. It is an ORDINARY branch —
    /// `git ls-remote --heads` advertises it like any other — so while the queue entry is in
    /// flight the liveness test keeps its cache, and once GitHub deletes the ref the entry is
    /// unreachable exactly like any other deleted branch's.
    ///
    /// The review of #431 read this as a live-deletion hazard on the assumption that the REST
    /// branch listing hides these refs. That assumption could not be verified here (this repo
    /// cannot enable a merge queue: `gh api .../rulesets` answers 403), so the fix was not to
    /// special-case the prefix but to take the branch list from a source that cannot hide a
    /// namespace at all — see the module header, point 2. This test pins the behaviour either
    /// way: whatever the listing does, a queue ref that IS listed survives.
    #[test]
    fn a_merge_queue_ref_lives_and_dies_with_its_branch() {
        let queue_ref = "refs/heads/gh-readonly-queue/main/pr-431-2a7e2445";
        let e = [entry(1, queue_ref, 780 * 1024 * 1024)];

        let in_flight = world(&["gh-readonly-queue/main/pr-431-2a7e2445"], &[], None);
        assert!(
            deleted(&e, &in_flight).is_empty(),
            "a queue entry still in flight is still reading this cache"
        );

        assert_eq!(
            deleted(&e, &world(&[], &[], None)),
            vec![1],
            "once GitHub drops the ephemeral ref, no run can carry it again"
        );
    }

    /// The one loud failure. An empty branch list is a failed query, and reading it as fact
    /// would sweep every branch-scoped entry in the repo.
    #[test]
    fn a_branch_list_without_the_default_branch_is_refused() {
        assert!(Reachable::new(Vec::<String>::new(), [], None).is_err());
        assert!(Reachable::new(["feat/x".to_string()], [], None).is_err());
        assert!(Reachable::new([DEFAULT_BRANCH.to_string()], [], None).is_ok());
    }

    /// Sizes a human reads next to a delete/keep verdict. The sub-megabyte arm exists because
    /// this repo really holds a 403 KB entry that used to print as `0 MB`.
    #[test]
    fn sizes_never_read_as_empty_when_they_are_not() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1), "1 KB", "anything non-zero must not round away");
        assert_eq!(human(403_303), "404 KB");
        assert_eq!(human(999_999), "1000 KB");
        assert_eq!(human(1_000_000), "1 MB");
        assert_eq!(human(821_166_633), "821 MB");
        assert_eq!(human(5_173_207_206), "5.17 GB");
    }

    /// The number the dry run reports is the number the deletions actually free — the entries
    /// that stay must not be counted into it.
    #[test]
    fn the_freed_total_counts_only_what_is_deleted() {
        let e = [
            entry(1, "refs/heads/main", 1_000),
            entry(2, "refs/heads/feat/gone", 200),
            entry(3, "refs/pull/428/merge", 30),
            entry(4, "refs/pull/431/merge", 4_000),
        ];
        let decisions = sweep(&e, &world(&[], &[431], None));
        assert_eq!(freed_bytes(&decisions), 230);
    }

    /// The measured tree of 2026-09-04, entry for entry: 10,83 GB in, the 6,30 GB that no run
    /// can reach out. If a later change to the rule moves this number, that is the change asking
    /// to be explained.
    #[test]
    fn the_measured_tree_of_2026_09_04_sweeps_to_its_reachable_half() {
        // (ref, size in bytes) exactly as the API reported them, pulled on 2026-09-04 with
        //
        //     gh api --paginate "repos/nxsflow/nexus-flow/actions/caches?per_page=100" \
        //       -q '.actions_caches[] | [.ref, .size_in_bytes] | @tsv'
        //
        // TO A LATER MAINTAINER: do not patch rows to make this pass. This is a snapshot of one
        // real tree and its value is that nobody chose it to suit the rule — a hand-edited row
        // turns it into a synthetic case with a misleading name. If a new arm changes what the
        // rule decides here, re-pull the whole set with the query above (against whatever the
        // tree looks like then), rename the test to its date, and restate the three totals.
        let raw: &[(&str, u64)] = &[
            ("refs/heads/main", 821166633),
            ("refs/heads/feat/wwrh-docs-blocks-feed", 820682480),
            ("refs/heads/feat/wwrh-docs-blocks-feed", 651624718),
            ("refs/heads/main", 650773485),
            ("refs/heads/main", 643521389),
            ("refs/heads/feat/wwrh-docs-blocks-feed", 643013620),
            ("refs/heads/main", 519607395),
            ("refs/heads/main", 406784291),
            ("refs/heads/feat/wwrh-docs-blocks-feed", 406287147),
            ("refs/heads/main", 398061343),
            ("refs/pull/426/merge", 398061339),
            ("refs/heads/main", 368062869),
            ("refs/heads/feat/macos-test-lane", 368032802),
            ("refs/heads/refs/tags/v0.83.0", 290804197),
            ("refs/heads/refs/tags/v0.82.0", 290165848),
            ("refs/heads/refs/tags/v0.83.0", 289823328),
            ("refs/heads/refs/tags/v0.82.0", 289748665),
            ("refs/heads/main", 274368284),
            ("refs/heads/main", 246074152),
            ("refs/pull/426/merge", 246058629),
            ("refs/pull/425/merge", 246056265),
            ("refs/pull/430/merge", 246041594),
            ("refs/pull/429/merge", 246033425),
            ("refs/pull/428/merge", 246024236),
            ("refs/pull/423/merge", 246022399),
            ("refs/pull/427/merge", 245994347),
            ("refs/heads/main", 197540688),
            ("refs/heads/refs/tags/v0.83.0", 47085060),
            ("refs/heads/refs/tags/v0.82.0", 47085060),
            ("refs/heads/refs/tags/v0.83.0", 19130789),
            ("refs/heads/refs/tags/v0.82.0", 18900562),
            ("refs/heads/main", 403303),
        ];
        let entries: Vec<CacheEntry> = raw
            .iter()
            .enumerate()
            .map(|(i, (r, s))| entry(i as u64, r, *s))
            .collect();
        // The repo as it stood: main plus two untouched branches, no open PR, v0.83.0 newest.
        let w = world(
            &["feat/daemon-restart-spec", "feat/nxc-surface-recut"],
            &[],
            Some("v0.83.0"),
        );
        let decisions = sweep(&entries, &w);

        let total: u64 = raw.iter().map(|(_, s)| s).sum();
        let kept: u64 = total - freed_bytes(&decisions);
        assert_eq!(total, 10_829_040_342, "the measured occupancy");
        assert_eq!(
            decisions.iter().filter(|d| d.delete).count(),
            17,
            "the entries no run can reach (v0.83.0's four are kept as the newest tag)"
        );
        assert_eq!(freed_bytes(&decisions), 5_655_833_136);
        // The whole point: what is left fits under the 10 GB limit with room to spare.
        assert_eq!(kept, 5_173_207_206, "what a future run can still read");
        assert!(
            kept < 6_000_000_000,
            "kept {kept} bytes — half the 10 GB limit"
        );
    }
}
