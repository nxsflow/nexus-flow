//! The actor-contract DETECTOR (nexus-flow-6j6v.d4dw): what the public write entry points of the
//! consumed library surfaces do with the `actor` they are handed, pinned at the boundaries of the
//! rule — so a tightening goes red BY ITSELF instead of depending on an author noticing.
//!
//! # Why this exists
//!
//! Twice on 2026-08-09 a behavioural break reached the release gate unseen. PR #311 made
//! `create(…, "")` return `Err` where it had returned `Ok` and written; PR #314 did the same for an
//! actor built only from characters that render as nothing (a zero-width space, a BOM). Both moved
//! the behaviour of all 26 public write entry points across `nexus-flow-facade`, `nexus-memory` and
//! `nexus-chat` without moving a single signature — so `cargo-semver-checks` had nothing to diff,
//! and the changelog gate saw a fragment with no `facade:` marker, which is indistinguishable from
//! "checked, no impact".
//!
//! `6j6v.gmjd` closed the second half: a diff that touches a consumed surface must now carry an
//! EXPLICIT `facade:` verdict. That makes a silent omission into a deliberate statement — it cannot
//! make the statement true. An author who writes `facade: none` in good faith and is wrong still
//! gets through, and that is exactly how both real cases happened: not dishonesty, but nobody
//! noticing anything had changed.
//!
//! This module is the other half. It does not ask; it looks.
//!
//! # What is pinned, and what deliberately is not
//!
//! **Pinned: the attributability verdict of the `actor` argument, at each boundary the shared rule
//! itself names.** For every boundary in [`BOUNDARIES`] and every entry point, the pin is (a)
//! accepted or rejected, (b) with which [`ErrorKind`](nxs_foundation::error::ErrorKind) if
//! rejected, (c) under which name the write is attributed if accepted, and (d) that a REJECTED
//! call appended nothing at all — the substrate claim, held at every entry point rather than
//! sampled at one (PR #317 review, Test Quality #1).
//!
//! **Not pinned, on purpose:**
//!
//! * **Error message text.** Only the kind. Wording changes are not contract changes, and a pin
//!   that reddens on a reworded sentence teaches its readers that red means nothing.
//! * **Anything about the actor beyond attributability** — length, interior control characters,
//!   case, Unicode normalization form, homoglyphs, the `origin/handle` shape. No promise exists
//!   today, so there is nothing to hold; and a sample drawn from a space the rule does not name
//!   produces a red test that cannot say WHICH boundary moved.
//!
//! The other three shared validators (`validate::label`, `validate::thread_id`,
//! `validate::iso_date`) have the same shape — one rule, N entry points, signatures unmoved — but
//! are not the axis that tipped twice. This module carries them without a redesign; the lift is
//! filed as `6j6v.amfs` rather than absorbed here.
//!
//! The rule behind the split, in one line: **pin the boundaries the rule itself names, never sample
//! the space it does not.** If a row cannot be written as a sentence the contract states, it is
//! implementation detail. That is the answer to the ticket's hardest question — over-pinning is
//! worse than under-pinning here, because every pinned row taxes every later change, including the
//! tightenings we want.
//!
//! Going red is therefore NOT "you broke the rules". It is "the consumed behaviour moved; say so".
//! The remedy is a decision, and it is meant to be cheap when the move was intended: change the row
//! and mark the fragment.
//!
//! # Relationship to the changelog gate (`6j6v.gmjd`)
//!
//! It COMPLEMENTS it, and the complement runs both ways — neither contains the other:
//!
//! * The gate fires where this is blind: a behavioural change at an edge nobody enumerated, or in a
//!   part of a consumed surface that has no `actor` at all.
//! * This fires where the gate is blind: a tightening in a directory that is neither a consumed
//!   surface nor a `CONTRACT_CRITICAL_DIR` — `crates/core/` is on the write path of all 17 flow
//!   entry points and is in neither list, so `touches_consumed_surface` is false for it and the
//!   marker duty never triggers.
//!
//! # Where the tests live
//!
//! The table and the runner live HERE, once, so a boundary that moves moves in a single reviewable
//! place. The WIRING lives beside each surface (`crates/{facade,memory,chat}/tests/
//! actor_contract.rs`), because the three call shapes differ and because `cargo test -p nexus-chat`
//! should run chat's rows. Same split as the verb-seam gate ([`crate::verb_seam`]).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What an entry point promises for one boundary input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The call goes through, and every op it appends is attributed to the actor's stored form.
    Accepted,
    /// The call is refused with this error kind (`ErrorKind::as_str`), before anything is written.
    Rejected(&'static str),
}

/// One named boundary of the shared actor rule, with the contract sentence it stands for.
#[derive(Debug, Clone, Copy)]
pub struct Boundary {
    /// Short identifier, used in failure messages and in a per-entry-point override.
    pub name: &'static str,
    /// The actor value handed to the entry point.
    pub actor: &'static str,
    /// The sentence this row pins — what a consumer may rely on.
    pub promise: &'static str,
    /// What the entry point does with it today.
    pub verdict: Verdict,
}

/// The boundaries of the shared actor rule (`nxs_foundation::model::validate_author` /
/// `is_attributable`), each one a sentence the contract states.
///
/// Seven rows, not seventy: the two that broke in the real cases ([`empty`](BOUNDARIES),
/// `invisible_only`), the two that say what still works after them (`plain`, `padded`), the line
/// PR #314 actually drew (`invisible_plus_visible` — one visible character is enough), the script
/// promise (`unicode_name`), and whitespace-only as the case `str::trim` alone would have caught.
pub const BOUNDARIES: &[Boundary] = &[
    Boundary {
        name: "plain",
        actor: "alice",
        promise: "an ordinary name is accepted and is the identity of record",
        verdict: Verdict::Accepted,
    },
    Boundary {
        name: "padded",
        actor: "  alice  ",
        promise: "surrounding whitespace is trimmed, not rejected; the trimmed form is stored",
        verdict: Verdict::Accepted,
    },
    Boundary {
        name: "unicode_name",
        actor: "Ünal 张三",
        promise: "a name may be written in any script — the rule is attribution, not ASCII",
        verdict: Verdict::Accepted,
    },
    Boundary {
        name: "invisible_plus_visible",
        actor: "\u{200B}alice",
        promise: "one visible character is enough — invisible characters around a real name are \
                  kept verbatim, not stripped (the line PR #314 drew)",
        verdict: Verdict::Accepted,
    },
    Boundary {
        name: "empty",
        actor: "",
        promise: "an empty actor is a caller mistake, refused as `validation` before any write \
                  (PR #311)",
        verdict: Verdict::Rejected("validation"),
    },
    Boundary {
        name: "whitespace_only",
        actor: " \t\n ",
        promise: "an actor of nothing but whitespace names no one — same refusal as empty",
        verdict: Verdict::Rejected("validation"),
    },
    Boundary {
        name: "invisible_only",
        actor: "\u{200B}\u{FEFF}\u{2069}",
        promise: "an actor built only from characters that RENDER as nothing names no one either \
                  — `str::trim` strips none of them (PR #314)",
        verdict: Verdict::Rejected("validation"),
    },
];

/// What a call actually did — the observation the wiring hands back for one boundary.
///
/// BOTH variants carry the ops the call appended, because "it was refused" and "it wrote nothing"
/// are two different claims and the second is the one that matters on an append-only log: an
/// unattributable author cannot be repaired afterwards (`6j6v.xsf3` invariant 1). A refusal that
/// had already written half of its work would satisfy the verdict table and still be the defect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The call returned `Ok`, appending these ops (in order), each with the author it stamped.
    Accepted { authors: Vec<String> },
    /// The call returned `Err` with this error kind, having appended these ops (must be none).
    Rejected { kind: String, appended: Vec<String> },
}

impl Outcome {
    /// An accepted call, with the authors of every op it appended.
    pub fn accepted(authors: impl IntoIterator<Item = String>) -> Outcome {
        Outcome::Accepted {
            authors: authors.into_iter().collect(),
        }
    }

    /// A refused call, with the error kind it reported (`ErrorKind::as_str`) and the authors of
    /// any ops it appended before giving up — which the runner requires to be none.
    pub fn rejected(kind: &str, appended: impl IntoIterator<Item = String>) -> Outcome {
        Outcome::Rejected {
            kind: kind.to_owned(),
            appended: appended.into_iter().collect(),
        }
    }
}

/// A boundary this entry point answers differently, with the reason it may.
///
/// Same philosophy as [`crate::verb_seam::Waiver`]: an exception carries a reason or it is a
/// finding. The runner also refuses an override that no longer deviates from [`BOUNDARIES`] — a
/// stale exception is worse than none, because it silently absorbs the very movement this detector
/// exists to report.
#[derive(Debug, Clone, Copy)]
pub struct Deviation {
    /// The [`Boundary::name`] this entry point answers differently.
    pub boundary: &'static str,
    /// What it does instead.
    pub verdict: Verdict,
    /// Why the difference is the contract rather than a defect. Required.
    pub reason: &'static str,
}

/// Hold one entry point against every boundary.
///
/// `author_of` maps an actor's stored (trimmed) form to the author the surface stamps on its ops —
/// identity for flow and memory, `origin/handle` for chat. It is computed by the TEST, from the
/// boundary input, so a production change that stopped trimming reddens here instead of being
/// mirrored.
///
/// `call` runs the entry point with one boundary's actor against a FRESH fixture and reports what
/// happened. It must return only the ops the call itself appended — seeding ops would drown the
/// author assertion.
///
/// # Panics
///
/// On any divergence from the pinned contract, with the boundary, the sentence it stands for, and
/// what to do about it.
pub fn assert_actor_contract(
    entry: &str,
    author_of: impl Fn(&str) -> String,
    call: impl FnMut(&Boundary) -> Outcome,
) {
    assert_actor_contract_with(entry, &[], author_of, call)
}

/// [`assert_actor_contract`] for an entry point that answers some boundary differently — see
/// [`Deviation`].
pub fn assert_actor_contract_with(
    entry: &str,
    deviations: &[Deviation],
    author_of: impl Fn(&str) -> String,
    mut call: impl FnMut(&Boundary) -> Outcome,
) {
    for deviation in deviations {
        let boundary = BOUNDARIES
            .iter()
            .find(|b| b.name == deviation.boundary)
            .unwrap_or_else(|| {
                panic!(
                    "{entry} declares a deviation for boundary `{}`, which is not in BOUNDARIES — \
                     a deviation from nothing waives nothing",
                    deviation.boundary
                )
            });
        assert!(
            !deviation.reason.trim().is_empty(),
            "{entry}'s deviation for `{}` carries no reason; an exception without one is a \
             finding, not a free pass",
            deviation.boundary
        );
        assert_ne!(
            deviation.verdict, boundary.verdict,
            "{entry}'s deviation for `{}` states the SAME verdict as the shared contract — drop \
             it, or it will absorb a real change in silence",
            deviation.boundary
        );
    }

    for boundary in BOUNDARIES {
        let expected = deviations
            .iter()
            .find(|d| d.boundary == boundary.name)
            .map(|d| d.verdict)
            .unwrap_or(boundary.verdict);
        let observed = call(boundary);
        if let Some(complaint) = disagreement(boundary, expected, &observed, &author_of) {
            panic!(
                "{}",
                report(entry, boundary, expected, &observed, &complaint)
            );
        }
    }
}

/// The whole judgement as a pure function, so every failure mode is unit-testable without a store.
/// `None` means the entry point honours the boundary.
fn disagreement(
    boundary: &Boundary,
    expected: Verdict,
    observed: &Outcome,
    author_of: &impl Fn(&str) -> String,
) -> Option<String> {
    match (expected, observed) {
        (Verdict::Accepted, Outcome::Accepted { authors }) => {
            if authors.is_empty() {
                return Some(
                    "the call reported success but appended no op, so this row proves nothing — \
                     give the fixture something to write"
                        .to_owned(),
                );
            }
            let want = author_of(boundary.actor.trim());
            let wrong: Vec<&String> = authors.iter().filter(|a| **a != want).collect();
            if wrong.is_empty() {
                None
            } else {
                Some(format!(
                    "it was accepted, but wrote ops attributed to {wrong:?} instead of {want:?} — \
                     the stored form of the actor moved"
                ))
            }
        }
        (
            Verdict::Rejected(_),
            Outcome::Rejected {
                kind: got,
                appended,
            },
        ) => {
            // "Refused" and "wrote nothing" are separate claims; the second is the one an
            // append-only log cannot forgive. Checked before the kind, because a refusal that
            // already wrote is a defect whatever kind it carries.
            if !appended.is_empty() {
                return Some(format!(
                    "it was refused, but had ALREADY appended ops attributed to {appended:?} — a \
                     rejected write must leave the log exactly as it found it, because attribution \
                     on an append-only log cannot be repaired afterwards"
                ));
            }
            match expected {
                Verdict::Rejected(want) if want == got => None,
                _ => Some(format!(
                    "it was refused, but as `{got}` — the rejection moved to a different kind, \
                     which a consumer branches on"
                )),
            }
        }
        (Verdict::Accepted, Outcome::Rejected { kind, .. }) => Some(format!(
            "it is now REFUSED (`{kind}`) — an input this surface used to accept and write. This \
             is the shape of PR #311 and PR #314"
        )),
        (Verdict::Rejected(_), Outcome::Accepted { authors }) => Some(format!(
            "it is now ACCEPTED, writing ops attributed to {authors:?} — an input this surface \
             used to refuse"
        )),
    }
}

/// The panic text: what moved, what it was for, and that the answer is a decision.
fn report(
    entry: &str,
    boundary: &Boundary,
    expected: Verdict,
    observed: &Outcome,
    complaint: &str,
) -> String {
    let expected = match expected {
        Verdict::Accepted => "accepted".to_owned(),
        Verdict::Rejected(kind) => format!("refused with kind `{kind}`"),
    };
    format!(
        "\n{entry} no longer honours the actor boundary `{name}`.\n\
         \n  actor    : {actor:?}\
         \n  contract : {promise}\
         \n  expected : {expected}\
         \n  observed : {observed:?}\
         \n  so       : {complaint}\n\
         \nThis is a BEHAVIOURAL change to a CONSUMED library surface — not a broken test. Nothing \
         else in the chain\ncan see it: signatures did not move, so `cargo-semver-checks` has \
         nothing to diff.\n\
         \nIf you did NOT mean it, revert it. If you did, move the row in\n\
         crates/test-support/src/actor_contract.rs and give this PR's changelog fragment\n\
         `facade: breaking` (nexus-flow-6j6v.d4dw, 6j6v.gmjd).\n",
        name = boundary.name,
        actor = boundary.actor,
        promise = boundary.promise,
    )
}

// ---- completeness: the table cannot quietly miss the 27th entry point -------

/// Every `pub fn` under `src_dir` that validates an actor through the shared rule — the entry-point
/// set DERIVED from the source, so the covered set can be held against it.
///
/// A gate whose input is a hand-written second list drifts exactly like the thing it watches. The
/// 26 points this ticket starts from were counted by hand in PR #312; nothing but this keeps the
/// count honest when a 27th arrives.
///
/// Matching is on the parsed token stream, never the text, so a doc comment that merely NAMES
/// `validate_author` (there are several) is not a call. The whole `src/` tree is walked rather than
/// a declared file list: a new write seam in a new module is precisely the case worth catching.
/// A `pub fn` in a PRIVATE module would be counted too — that errs toward "look at this", which is
/// the safe direction for a gate, and there is none today.
pub fn actor_validating_fns(src_dir: &Path) -> Result<BTreeSet<String>, String> {
    let mut sources = Vec::new();
    collect_rs(src_dir, &mut sources)
        .map_err(|e| format!("{} could not be walked: {e}", src_dir.display()))?;
    if sources.is_empty() {
        return Err(format!("{} holds no Rust source at all", src_dir.display()));
    }
    sources.sort();
    let mut found = BTreeSet::new();
    for src in &sources {
        let text = std::fs::read_to_string(src)
            .map_err(|e| format!("{} could not be read: {e}", src.display()))?;
        let file = syn::parse_file(&text)
            .map_err(|e| format!("{} could not be parsed as Rust: {e}", src.display()))?;
        collect_validating(&file.items, &mut found);
    }
    if found.is_empty() {
        return Err(format!(
            "{} holds no public function that validates an actor — the detector would report \
             all-clear forever",
            src_dir.display()
        ));
    }
    Ok(found)
}

/// A public function that calls the shared rule without BEING a write entry point — it only passes
/// the rule on under a local name (flow's `validate::actor` is the whole population today).
///
/// A reason is required, and a declaration that no longer matches anything is refused: a
/// pass-through that has since grown a write is exactly the case this gate must not absorb.
#[derive(Debug, Clone, Copy)]
pub struct Delegator {
    /// The function name as the source spells it.
    pub name: &'static str,
    /// Why it forwards the rule rather than applying it to a write. Required.
    pub reason: &'static str,
}

/// Hold the wired entry points against [`actor_validating_fns`].
///
/// # Panics
///
/// If the surface has grown (or lost) an actor-validating entry point the table does not name.
pub fn assert_every_entry_point_is_covered(
    surface: &str,
    src_dir: &Path,
    covered: &[&str],
    delegators: &[Delegator],
) {
    let derived = actor_validating_fns(src_dir).unwrap_or_else(|e| {
        panic!(
            "{surface}: the entry-point set could not be derived, so this gate would report \
                all-clear forever — {e}"
        )
    });
    for delegator in delegators {
        assert!(
            !delegator.reason.trim().is_empty(),
            "{surface}: the delegator `{}` carries no reason; an exception without one is a \
             finding, not a free pass",
            delegator.name
        );
        assert!(
            derived.contains(delegator.name),
            "{surface}: `{}` is declared a delegator but no longer calls the shared actor rule — \
             drop the declaration before it starts excusing something else",
            delegator.name
        );
    }
    let wired: BTreeSet<String> = covered
        .iter()
        .map(|s| (*s).to_owned())
        .chain(delegators.iter().map(|d| d.name.to_owned()))
        .collect();
    let missing: Vec<&String> = derived.difference(&wired).collect();
    let stale: Vec<&String> = wired.difference(&derived).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "\n{surface}: the actor-contract table no longer matches the surface.\n\
         \n  validates an actor but is NOT pinned : {missing:?}\
         \n  pinned but no longer validates       : {stale:?}\n\
         \nA new public write entry point inherits the whole actor contract the moment it calls the\n\
         shared rule; a pin that outlives its entry point holds nothing. Wire it or drop it in this\n\
         file (nexus-flow-6j6v.d4dw).\n"
    );
}

/// Every `.rs` file under `dir`, recursively — **not** following symlinks.
///
/// `symlink_metadata` rather than `Path::is_dir`, so a symlinked directory is skipped instead of
/// descended into (PR #317 review, Integrity & Robustness #1). Nothing under `crates/` is a symlink
/// today; the point is that a loop introduced later would hang the test binary in CI rather than
/// fail it, and a gate whose failure mode is a hang gets disabled instead of fixed. Skipping is the
/// safe direction for this gate either way: an entry point reachable only through a symlinked tree
/// would come out as "not pinned", never as false coverage.
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_symlink() {
            continue;
        }
        if meta.is_dir() {
            collect_rs(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// The `pub fn`s in `items` whose body calls the shared actor rule, following inherent `impl`s and
/// inline `pub mod`s exactly as [`crate::verb_seam::public_symbols`] does.
fn collect_validating(items: &[syn::Item], out: &mut BTreeSet<String>) {
    fn is_public(vis: &syn::Visibility) -> bool {
        matches!(vis, syn::Visibility::Public(_))
    }
    for item in items {
        match item {
            syn::Item::Fn(f) if is_public(&f.vis) && validates_actor(&f.block) => {
                out.insert(f.sig.ident.to_string());
            }
            syn::Item::Impl(imp) if imp.trait_.is_none() => {
                for member in &imp.items {
                    if let syn::ImplItem::Fn(f) = member {
                        if is_public(&f.vis) && validates_actor(&f.block) {
                            out.insert(f.sig.ident.to_string());
                        }
                    }
                }
            }
            syn::Item::Mod(m) if is_public(&m.vis) => {
                if let Some((_, items)) = &m.content {
                    collect_validating(items, out);
                }
            }
            _ => {}
        }
    }
}

/// Whether a function body CALLS the shared actor rule — `…::validate_author(…)` (the substrate's
/// own name) or `validate::actor(…)` (flow's name for it).
///
/// A syntax walk over call expressions, not a text search: `validate_author` is named in half a
/// dozen doc comments and in prose inside other functions, and a gate that counted those would put
/// entry points on the list that have none.
fn validates_actor(block: &syn::Block) -> bool {
    use syn::visit::Visit;

    #[derive(Default)]
    struct Calls {
        found: bool,
    }
    impl<'ast> Visit<'ast> for Calls {
        fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
            if let syn::Expr::Path(p) = &*call.func {
                let segments: Vec<String> = p
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect();
                let tail: Vec<&str> = segments.iter().rev().take(2).map(String::as_str).collect();
                self.found |= matches!(tail.as_slice(), ["validate_author", ..])
                    || matches!(tail.as_slice(), ["actor", "validate"]);
            }
            syn::visit::visit_expr_call(self, call);
        }
    }

    let mut calls = Calls::default();
    calls.visit_block(block);
    calls.found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> &'static Boundary {
        BOUNDARIES.iter().find(|b| b.name == "plain").unwrap()
    }

    fn empty() -> &'static Boundary {
        BOUNDARIES.iter().find(|b| b.name == "empty").unwrap()
    }

    fn identity(a: &str) -> String {
        a.to_owned()
    }

    #[test]
    fn every_boundary_is_named_once_and_carries_a_promise() {
        let mut names = BTreeSet::new();
        for b in BOUNDARIES {
            assert!(names.insert(b.name), "duplicate boundary name `{}`", b.name);
            assert!(
                !b.promise.trim().is_empty(),
                "`{}` states no promise",
                b.name
            );
        }
        // Both directions have to be represented, or the table only detects one kind of movement.
        assert!(BOUNDARIES.iter().any(|b| b.verdict == Verdict::Accepted));
        assert!(BOUNDARIES
            .iter()
            .any(|b| matches!(b.verdict, Verdict::Rejected(_))));
    }

    #[test]
    fn an_honoured_boundary_is_silent() {
        assert!(disagreement(
            plain(),
            Verdict::Accepted,
            &Outcome::accepted(["alice".to_owned()]),
            &identity
        )
        .is_none());
        assert!(disagreement(
            empty(),
            Verdict::Rejected("validation"),
            &Outcome::rejected("validation", []),
            &identity
        )
        .is_none());
    }

    #[test]
    fn a_tightening_is_caught_which_is_the_whole_point() {
        // PR #311/#314 in miniature: an input that was accepted now comes back as an error.
        let complaint = disagreement(
            plain(),
            Verdict::Accepted,
            &Outcome::rejected("validation", []),
            &identity,
        )
        .expect("a tightening must be reported");
        assert!(complaint.contains("REFUSED"), "{complaint}");
    }

    #[test]
    fn a_loosening_is_caught_too() {
        let complaint = disagreement(
            empty(),
            Verdict::Rejected("validation"),
            &Outcome::accepted(["".to_owned()]),
            &identity,
        )
        .expect("a loosening must be reported");
        assert!(complaint.contains("ACCEPTED"), "{complaint}");
    }

    #[test]
    fn a_rejection_that_changed_kind_is_caught() {
        let complaint = disagreement(
            empty(),
            Verdict::Rejected("validation"),
            &Outcome::rejected("forbidden", []),
            &identity,
        )
        .expect("the kind is part of the contract");
        assert!(complaint.contains("forbidden"), "{complaint}");
    }

    #[test]
    fn a_moved_stored_form_is_caught() {
        // If trimming stopped, `"  alice  "` would be written verbatim onto an append-only log.
        let padded = BOUNDARIES.iter().find(|b| b.name == "padded").unwrap();
        let complaint = disagreement(
            padded,
            Verdict::Accepted,
            &Outcome::accepted(["  alice  ".to_owned()]),
            &identity,
        )
        .expect("the stored form is part of the contract");
        assert!(complaint.contains("stored form"), "{complaint}");
    }

    #[test]
    fn a_refusal_that_had_already_written_is_caught_whatever_kind_it_carries() {
        // The substrate claim (PR #311, PR #317 review Test Quality #1): the verdict alone would
        // pass this — the call DID come back as `validation`. What must not happen is that it
        // wrote first, because attribution on an append-only log cannot be repaired afterwards.
        let complaint = disagreement(
            empty(),
            Verdict::Rejected("validation"),
            &Outcome::rejected("validation", ["".to_owned()]),
            &identity,
        )
        .expect("a half-written refusal must be reported");
        assert!(complaint.contains("ALREADY appended"), "{complaint}");

        // …and it is checked BEFORE the kind, so a wrong-kind refusal that also wrote reports the
        // write, which is the more serious of the two.
        let complaint = disagreement(
            empty(),
            Verdict::Rejected("validation"),
            &Outcome::rejected("forbidden", ["x".to_owned()]),
            &identity,
        )
        .expect("still reported");
        assert!(complaint.contains("ALREADY appended"), "{complaint}");
    }

    #[test]
    fn an_accepted_call_that_wrote_nothing_proves_nothing() {
        let complaint = disagreement(
            plain(),
            Verdict::Accepted,
            &Outcome::accepted([]),
            &identity,
        )
        .expect("a vacuous row must not pass");
        assert!(complaint.contains("appended no op"), "{complaint}");
    }

    #[test]
    fn the_author_mapping_is_applied_not_assumed() {
        // chat attributes `origin/handle`, so the same boundary expects a different author there.
        let prefixed = |a: &str| format!("local/{a}");
        assert!(disagreement(
            plain(),
            Verdict::Accepted,
            &Outcome::accepted(["local/alice".to_owned()]),
            &prefixed
        )
        .is_none());
        assert!(disagreement(
            plain(),
            Verdict::Accepted,
            &Outcome::accepted(["alice".to_owned()]),
            &prefixed
        )
        .is_some());
    }

    #[test]
    fn a_deviation_that_agrees_with_the_contract_is_refused() {
        let hit = std::panic::catch_unwind(|| {
            assert_actor_contract_with(
                "x::y",
                &[Deviation {
                    boundary: "empty",
                    verdict: Verdict::Rejected("validation"),
                    reason: "same as the shared rule",
                }],
                identity,
                |_| Outcome::rejected("validation", []),
            )
        });
        assert!(hit.is_err(), "a no-op deviation must be refused");
    }

    #[test]
    fn a_deviation_without_a_reason_is_itself_a_finding() {
        let hit = std::panic::catch_unwind(|| {
            assert_actor_contract_with(
                "x::y",
                &[Deviation {
                    boundary: "empty",
                    verdict: Verdict::Rejected("forbidden"),
                    reason: "  ",
                }],
                identity,
                |_| Outcome::rejected("forbidden", []),
            )
        });
        assert!(hit.is_err(), "a reasonless deviation must be refused");
    }

    #[test]
    fn a_deviation_for_an_unknown_boundary_waives_nothing() {
        let hit = std::panic::catch_unwind(|| {
            assert_actor_contract_with(
                "x::y",
                &[Deviation {
                    boundary: "no_such_boundary",
                    verdict: Verdict::Rejected("forbidden"),
                    reason: "typo",
                }],
                identity,
                |_| Outcome::rejected("validation", []),
            )
        });
        assert!(hit.is_err(), "a deviation from nothing must be refused");
    }

    #[test]
    fn validating_functions_are_found_by_call_not_by_mention() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            r#"
            /// Mentions validate_author in prose only — this is not a call.
            pub fn documented_only(actor: &str) -> u8 { 0 }
            pub fn writes(actor: &str) -> u8 { let a = validate_author(actor); 1 }
            pub fn flows(actor: &str) -> u8 { let a = validate::actor(actor); 1 }
            fn private_write(actor: &str) -> u8 { let a = validate_author(actor); 1 }
            #[cfg(test)]
            mod tests { pub fn helper(a: &str) { validate_author(a); } }
            "#,
        )
        .unwrap();
        let found = actor_validating_fns(dir.path()).unwrap();
        assert_eq!(
            found,
            ["flows".to_owned(), "writes".to_owned()]
                .into_iter()
                .collect::<BTreeSet<_>>(),
            "a prose mention, a private fn and a private test module are not entry points"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_source_walk_does_not_follow_a_symlink() {
        // PR #317 review, Integrity & Robustness #1: a symlink loop under a scanned tree would
        // HANG the test binary, and a gate whose failure mode is a hang gets disabled, not fixed.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "pub fn one(a: &str) { validate_author(a); }\n",
        )
        .unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(
            nested.join("b.rs"),
            "pub fn two(a: &str) { validate_author(a); }\n",
        )
        .unwrap();
        // The loop: `nested/back` points at the root of the very tree being walked.
        std::os::unix::fs::symlink(dir.path(), nested.join("back")).unwrap();

        let found = actor_validating_fns(dir.path()).expect("the walk terminates");
        assert_eq!(
            found,
            ["one".to_owned(), "two".to_owned()]
                .into_iter()
                .collect::<BTreeSet<_>>(),
            "a real subdirectory is still walked; only the symlink is skipped"
        );
    }

    #[test]
    fn a_surface_with_nothing_to_derive_is_loud_not_all_clear() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "pub fn reads() {}\n").unwrap();
        assert!(actor_validating_fns(dir.path()).is_err());
        assert!(actor_validating_fns(&dir.path().join("nope")).is_err());
    }

    #[test]
    fn an_entry_point_the_table_forgot_fails_the_completeness_gate() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "pub fn one(a: &str) { validate_author(a); }\n\
             pub fn two(a: &str) { validate_author(a); }\n",
        )
        .unwrap();
        let hit = std::panic::catch_unwind(|| {
            assert_every_entry_point_is_covered("x", dir.path(), &["one"], &[])
        });
        assert!(hit.is_err(), "the 27th entry point must not slip through");
        assert_every_entry_point_is_covered("x", dir.path(), &["one", "two"], &[]);
    }

    #[test]
    fn a_delegator_is_excused_only_while_it_is_really_there() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.rs"),
            "pub fn one(a: &str) { validate_author(a); }\n\
             pub fn passes_it_on(a: &str) { validate_author(a); }\n",
        )
        .unwrap();
        let real = Delegator {
            name: "passes_it_on",
            reason: "forwards the rule under a local name; applies it to no write",
        };
        assert_every_entry_point_is_covered("x", dir.path(), &["one"], &[real]);

        for bad in [
            Delegator {
                name: "passes_it_on",
                reason: "   ",
            },
            Delegator {
                name: "gone",
                reason: "moved away a release ago",
            },
        ] {
            let hit = std::panic::catch_unwind(|| {
                assert_every_entry_point_is_covered("x", dir.path(), &["one"], &[bad])
            });
            assert!(
                hit.is_err(),
                "a reasonless or stale delegator must be refused"
            );
        }
    }
}
