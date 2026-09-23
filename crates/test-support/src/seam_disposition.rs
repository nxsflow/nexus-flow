//! The DISPOSITION gate (nexus-flow-6j6v.dvyq, acceptance point 3): for every entrance taken off a
//! module's agent surface, whether the corresponding read/write stays on the library seam is
//! **decided, recorded, and held by a gate** — not left to diligence.
//!
//! # Why this is a second gate and not more of the first one
//!
//! [`crate::verb_seam`] next door checks ONE direction, and its own test name says so:
//! *every nxc verb is reachable from the library seam*. A capability that leaves the CLI and stays
//! on the seam does not trip it — `Engine::inbox` could be deleted tomorrow and that gate would go
//! green, because there would be no verb left asking for it. (It WAS deleted, by nexus-flow-6j6v.yr59
//! on 2026-08-21, while `nxc inbox` stood for another six days; nexus-flow-6j6v.1gm9 then took the
//! verb too. So BOTH halves of the worked example below are history now rather than a hypothesis,
//! they left in the right order and at different times, and each move is recorded as its own row in
//! chat's table. `Engine::mark_read` was the live pair of that shape for a fortnight — an app's
//! acknowledgement long after nobody typed `nxc read` — until nexus-flow-6j6v.4d2z measured it,
//! found no app had ever called it, and removed it; its row records all three moves.)
//!
//! That is exactly the failure this one exists for, and it is not hypothetical. The item's §5 calls
//! it the most expensive place in the whole consolidation:
//!
//! > **Removing a CLI verb is NOT the same as removing a seam read.** `Engine::inbox` is what an
//! > app builds its unread view from; `threads`/`thread_board` are what `nxc status` is made of.
//! > What disappears from the AGENT surface stays on the APP seam, provided an app renders it. Too
//! > coarse a cut breaks app-foundations at a place the CLI cannot see.
//!
//! So the two gates ask opposite questions. The first: *can an app do everything an agent can?*
//! This one: *when an agent stops being able to do something, what happens to the app?*
//!
//! # What it holds, claim by claim
//!
//! Both inputs are derived — the `clap` tree and the module's own engine/facade sources — and the
//! module declares only the DECISION. Every state is a claim the gate can check, and each is
//! checked in both directions, so neither half of a removal can quietly drift away from the other:
//!
//! * [`SeamState::Stays`] — the symbol must BE on the seam. This is the one app-foundations asked
//!   for by name for `Engine::definitions`: read and write must not be grabbed together during a
//!   cleanup.
//! * [`SeamState::Leaves`] — the symbol must be on the seam while the removal has not landed, and
//!   absent once it has. A half-finished cut fails either way round.
//! * [`SeamState::NoCounterpart`] — the symbol must NOT be on the seam. The claim "there was never
//!   anything here to keep" stops being true the moment somebody lifts the verb, and the entry has
//!   to be revisited rather than silently mean something else.
//! * [`CliState`] — the entrance must be gone from the `clap` tree, or still in it, exactly as the
//!   record says. A record that has fallen behind the code is worse than no record: it reads as a
//!   considered decision while describing a surface that no longer exists.
//!
//! # What it deliberately cannot check
//!
//! **Names, not behaviour** — a coincidentally-named `pub fn` satisfies it, exactly as in
//! `verb_seam`. Divergence between two implementations is the parity differential's job.
//!
//! **Leaf verbs and long flags, not positional shapes.** `nxc send <channel> <body>` collapsing
//! into `nxc send --to <channel>` is an entrance removal this table records, and the gate can hold
//! the flag half of it (`--to` exists, `--role` does not) but not the arity of a positional. Those
//! rows carry their decision for a human; the gate holds what a name comparison can hold.
//!
//! **Completeness of the table itself.** There is no second list of "everything that was removed"
//! to compare against — this table IS that record. What the gate guarantees is that every claim in
//! it stays true, which is the part diligence actually fails at.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::verb_seam::{leaf_verbs, read_seam, seam_name};

/// Whether the entrance is still on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliState {
    /// Already taken off the surface. The gate checks it really is gone.
    Gone,
    /// Still on the surface: the decision below is recorded, the cut has not landed yet. The gate
    /// checks it really is still there, so a removal cannot land without this record moving with
    /// it.
    Standing,
    /// Never was a CLI entrance at all — a seam capability an agent reaches only indirectly, or not
    /// at all. `verb` is empty for these and the symbol is what identifies the row.
    NeverWas,
}

/// What was decided about the library seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeamState {
    /// The read/write STAYS on the seam — an app renders it, so the capability outlives the verb.
    Stays {
        /// The public seam symbol that carries it. Must exist.
        symbol: &'static str,
    },
    /// The seam method LEAVES with the verb — nothing renders it, and keeping it would leave the
    /// deleted concept alive exactly where an app sees it.
    Leaves {
        /// The public seam symbol that goes.
        symbol: &'static str,
        /// Whether that removal has landed. `false` holds the symbol PRESENT (the decision is
        /// recorded, the work is not done); `true` holds it ABSENT.
        landed: bool,
    },
    /// There is no seam counterpart at all, and never was — the CLI acted straight on the store.
    /// Nothing stays and nothing leaves; the gate holds that the seam really is empty here.
    ///
    /// For a whole VERB only. A flag has no seam symbol of its own — it is a field on the request
    /// its verb's method takes — so a flag row decides about that method instead, and the gate
    /// rejects this state on one.
    NoCounterpart,
}

/// One recorded decision: an entrance, whether it is still there, and what became of the seam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disposition {
    /// The entrance exactly as a user types it after the program name — a leaf path
    /// (`"transcript append"`), a leaf path plus a long flag (`"send --model"`), or `""` for a
    /// seam capability with no CLI entrance ([`CliState::NeverWas`]).
    pub verb: &'static str,
    pub cli: CliState,
    pub seam: SeamState,
    /// Why. Required — a decision without a reason is a note, not a decision.
    pub reason: &'static str,
}

impl Disposition {
    /// The seam symbol this row speaks about, whichever state it is in.
    fn symbol(&self) -> String {
        match self.seam {
            SeamState::Stays { symbol } | SeamState::Leaves { symbol, .. } => symbol.to_owned(),
            SeamState::NoCounterpart => seam_name(self.verb),
        }
    }

    /// The row's identity: the entrance plus the symbol it decides about. A pair, because one
    /// entrance can retire more than one seam method (`ask` and its `--expect`), and several
    /// seam-only rows share the empty verb.
    fn key(&self) -> (String, String) {
        (self.verb.to_owned(), self.symbol())
    }

    /// The leaf verb path, with a trailing long flag (if any) stripped off.
    fn leaf(&self) -> &'static str {
        match self.flag() {
            Some(flag) => self.verb[..self.verb.len() - flag.len() - 1].trim_end(),
            None => self.verb,
        }
    }

    /// The long flag this row is about, when it is about a flag rather than a whole verb.
    fn flag(&self) -> Option<&'static str> {
        self.verb
            .rsplit(' ')
            .next()
            .filter(|last| last.starts_with("--") && self.verb.contains(' '))
    }
}

/// Fail the current test unless every recorded disposition still describes reality.
///
/// `module` names the persona in the failure text (`"nxc"`); `seam_sources` are the module's engine
/// + facade sources, the same union [`crate::verb_seam`] reads.
pub fn assert_seam_dispositions(
    module: &str,
    cli: &clap::Command,
    seam_sources: &[PathBuf],
    record: &[Disposition],
) {
    let seam = match read_seam(seam_sources) {
        Ok(seam) => seam,
        Err(blind) => panic!("{}", blind_gate(module, &blind)),
    };
    if let Some(complaint) = verdict(module, cli, &seam, record) {
        panic!("{complaint}");
    }
}

/// Every long flag a leaf verb accepts, keyed by the leaf's space-joined path.
fn long_flags(cli: &clap::Command) -> BTreeMap<String, BTreeSet<String>> {
    fn walk(cmd: &clap::Command, path: &[String], out: &mut BTreeMap<String, BTreeSet<String>>) {
        let children: Vec<&clap::Command> = cmd
            .get_subcommands()
            .filter(|s| s.get_name() != "help")
            .collect();
        if children.is_empty() {
            if !path.is_empty() {
                out.insert(
                    path.join(" "),
                    cmd.get_arguments()
                        .filter_map(|a| a.get_long())
                        .map(|l| format!("--{l}"))
                        .collect(),
                );
            }
            return;
        }
        for child in children {
            let mut next = path.to_vec();
            next.push(child.get_name().to_owned());
            walk(child, &next, out);
        }
    }
    let mut out = BTreeMap::new();
    walk(cli, &[], &mut out);
    out
}

/// The whole decision as a pure function, so every failure mode is unit-testable.
///
/// `None` means the record holds. `Some(text)` is the complaint, formatted for the panic.
fn verdict(
    module: &str,
    cli: &clap::Command,
    seam: &BTreeSet<String>,
    record: &[Disposition],
) -> Option<String> {
    let verbs: BTreeSet<String> = leaf_verbs(cli).into_iter().collect();
    let flags = long_flags(cli);
    let mut findings: Vec<String> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();

    for d in record {
        let at = if d.verb.is_empty() {
            format!("(seam-only) {}", d.symbol())
        } else {
            d.verb.to_owned()
        };
        if !seen.insert(d.key()) {
            findings.push(format!(
                "{at:<32} is recorded twice for the same seam symbol — one decision, one row."
            ));
        }
        if d.flag().is_some() && d.seam == SeamState::NoCounterpart {
            findings.push(format!(
                "{at:<32} is a FLAG recorded as having no seam counterpart. A flag has no symbol \
                 of its own — decide about the method its verb reaches (`Stays`/`Leaves`) and say \
                 in the reason what happens to the field."
            ));
        }
        if d.reason.trim().is_empty() {
            findings.push(format!(
                "{at:<32} carries no reason. A decision without a reason is a note, not a decision."
            ));
        }

        // ---- the CLI half -------------------------------------------------------------------
        let present = match d.flag() {
            Some(flag) => flags.get(d.leaf()).is_some_and(|f| f.contains(flag)),
            None => verbs.contains(d.verb),
        };
        match d.cli {
            CliState::Gone if present => findings.push(format!(
                "{at:<32} is recorded as REMOVED but is still on the {module} surface. Either the \
                 removal did not land or this record ran ahead of it."
            )),
            CliState::Standing if !present => findings.push(format!(
                "{at:<32} is recorded as still standing but is NOT on the {module} surface any \
                 more. The removal landed and this record did not move with it — say what became \
                 of the seam."
            )),
            CliState::NeverWas if !d.verb.is_empty() => findings.push(format!(
                "{at:<32} is recorded as never having been a CLI entrance, but names one. Use an \
                 empty `verb` for a seam-only capability."
            )),
            _ => {}
        }

        // ---- the SEAM half ------------------------------------------------------------------
        let on_seam = seam.contains(d.symbol().as_str());
        match d.seam {
            SeamState::Stays { symbol } if !on_seam => findings.push(format!(
                "{at:<32} says `{symbol}` STAYS on the seam, and it is not there. This is the cut \
                 the item's §5 warns about: a read an app renders, taken away with the verb that \
                 used to reach it."
            )),
            SeamState::Leaves { symbol, landed } => {
                if landed && on_seam {
                    findings.push(format!(
                        "{at:<32} says `{symbol}` has LEFT the seam, and it is still there. Either \
                         the removal is unfinished or the record claims more than happened."
                    ));
                }
                if !landed && !on_seam {
                    findings.push(format!(
                        "{at:<32} says `{symbol}` leaves the seam LATER, and it is already gone. \
                         Half a removal: mark it landed, or put the method back until the entrance \
                         goes with it."
                    ));
                }
            }
            SeamState::NoCounterpart if on_seam => findings.push(format!(
                "{at:<32} says the seam has no counterpart, but `{}` is on it now. The debt was \
                 paid — re-decide whether it stays.",
                d.symbol()
            )),
            _ => {}
        }
    }

    if findings.is_empty() {
        return None;
    }
    let mut out = format!(
        "\n\nSEAM DISPOSITION ({module}) — a recorded decision no longer describes reality.\n\n"
    );
    for f in &findings {
        out.push_str(&format!("    {f}\n"));
    }
    out.push_str(
        "\n  For every entrance taken off the agent surface it is DECIDED and RECORDED whether\n  \
         the corresponding read/write stays on the library seam (nexus-flow-6j6v.dvyq,\n  \
         acceptance point 3). A CLI verb and a seam read are not the same thing:\n  \
         `Engine::status`\n  \
         is what an app asks where an operation stands with, long after nobody types\n  \
         `nxc status`.\n\n  \
         Fix it in the record, not by deleting the row: change `cli`/`seam` to what is now true\n  \
         and say why. The table lives in this module's `tests/seam_disposition.rs`.\n",
    );
    Some(out)
}

/// The gate has lost sight of the seam it watches — distinct from a decision that stopped being
/// true, and phrased so nobody goes looking for a missing verb.
fn blind_gate(module: &str, detail: &str) -> String {
    format!(
        "\n\nSEAM DISPOSITION GATE IS BLIND ({module}) — it cannot read the seam it checks \
         against.\n\n  {detail}\n\n\
         This is NOT a broken decision. With none of the seam sources readable the gate would\n\
         report `all clear` forever, which is the silence it exists to abolish.\n\n\
         Re-point the source list in this module's `tests/seam_disposition.rs`; do not delete the\n\
         gate.\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seam(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    /// A tiny `nxc`-shaped tree: `inbox`, `send --to/--role`, and `transcript append` — a stand-in
    /// two-level path, chosen because the gate has to handle one and this module's own vocabulary
    /// should name verbs that exist.
    fn cli() -> clap::Command {
        clap::Command::new("nxc")
            .subcommand(clap::Command::new("inbox"))
            .subcommand(
                clap::Command::new("send")
                    .arg(clap::Arg::new("to").long("to"))
                    .arg(clap::Arg::new("role").long("role")),
            )
            .subcommand(clap::Command::new("transcript").subcommand(clap::Command::new("append")))
    }

    fn row(verb: &'static str, cli: CliState, seam: SeamState) -> Disposition {
        Disposition {
            verb,
            cli,
            seam,
            reason: "because",
        }
    }

    #[test]
    fn a_record_that_matches_reality_holds() {
        let record = [
            row(
                "inbox",
                CliState::Standing,
                SeamState::Stays { symbol: "inbox" },
            ),
            row(
                "transcript append",
                CliState::Standing,
                SeamState::Leaves {
                    symbol: "transcript_append",
                    landed: false,
                },
            ),
            row(
                "send --model",
                CliState::Gone,
                SeamState::Stays { symbol: "send" },
            ),
        ];
        assert_eq!(
            verdict(
                "nxc",
                &cli(),
                &seam(&["inbox", "transcript_append", "send"]),
                &record
            ),
            None
        );
    }

    /// The finding this gate exists for: a read an app renders, deleted with the verb.
    #[test]
    fn a_seam_read_that_must_stay_and_has_gone_is_the_headline_failure() {
        let record = [row(
            "inbox",
            CliState::Standing,
            SeamState::Stays { symbol: "inbox" },
        )];
        let complaint = verdict("nxc", &cli(), &seam(&["messages"]), &record)
            .expect("a vanished `Stays` symbol must fail the gate");
        assert!(complaint.contains("STAYS on the seam"), "{complaint}");
        assert!(complaint.contains("§5"), "{complaint}");
    }

    /// `verb_seam`'s blind spot, in miniature: nothing on the CLI asks for `inbox` any more, so
    /// that gate is green — and this one is not.
    #[test]
    fn the_blind_spot_of_the_omission_gate_is_caught_here() {
        let no_inbox = clap::Command::new("nxc").subcommand(clap::Command::new("send"));
        let record = [row(
            "inbox",
            CliState::Gone,
            SeamState::Stays { symbol: "inbox" },
        )];
        // The verb really is gone (that half is fine); the seam read is what went missing with it.
        let complaint = verdict("nxc", &no_inbox, &seam(&["send"]), &record)
            .expect("a read that left with its verb must fail");
        assert!(complaint.contains("STAYS on the seam"), "{complaint}");
    }

    #[test]
    fn a_verb_recorded_as_removed_that_is_still_there_fails() {
        let record = [row(
            "inbox",
            CliState::Gone,
            SeamState::Stays { symbol: "inbox" },
        )];
        let complaint = verdict("nxc", &cli(), &seam(&["inbox"]), &record)
            .expect("a record that ran ahead of the code must fail");
        assert!(
            complaint.contains("still on the nxc surface"),
            "{complaint}"
        );
    }

    #[test]
    fn a_verb_recorded_as_standing_that_has_gone_fails() {
        let no_inbox = clap::Command::new("nxc").subcommand(clap::Command::new("send"));
        let record = [row(
            "inbox",
            CliState::Standing,
            SeamState::Stays { symbol: "inbox" },
        )];
        let complaint = verdict("nxc", &no_inbox, &seam(&["inbox"]), &record)
            .expect("a record the code moved past must fail");
        assert!(complaint.contains("did not move with it"), "{complaint}");
    }

    #[test]
    fn half_a_removal_fails_from_either_side() {
        // The seam method went first, with the entrance still standing.
        let early = [row(
            "transcript append",
            CliState::Standing,
            SeamState::Leaves {
                symbol: "transcript_append",
                landed: false,
            },
        )];
        let complaint = verdict("nxc", &cli(), &seam(&["inbox"]), &early).expect("early removal");
        assert!(complaint.contains("Half a removal"), "{complaint}");

        // …and the mirror image: recorded as done, still on the seam.
        let late = [row(
            "transcript append",
            CliState::Standing,
            SeamState::Leaves {
                symbol: "transcript_append",
                landed: true,
            },
        )];
        let complaint =
            verdict("nxc", &cli(), &seam(&["transcript_append"]), &late).expect("late removal");
        assert!(complaint.contains("still there"), "{complaint}");
    }

    #[test]
    fn a_no_counterpart_claim_that_stopped_being_true_fails() {
        let record = [row(
            "transcript append",
            CliState::Standing,
            SeamState::NoCounterpart,
        )];
        let complaint = verdict("nxc", &cli(), &seam(&["transcript_append"]), &record)
            .expect("a lifted verb must invalidate the claim");
        assert!(complaint.contains("The debt was paid"), "{complaint}");
    }

    /// A flag is a field on a request, not a symbol — recording it as "no counterpart" would be a
    /// claim the gate cannot hold, so it is refused outright.
    #[test]
    fn a_flag_may_not_be_recorded_as_having_no_counterpart() {
        let record = [row(
            "send --role",
            CliState::Standing,
            SeamState::NoCounterpart,
        )];
        let complaint = verdict("nxc", &cli(), &seam(&["send"]), &record)
            .expect("a flag with NoCounterpart must fail");
        assert!(complaint.contains("has no symbol of"), "{complaint}");
    }

    #[test]
    fn a_decision_without_a_reason_is_itself_a_finding() {
        let record = [Disposition {
            verb: "inbox",
            cli: CliState::Standing,
            seam: SeamState::Stays { symbol: "inbox" },
            reason: "   ",
        }];
        let complaint = verdict("nxc", &cli(), &seam(&["inbox"]), &record)
            .expect("a reasonless decision must fail");
        assert!(complaint.contains("carries no reason"), "{complaint}");
    }

    #[test]
    fn one_entrance_decides_one_symbol_once() {
        let record = [
            row(
                "inbox",
                CliState::Standing,
                SeamState::Stays { symbol: "inbox" },
            ),
            row(
                "inbox",
                CliState::Standing,
                SeamState::Stays { symbol: "inbox" },
            ),
        ];
        let complaint =
            verdict("nxc", &cli(), &seam(&["inbox"]), &record).expect("a doubled row must fail");
        assert!(complaint.contains("recorded twice"), "{complaint}");
    }

    /// One entrance MAY decide two different symbols — `ask` retires both `ask` and its
    /// channel-open path — so the duplicate rule keys on the pair, not on the verb.
    #[test]
    fn one_entrance_may_decide_two_different_symbols() {
        let record = [
            row(
                "transcript append",
                CliState::Standing,
                SeamState::Leaves {
                    symbol: "transcript_append",
                    landed: false,
                },
            ),
            row(
                "transcript append",
                CliState::Standing,
                SeamState::Leaves {
                    symbol: "inbox",
                    landed: false,
                },
            ),
        ];
        assert_eq!(
            verdict(
                "nxc",
                &cli(),
                &seam(&["transcript_append", "inbox"]),
                &record
            ),
            None
        );
    }

    #[test]
    fn a_flag_entrance_is_checked_on_its_own_leaf() {
        let gone = [row(
            "send --model",
            CliState::Gone,
            SeamState::Stays { symbol: "send" },
        )];
        assert_eq!(verdict("nxc", &cli(), &seam(&["send"]), &gone), None);

        // `--role` IS still on `send`, so recording it as gone must fail.
        let wrong = [row(
            "send --role",
            CliState::Gone,
            SeamState::Stays { symbol: "send" },
        )];
        let complaint = verdict("nxc", &cli(), &seam(&["send"]), &wrong)
            .expect("a flag that is still there must fail");
        assert!(
            complaint.contains("still on the nxc surface"),
            "{complaint}"
        );
    }

    #[test]
    fn a_seam_only_row_may_not_name_a_verb() {
        let record = [row(
            "inbox",
            CliState::NeverWas,
            SeamState::Stays { symbol: "inbox" },
        )];
        let complaint = verdict("nxc", &cli(), &seam(&["inbox"]), &record)
            .expect("a NeverWas row naming a verb must fail");
        assert!(complaint.contains("names one"), "{complaint}");
    }

    #[test]
    fn a_seam_only_row_is_identified_by_its_symbol_in_the_complaint() {
        let record = [row(
            "",
            CliState::NeverWas,
            SeamState::Stays {
                symbol: "definitions",
            },
        )];
        let complaint = verdict("nxc", &cli(), &seam(&["inbox"]), &record)
            .expect("a vanished seam-only read must fail");
        assert!(complaint.contains("(seam-only) definitions"), "{complaint}");
    }

    /// The entry point, not just the pure helper: a regression that logged instead of panicking
    /// would leave the gate green forever.
    #[test]
    #[should_panic(expected = "SEAM DISPOSITION")]
    fn assert_seam_dispositions_panics_on_a_broken_decision() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("engine.rs");
        std::fs::write(&src, "pub fn send() {}\n").unwrap();
        assert_seam_dispositions(
            "nxc",
            &cli(),
            &[src],
            &[row(
                "inbox",
                CliState::Standing,
                SeamState::Stays { symbol: "inbox" },
            )],
        );
    }

    #[test]
    #[should_panic(expected = "SEAM DISPOSITION GATE IS BLIND")]
    fn assert_seam_dispositions_panics_loudly_when_the_seam_cannot_be_read() {
        assert_seam_dispositions(
            "nxc",
            &cli(),
            &[PathBuf::from("/nonexistent/engine.rs")],
            &[],
        );
    }

    /// The counterpart to the two above, so "it always panics" cannot pass for "it panics when it
    /// should".
    #[test]
    fn assert_seam_dispositions_stays_silent_when_the_record_holds() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("engine.rs");
        std::fs::write(&src, "pub fn inbox() {}\n").unwrap();
        assert_seam_dispositions(
            "nxc",
            &cli(),
            &[src],
            &[row(
                "inbox",
                CliState::Standing,
                SeamState::Stays { symbol: "inbox" },
            )],
        );
    }
}
