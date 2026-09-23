//! **How the requester learns the answer** (nxf 6j6v.7qfm) — the one block `send` and `reply`
//! hand back, in prose for a human and as fields for a machine.
//!
//! # What was wrong with the line this replaces
//!
//! `nxc send --to <persona>` answered with `opened thread <id> in dm:<24 hex> — reply with
//! `nxc reply --thread <id> "…"``, and that was the caller's ONLY guidance. It answers the wrong
//! question: the reader is the REQUESTER, at the moment nobody has answered yet, and
//! `reply --thread` is how to keep TALKING — the addressee's verb, or the requester's much later.
//! How to learn that an answer arrived was not mentioned at all.
//!
//! Measured with the owner on 2026-09-07, from the threads' own message timestamps: a declared
//! persona answers in **eight to nine seconds**. The return path is not slow, it is SILENT — three
//! commissions were sent, three thread ids came back, and not one answer would have been seen
//! without being told what to ask for.
//!
//! # The two readers, and why one line cannot serve both
//!
//! `nxc` already separates a registered persona from everybody else — the `--stream` refusal
//! ([`crate::cli`]) gates on exactly that — and for the two the correct advice is OPPOSITE. A human
//! (or any caller the engine did not start) has nobody to wake it, so it must come back and look:
//! [`AwaitHow::Poll`]. A registered persona is RESUMED by the coordinator when the answer arrives,
//! so polling would be a session burning turns on a question it will be handed:
//! [`AwaitHow::Resume`].
//!
//! # The predicates are a promise
//!
//! [`Await::done_when`] and [`Await::stopped_when`] name FIELDS of the payload
//! [`Await::poll`] produces, and they are a published contract rather than a hint — see each
//! field's own doc for the connective it carries, and
//! `crates/chat/tests/the_return_path_is_named.rs`, which fails if either meaning changes.
//!
//! Both terminal states are named, not only the good one: a loop that waits for `done_when` alone
//! hangs forever when the round STOPPED — an escalation releases nothing, and a silent step runs
//! into its window.

use serde::Serialize;

use crate::store::ThreadQuorum;

/// **The window to report for a thread — the declared one, and only while somebody stands behind
/// it** (nxf 6j6v.0vd9).
///
/// [`Await::deadline`] is the field a waiting caller keys on: it is what separates "wait inline,
/// this comes back in nine seconds" from "come back later". Both surfaces that fill it used to read
/// the register straight off the thread, which reports the window a round was opened with whether or
/// not that round is still running — so a reply into a thread nobody owes an answer on came back
/// with a due date under it. Measured on 2026-09-08 in a live `watch-bundestag` operation: the owner
/// waited twenty minutes on such a date for work nobody had been asked to do.
///
/// The condition is `outstanding ≠ ∅`, which is not a new rule but the one
/// [`ThreadQuorum::stale`] is already derived with (`deadline ≠ NULL ∧ now > deadline ∧
/// outstanding ≠ ∅`). A deadline the thread can never go `stale` against is an instant nothing will
/// ever happen at, and reporting it invites exactly the wait it cannot end.
///
/// `None` for a thread with no board at all — there was never a window to report.
pub fn deadline_of(quorum: Option<&ThreadQuorum>) -> Option<String> {
    quorum
        .filter(|q| !q.outstanding.is_empty())
        .and_then(|q| q.deadline.clone())
}

/// **The sub-round the party that owes `thread` an answer commissioned ITSELF, and is waiting on**
/// (nxf 6j6v.hw2t) — the open child threads of `thread` that were opened by a handle `thread` is
/// still waiting for. Empty means nobody here is waiting on work of their own.
///
/// # Why this is DERIVED and not declared
///
/// From the outside, a role that commissions a sub-round and then ends its turn looks exactly like
/// one that simply forgot to answer — and that is the whole of the measured damage: on 2026-09-08 a
/// `head-of-marketing` consulted four specialists ad hoc, had no way to say "I am waiting on them",
/// and spent `--escalate` on it. The operation view then reported `NEEDS DECISION` while four
/// sessions were working.
///
/// The engine already KNOWS. It opened those threads itself, on this caller's behalf, and hung them
/// under the thread the caller stands in ([`crate::orchestration::open_child_thread`], whose
/// `opener` is the caller's own qualified handle). So the state is a fact about the RECORD, not a
/// token anybody has to remember to say — which is the property nxf 6j6v.553s asked of every other
/// exit, and the reason no fourth reply verb was built for this one.
///
/// # The predicate, term by term
///
/// * **`thread` still owes an answer** — `outstanding` names who. A discharged thread is waiting for
///   nobody, so nobody there can be waiting on anything. This term is inside the opener test below;
///   the early return that repeats it is a short-circuit and says so at the line.
/// * **the child is still OPEN** — a sub-round that came back is not being waited on, and the
///   caller has been woken with it.
/// * **the child's `opener` is one of `thread`'s `outstanding`** — the party that owes the answer is
///   the party that commissioned the sub-round. Without this term ANY open thread hanging under
///   `thread` would count: a bystander's conversation, a branch some third party opened in the same
///   tree, work that has nothing to do with the answer being waited for. (It is NOT what keeps a
///   channel supervisor's own fan-out out of the answer — the engine-identity term below does that,
///   and this doc credited the wrong term until the review of PR #460, Code Quality #3.)
/// * **and that opener is not an engine identity** — the channel supervisor fans out under a thread
///   that expects the supervisor, which satisfies every term above and is not this state at all: it
///   is the engine's own machinery, it needs no turn back, and nothing about it reaches an agent.
///
/// # Thread-id ordered, and it is SORTED here rather than assumed of the caller
///
/// Two callers feed this from two different reads and they do not agree on an order: the status
/// report walks the forest's edges (thread-id ordered), while the `reply` write point reads the
/// children's quorums in bulk — and [`crate::store::ChatStore::thread_quorums`] orders by
/// `(channel_id, thread_id, …)`, so two consultations in two different direct channels come back in
/// channel-hash order. Both surfaces name these ids to a reader, and one of them names the FIRST as
/// the round to go and read, so "whichever the caller happened to hand in first" is not an order.
/// Thread ids are ULIDs, so sorting them is chronological — oldest consultation first.
pub fn own_open_sub_round<'a>(
    thread: &ThreadQuorum,
    children: impl IntoIterator<Item = &'a ThreadQuorum>,
) -> Vec<String> {
    // A SHORT-CIRCUIT of the `outstanding` term below, not a second rule — mutating it away leaves
    // every answer unchanged, because `any()` over an empty `outstanding` is already false. It is
    // here because `facade::status` asks this of EVERY thread on a report and most of them are
    // discharged: the early return skips their children entirely. Said plainly so nobody reads it
    // as the place the discharged case is decided and edits one of the two halves alone.
    if thread.outstanding.is_empty() {
        return Vec::new();
    }
    opened_by(&thread.outstanding, children)
}

/// **The same question asked of ONE party: is `handle` waiting on a round IT commissioned?** (nxf
/// 6j6v.hw2t, review of PR #460 · Code Quality #2, corroborated independently as Integrity #2).
///
/// [`own_open_sub_round`] is a fact about the THREAD — "somebody this thread waits for is itself
/// waiting" — and that is the right question for a status row, which describes the thread. It is
/// the WRONG question at the `reply` write point, which refuses one named caller's claim to be
/// finished: on a board with two outstanding handles it would refuse A because B has a consultation
/// open, and A would be told it is waiting on a round it never commissioned.
///
/// No production call site opens such a board today — every one of them writes a single-element
/// `expects_reply_from` — so the defect is dormant rather than live. It is reachable all the same
/// through [`crate::facade::set_expects`], which is public and takes a list, and a refusal that can
/// be wrong about WHO is waiting is exactly the kind of false map this item exists to remove. Two
/// reviewers arriving at the same code path from different directions is what settled it.
///
/// Empty when `handle` owes this thread nothing: a caller that is not among `outstanding` is not
/// claiming to be finished with anything here, so there is nothing to refuse.
pub fn own_open_sub_round_of<'a>(
    handle: &str,
    thread: &ThreadQuorum,
    children: impl IntoIterator<Item = &'a ThreadQuorum>,
) -> Vec<String> {
    if !thread.outstanding.iter().any(|owed| owed == handle) {
        return Vec::new();
    }
    opened_by(std::slice::from_ref(&handle.to_string()), children)
}

/// The shared body: the open children `waiting_for` opened, thread-id sorted. Both entry points run
/// it so the four terms have exactly one implementation — the property that stops a status row and
/// a refusal ever disagreeing about the same tree.
fn opened_by<'a>(
    waiting_for: &[String],
    children: impl IntoIterator<Item = &'a ThreadQuorum>,
) -> Vec<String> {
    let mut waiting: Vec<String> = children
        .into_iter()
        .filter(|child| !child.outstanding.is_empty())
        .filter(|child| {
            child.opener.as_deref().is_some_and(|opener| {
                !crate::orchestration::is_engine_identity(opener)
                    && waiting_for.iter().any(|owed| owed == opener)
            })
        })
        .map(|child| child.thread_id.clone())
        .collect();
    waiting.sort();
    waiting
}

/// The read a polling caller runs, as argv rather than as a command line: a caller that has only
/// the receipt must not have to quote a shell.
///
/// `nxc threads show <id> --json` is the one payload every field below is named against — it
/// carries `complete`, `outstanding`, `stale`, `escalated` and `messages` in a single read, which
/// is why the predicates can be fields at all.
fn poll_argv(thread_id: &str) -> Vec<String> {
    ["nxc", "threads", "show", thread_id, "--json"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

/// **Where the answer sits in the polled payload** — the newest message of the thread.
///
/// `messages` is ordered oldest-first by the causal `(lamport, site, message_id)` key every thread
/// read in this crate uses ([`crate::store::ChatStore::messages_in_thread`]), so the last element
/// is the newest thing said. Written in the `-1` idiom because the caller reading it is as likely
/// to be a `jq` expression as a program.
pub const ANSWER_AT: &str = "messages[-1]";

/// Whether this caller has to come back and look, or will be handed the answer.
///
/// `#[non_exhaustive]` for [`crate::orchestration::WakeSkipReason`]'s reason: this goes out with
/// the open-source launch, and a third way of learning an answer must be an additive minor rather
/// than a break for every downstream `match`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AwaitHow {
    /// **Nobody will wake you.** Run [`Await::poll`] until the round reaches one of the two
    /// terminal states below.
    Poll,
    /// **Do not poll — you are resumed with it.** The caller is a registered persona, so the
    /// coordinator starts its session again with the answer when it arrives (nxf 6j6v.gn8b
    /// collects those into one wake). [`Await::poll`] is `null` for exactly this reader.
    Resume,
}

/// **The round is FINISHED — all of these hold at once.** An AND, and the two terms are deliberately
/// redundant: `complete` is `E ≠ ∅ ∧ outstanding = ∅` ([`crate::store::ThreadQuorum::complete`]),
/// so it already implies the empty list. A caller that checks only `outstanding == []` would read a
/// thread that expects NOBODY as finished, which is why the flag is named first and named at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoneWhen {
    /// `complete` on the polled payload must be `true`.
    pub complete: bool,
    /// `outstanding` on the polled payload must be empty.
    pub outstanding: Vec<String>,
}

/// **The round has STOPPED — ANY of these means no answer is coming.** An OR, unlike its
/// neighbour, and the difference is the whole reason both exist: a loop that waits for
/// [`DoneWhen`] alone hangs forever on a round that ended without an answer.
///
/// `stale` is the declared window run out with a reply still owed
/// ([`crate::store::ThreadQuorum::stale`]); `escalated` is the agent side handing the task back
/// rather than answering it ([`crate::facade::ThreadBoardView::escalated`]). Neither releases
/// anything by itself — a human or a supervisor decides what happens next — which is exactly what
/// separates "wait" from "it is stuck".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoppedWhen {
    /// `stale` on the polled payload being `true` is enough.
    pub stale: bool,
    /// `escalated` on the polled payload being `true` is enough.
    pub escalated: bool,
}

/// **How to learn the answer to what was just sent** — the same content the human line carries,
/// as fields, because an agent must not parse prose.
///
/// Declared field order = the JSON contract, as everywhere else on these receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct Await {
    /// Which of the two readers this is, and therefore whether [`poll`](Await::poll) is a
    /// programme or a `null`. The typed discriminator a caller branches on — never the presence of
    /// a key.
    pub how: AwaitHow,
    /// The argv to run to see where the round stands; `null` for [`AwaitHow::Resume`], where
    /// running it would be a session polling for something it is about to be handed.
    pub poll: Option<Vec<String>>,
    /// The state that means finished — see [`DoneWhen`] for the connective.
    pub done_when: DoneWhen,
    /// The markers that mean stopped — see [`StoppedWhen`] for the connective.
    pub stopped_when: StoppedWhen,
    /// Where the answer sits in the polled payload: [`ANSWER_AT`].
    ///
    /// Present for [`AwaitHow::Resume`] too, and not as an oversight: a resumed persona is handed
    /// the answer, but the thread it lands in is the same one, and a caller that does look reads it
    /// in the same place.
    pub answer_at: &'static str,
    /// **How patient to be** — the absolute instant the answer is due at, when a window was
    /// declared for it, and `null` when none was.
    ///
    /// It is the third decision of nxf 6j6v.7qfm and the one the engine simply withheld: nine
    /// seconds for a persona means a caller can wait inline, while a channel step with a declared
    /// `timeout:` means come back later. The engine has always known which of the two this is.
    pub deadline: Option<String>,
}

impl Await {
    /// The block for a caller that has to come back and look — a human at a terminal, or any
    /// process the engine did not start.
    pub fn poll(thread_id: &str, deadline: Option<String>) -> Await {
        Await {
            how: AwaitHow::Poll,
            poll: Some(poll_argv(thread_id)),
            ..Await::predicates(deadline)
        }
    }

    /// The block for a registered persona: the opposite advice in the same field.
    pub fn resumed(deadline: Option<String>) -> Await {
        Await {
            how: AwaitHow::Resume,
            poll: None,
            ..Await::predicates(deadline)
        }
    }

    /// The half both readers share, in ONE place — the predicates are the same promise whoever
    /// reads them, and a second literal is how the two shapes would come to disagree.
    fn predicates(deadline: Option<String>) -> Await {
        Await {
            how: AwaitHow::Poll,
            poll: None,
            done_when: DoneWhen {
                complete: true,
                outstanding: Vec::new(),
            },
            stopped_when: StoppedWhen {
                stale: true,
                escalated: true,
            },
            answer_at: ANSWER_AT,
            deadline,
        }
    }

    /// Pick the reader: `persona` is whether the caller is a REGISTERED persona — the same
    /// question [`crate::persona::resolve_identity`] answers for the `--stream` refusal, asked once
    /// here so the two surfaces cannot come to disagree about who is at the other end.
    pub fn for_caller(persona: bool, thread_id: &str, deadline: Option<String>) -> Await {
        match persona {
            true => Await::resumed(deadline),
            false => Await::poll(thread_id, deadline),
        }
    }

    /// **The same content, for the human** — the lines a terminal reader gets under the receipt's
    /// own first line, already indented and without a trailing newline.
    ///
    /// The direct-channel hash is deliberately absent from every line here: `dm:<24 hex>` is
    /// derived from the two handles so both sides share one id, and the reader already knows who
    /// they wrote to. It belongs in the record ([`crate::surface::SendToReceipt::channel`]), not in
    /// the guidance.
    pub fn render_human(&self, thread_id: &str) -> String {
        let due = match &self.deadline {
            Some(d) => format!("\n  The answer is due by {d}."),
            None => String::new(),
        };
        match self.how {
            // The two reads a requester actually takes next, padded to one column so the ids line
            // up, and printed IN FULL: these lines are meant to be run, and an abbreviated id is
            // not.
            AwaitHow::Poll => format!(
                "\n  The answer lands in this thread.\n    \
                 nxc threads show    {thread_id}    — the conversation\n    \
                 nxc status --thread {thread_id}    — where it stands{due}\n\n  \
                 To watch instead of coming back: add --stream next time."
            ),
            // No `--stream` line for this reader: it is refused to a registered persona (see
            // `crate::cli`), so offering it would be advice that cannot be taken.
            AwaitHow::Resume => format!(
                "\n  The answer lands in this thread. You are resumed with it when it arrives — \
                 do not poll for it.{due}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_readers_differ_in_exactly_one_field_and_agree_on_the_predicates() {
        // The promise is the SAME for both; what differs is whether the caller has to look.
        let polling = Await::poll("m-1", None);
        let resumed = Await::resumed(None);
        assert_eq!(polling.how, AwaitHow::Poll);
        assert_eq!(resumed.how, AwaitHow::Resume);
        assert_eq!(
            polling.poll.as_deref(),
            Some(
                ["nxc", "threads", "show", "m-1", "--json"]
                    .map(str::to_string)
                    .as_slice()
            )
        );
        assert_eq!(
            resumed.poll, None,
            "a persona is resumed, so it must not poll"
        );
        assert_eq!(polling.done_when, resumed.done_when);
        assert_eq!(polling.stopped_when, resumed.stopped_when);
        assert_eq!(polling.answer_at, resumed.answer_at);
    }

    #[test]
    fn the_human_block_names_no_direct_channel_hash_and_offers_stream_only_where_it_is_allowed() {
        let polling = Await::poll("m-1", None).render_human("m-1");
        assert!(polling.contains("nxc threads show    m-1"), "{polling}");
        assert!(polling.contains("nxc status --thread m-1"), "{polling}");
        assert!(polling.contains("--stream"), "{polling}");
        assert!(!polling.contains("dm:"), "{polling}");
        let resumed = Await::resumed(None).render_human("m-1");
        assert!(resumed.contains("do not poll"), "{resumed}");
        assert!(
            !resumed.contains("--stream"),
            "--stream is refused to a persona, so it must not be offered to one: {resumed}"
        );
    }

    /// A child thread as [`own_open_sub_round`] weighs it: who opened it, and whether it is still
    /// open (`owed` non-empty).
    fn child(id: &str, opener: &str, owed: &[&str]) -> ThreadQuorum {
        ThreadQuorum {
            thread_id: id.into(),
            channel_id: None,
            opener: Some(opener.into()),
            expects: owed.iter().map(|h| (*h).to_string()).collect(),
            replied: Vec::new(),
            outstanding: owed.iter().map(|h| (*h).to_string()).collect(),
            complete: owed.is_empty(),
            deadline: None,
            stale: false,
            name: None,
            held: false,
        }
    }

    #[test]
    fn a_caller_that_commissioned_its_own_round_is_waiting_on_exactly_those_threads() {
        // The measured shape (nxf 6j6v.hw2t): `head-of-marketing` owes the root an answer and has
        // opened four consultations of its own. Two are still running, one came back.
        let root = quorum(&["47jy/head-of-marketing"]);
        // Handed in out of order on purpose: the bulk quorum read orders by channel first, and
        // these two consultations live in two different direct channels.
        let kids = [
            child("m-c", "47jy/head-of-marketing", &["47jy/mkt-preis"]),
            child("m-b", "47jy/head-of-marketing", &[]),
            child(
                "m-a",
                "47jy/head-of-marketing",
                &["47jy/mkt-positionierung"],
            ),
        ];
        assert_eq!(
            own_open_sub_round(&root, &kids),
            vec!["m-a".to_string(), "m-c".to_string()],
            "the open ones, and only those, oldest first — an answered consultation is not being \
             waited on, and the order is the ids' own rather than the caller's"
        );
    }

    #[test]
    fn the_caller_scoped_form_answers_about_that_caller_and_nobody_else() {
        // Review of PR #460, Code Quality #2 / Integrity #2. The thread-level question and the
        // caller-level one give DIFFERENT answers on a board with two outstanding handles, and the
        // `reply` write point must ask the second: refusing A because B is consulting somebody
        // would tell A it is waiting on a round it never opened.
        let mut board = quorum(&["47jy/a", "47jy/b"]);
        board.expects = vec!["47jy/a".into(), "47jy/b".into()];
        let kids = [child("m-b1", "47jy/b", &["47jy/specialist"])];

        assert_eq!(
            own_open_sub_round(&board, &kids),
            vec!["m-b1".to_string()],
            "the THREAD is waiting on a sub-round — true, and it is what the status row says"
        );
        assert!(
            own_open_sub_round_of("47jy/a", &board, &kids).is_empty(),
            "…and A is not: it opened nothing, so there is nothing to refuse it for"
        );
        assert_eq!(
            own_open_sub_round_of("47jy/b", &board, &kids),
            vec!["m-b1".to_string()],
            "…while B is, and a finished claim from B would be false"
        );
    }

    #[test]
    fn a_caller_the_thread_is_not_waiting_for_is_waiting_on_nothing_here() {
        // The scoped form's own first term. A bystander, or a requester taking another turn in its
        // own conversation, owes this thread nothing — so it claims to be finished with nothing,
        // and the refusal must not reach it however many rounds it has open elsewhere.
        let board = quorum(&["47jy/a"]);
        let kids = [child("m-x", "47jy/somebody-else", &["47jy/specialist"])];
        assert!(own_open_sub_round_of("47jy/somebody-else", &board, &kids).is_empty());
    }

    #[test]
    fn a_thread_nobody_owes_an_answer_on_is_waiting_on_nothing() {
        // The discharged case, and it is not a shortcut: the round is over, so whatever hangs under
        // it belongs to somebody else's turn.
        let root = quorum(&[]);
        let kids = [child("m-a", "47jy/head-of-marketing", &["47jy/mkt-preis"])];
        assert!(own_open_sub_round(&root, &kids).is_empty());
    }

    #[test]
    fn a_round_somebody_else_commissioned_is_not_this_callers_waiting() {
        // The term that keeps this from firing on any open child: the sub-round has to have been
        // opened by the party that owes the answer. A bystander's thread under the same parent says
        // nothing about whether the owing party is waiting.
        let root = quorum(&["47jy/head-of-marketing"]);
        let kids = [child("m-a", "47jy/somebody-else", &["47jy/mkt-preis"])];
        assert!(own_open_sub_round(&root, &kids).is_empty());
    }

    #[test]
    fn the_channel_supervisors_own_fan_out_is_not_a_role_waiting_on_a_sub_round() {
        // A channel thread expects the supervisor and the supervisor opens the step slots under it,
        // which satisfies every other term. It is the engine's own machinery: nobody is owed a turn
        // back, and no agent ever reads this row as an instruction.
        let root = quorum(&["local/__channel__"]);
        let kids = [child("m-a", "local/__channel__", &["local/coder"])];
        assert!(own_open_sub_round(&root, &kids).is_empty());
    }

    /// A quorum with `outstanding` set from `owed` and a window on it — the two facts
    /// [`deadline_of`] weighs, with the rest of the record at its uninteresting defaults.
    fn quorum(owed: &[&str]) -> ThreadQuorum {
        ThreadQuorum {
            thread_id: "m-1".into(),
            channel_id: None,
            opener: None,
            expects: vec!["local/coder".into()],
            replied: Vec::new(),
            outstanding: owed.iter().map(|h| (*h).to_string()).collect(),
            complete: owed.is_empty(),
            deadline: Some("2026-09-08T22:41:34Z".into()),
            stale: false,
            name: None,
            held: false,
        }
    }

    #[test]
    fn a_window_is_reported_only_while_somebody_still_owes_an_answer_against_it() {
        // nxf 6j6v.0vd9's second half, at the one function that decides it. The condition is
        // `outstanding != []` and not `complete`, deliberately: a thread that expects NOBODY is
        // not `complete` either, and its window is just as unanswerable.
        assert_eq!(
            deadline_of(Some(&quorum(&["local/coder"]))).as_deref(),
            Some("2026-09-08T22:41:34Z"),
            "somebody owes an answer here, so how patient to be is a real question"
        );
        assert_eq!(
            deadline_of(Some(&quorum(&[]))),
            None,
            "the round is discharged — an instant nothing will happen at must not be reported"
        );
        assert_eq!(
            deadline_of(None),
            None,
            "a thread with no board never had a window to report"
        );
    }

    #[test]
    fn a_declared_window_says_how_patient_to_be_in_both_renderings() {
        let a = Await::poll("m-1", Some("2026-09-07T12:00:00Z".to_string()));
        assert_eq!(a.deadline.as_deref(), Some("2026-09-07T12:00:00Z"));
        assert!(a
            .render_human("m-1")
            .contains("due by 2026-09-07T12:00:00Z"));
        assert!(Await::resumed(Some("2026-09-07T12:00:00Z".to_string()))
            .render_human("m-1")
            .contains("due by 2026-09-07T12:00:00Z"));
    }
}
