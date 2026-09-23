//! **The persona coordinator collects** (nxf 6j6v.gn8b) — the durable queue that holds an answer
//! between "it arrived" and "the caller it belongs to has settled".
//!
//! # The situation this closes
//!
//! A declared CHANNEL knows when a round is complete: `expects` declares the set, `on_complete`
//! decides what the requester gets, and the requester's sleeping session is resumed ONCE with all
//! of it. **The persona path has no notion of a set.** A caller that commissions three personas in
//! parallel opens three threads, each reply resumes it separately, and the caller has to keep
//! count of who is still missing. Measured in this project's own use: the owner ran exactly that
//! shape from a marketing role and could not tell how a session gets all its answers. It does not
//! — it gets them one at a time, and only while it happens to be idle.
//!
//! The rest were not merely late, they were LOST to the caller: a reply arriving while the caller's
//! session is mid-turn is refused by the single-process guard (nxf 6j6v.7qtf), and until this the
//! whole of what happened was a `wake_skipped: {reason: "already_running"}` on the REPLIER's
//! receipt — a line the caller never sees, about an answer it will never be told about.
//!
//! # What is held, and what is not
//!
//! Only a wake that was ATTEMPTED and refused because the target session is working. An idle caller
//! is still woken immediately, with exactly the message it always got: this queue is what the
//! refusal becomes, not a new layer in front of every hand-off.
//!
//! # Why the queue is durable
//!
//! Because the alternative loses an answer exactly when it mattered. Holding a delivery in memory
//! means the process that holds it is a single point of failure between the moment a persona
//! answered and the moment its requester is told — and `nxc` is not a process, it is a command that
//! exits. A row in the workspace's own SQLite survives the command, the crash and the reboot.
//!
//! **Device-local and never synced**, like [`crate::session_map`] and the working-tree lease: it is
//! keyed by an internal session id, which names a live process on THIS machine. A peer that folded
//! it would be holding a delivery for a session it cannot start.
//!
//! # Delete AFTER the wake, never before
//!
//! [`ChatStore::release_held_wakes`] runs once the resume has been handed over. A crash in that
//! window means the caller is resumed with the same batch twice; the other order means the answers
//! are gone. Repeating an answer costs a reader one confused paragraph, losing one costs the round
//! — so the window is on the side of saying it twice.

use rusqlite::params;

use crate::channel::{
    attribution_notice, block_boundary, framing, untrusted_block, CollectedReply, ESCALATION_NOTICE,
};
use crate::error::Result;
use crate::store::ChatStore;

/// **One answer waiting for its caller** — what the coordinator holds, and everything the delivery
/// needs to render it.
///
/// The RAW facts, not the composed wake text: a delivery that batches four answers is composed once,
/// at the moment it goes out, from the same [`crate::channel::CollectedReply`] shape a channel round
/// uses. Storing the composed text instead would freeze four separate one-message wakes and there
/// would be nothing left to collect.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Held {
    /// The thread the answer landed in — WHICH commission this is, which is the fact a caller with
    /// three of them outstanding cannot recover from the body alone.
    pub thread_id: String,
    /// The message, so a re-attempt cannot enqueue the same answer twice (the row's unique key).
    pub message_id: String,
    /// Who answered, as the engine recorded it — the attribution
    /// [`crate::channel::ATTRIBUTION_NOTICE`] promises, from a column no answer can reach.
    pub sender: String,
    pub body: String,
    /// **"I need help, or a decision."** The one message that must not be batched away: it is
    /// delivered SET APART from the ordinary answers, under
    /// [`crate::channel::ESCALATION_NOTICE`].
    pub escalated: bool,
}

/// A [`Held`] answer as it comes back out of the queue. Declared field order = the JSON contract,
/// as everywhere else an app reads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HeldRow {
    /// The row id — what [`ChatStore::release_held_wakes`] deletes by, once the delivery has gone.
    pub id: i64,
    /// When it was held, so a delivery can be read in arrival order and a stuck queue can be aged.
    pub arrived: String,
    pub held: Held,
}

impl ChatStore {
    /// Hold `item` for `session` until that session settles. Answers whether a row was actually
    /// written — `false` means this exact message is already held, which is not an error.
    ///
    /// **Idempotent by the `(session, message_id)` unique index**, and that is what makes a
    /// re-attempted wake safe: `nxc tick` and a second reply both reach the same refusal, and
    /// neither may put the same answer in front of a caller twice.
    pub fn hold_wake(&mut self, session: &str, item: &Held, now: &str) -> Result<bool> {
        let n = self.connection().execute(
            "INSERT OR IGNORE INTO pending_wakes
                 (session, thread_id, message_id, sender, body, escalated, arrived)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session,
                item.thread_id,
                item.message_id,
                item.sender,
                item.body,
                item.escalated as i64,
                now
            ],
        )?;
        Ok(n > 0)
    }

    /// Everything held for `session`, in ARRIVAL order — `id` is monotonic per insert, so this is
    /// the order the answers came back in, which is the order a reader wants them.
    pub fn held_wakes(&self, session: &str) -> Result<Vec<HeldRow>> {
        let conn = self.connection();
        let mut st = conn.prepare(
            "SELECT id, arrived, thread_id, message_id, sender, body, escalated
               FROM pending_wakes WHERE session = ?1 ORDER BY id",
        )?;
        let rows = st
            .query_map([session], |r| {
                Ok(HeldRow {
                    id: r.get(0)?,
                    arrived: r.get(1)?,
                    held: Held {
                        thread_id: r.get(2)?,
                        message_id: r.get(3)?,
                        sender: r.get(4)?,
                        body: r.get(5)?,
                        escalated: r.get::<_, i64>(6)? != 0,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every session with something waiting for it, in a deterministic order — what the periodic
    /// sweep walks when a caller died without announcing its end.
    pub fn sessions_holding_wakes(&self) -> Result<Vec<String>> {
        let conn = self.connection();
        let mut st = conn.prepare("SELECT DISTINCT session FROM pending_wakes ORDER BY session")?;
        let rows = st
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Drop the rows a delivery has just carried. Called AFTER the resume was handed over — see
    /// this module's own doc for why that window falls on the side of repeating rather than losing.
    pub fn release_held_wakes(&mut self, ids: &[i64]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let mut removed = 0;
        let conn = self.connection();
        let mut st = conn.prepare("DELETE FROM pending_wakes WHERE id = ?1")?;
        for id in ids {
            removed += st.execute([id])?;
        }
        Ok(removed)
    }
}

// ---- the delivery ----------------------------------------------------------

/// **A commission of this caller's that has still not answered** — one line of the delivery's own
/// "and here is what you are still waiting for".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outstanding {
    pub thread_id: String,
    /// Who that thread still expects, in declaration order.
    pub expects: Vec<String>,
}

/// **How many outstanding commissions a delivery names before it stops counting.**
///
/// The list is bounded because the delivery is a PROMPT: a caller with sixty threads owing it
/// something would otherwise be handed sixty lines ahead of the answers it was woken for, and the
/// answers are what it was woken for. Past this the delivery says how many more there are, which is
/// the fact that matters at that size — `nxc status` is one call away and is the surface built for
/// reading a long list.
const MAX_NAMED_OUTSTANDING: usize = 10;

/// **"While you were working, this arrived"** (nxf 6j6v.gn8b) — ONE delivery, in the same shape a
/// channel round's `pass_through` uses: a head naming the count, the framing and the attribution
/// guarantee, then one delimited block per message carrying the engine's own record of who posted
/// and which commission it answers.
///
/// # The three things it says, and why each is load-bearing
///
/// 1. **The count.** "Everything that arrived" is not "all the answers" — two of three may answer
///    in a minute and the third in twenty.
/// 2. **What is still outstanding.** A wake with two answers is honest only if it says the third is
///    still owed. Without it the wakes are merely BATCHED and the bookkeeping still sits with the
///    caller, which is the defect this whole item exists to remove. The alternative the item
///    allowed — hold the delivery until the outstanding set is empty — was rejected: the measured
///    round-trip is eight to nine seconds, so holding a fast answer behind a slow one would leave a
///    caller asleep on work it could have done, and a set that never empties would hold it forever.
/// 3. **The hand-backs, SET APART.** "I need help, or a decision" is the one message that stops a
///    chain, and delivering it in a stack of four spends exactly the urgency it exists for. It gets
///    [`ESCALATION_NOTICE`] and a block of its own, ahead of the ordinary answers — the separation
///    is STRUCTURAL rather than a per-message flag, for the reason `CollectedReply` states where it
///    declines to carry one: adjacency is what makes a mark usable to the model reading it.
///
/// One boundary is chosen across BOTH groups, so a hand-back and an answer cannot end up delimited
/// differently in the same text.
pub fn compose_collected_wake(
    escalations: &[CollectedReply],
    answers: &[CollectedReply],
    outstanding: &[Outstanding],
    guidance: &str,
) -> String {
    let n = escalations.len() + answers.len();
    let all: Vec<CollectedReply> = escalations.iter().chain(answers).cloned().collect();
    let boundary = block_boundary(&all);
    // **Every engine-written instruction stands AHEAD of the untrusted blocks**, which is why the
    // addressee block (nxf 6j6v.dq59) goes here and not under the answers: a reader that meets an
    // instruction after somebody else's words has to decide which of the two the engine wrote, and
    // this module's whole delimiter discipline exists so it never has to.
    let mut out = format!(
        "While you were working, {n} message(s) arrived in threads you opened.\n{}\n{}{} {}",
        render_outstanding(outstanding),
        match guidance.is_empty() {
            true => String::new(),
            false => format!("{guidance}\n"),
        },
        framing(&boundary),
        attribution_notice(&boundary),
    );
    if !escalations.is_empty() {
        out.push_str(&format!("\n\n{ESCALATION_NOTICE}"));
        // The heading is written only when there is something to separate the hand-backs FROM.
        // Alone, they are the whole delivery and the notice above already says what they are.
        if !answers.is_empty() {
            out.push_str(&format!(
                "\n\nThe hand-back(s) — {} of the {n}:",
                escalations.len()
            ));
        }
        out.push_str(&format!("\n\n{}", untrusted_block(escalations, &boundary)));
    }
    if !answers.is_empty() {
        if !escalations.is_empty() {
            out.push_str(&format!("\n\nThe other {}:", answers.len()));
        }
        out.push_str(&format!("\n\n{}", untrusted_block(answers, &boundary)));
    }
    out
}

/// The "and here is what you are still waiting for" line — a SENTENCE when there is nothing left,
/// because "no more" is the fact that lets a caller stop counting.
fn render_outstanding(outstanding: &[Outstanding]) -> String {
    if outstanding.is_empty() {
        return "Nothing else is outstanding: every commission you opened has come back."
            .to_string();
    }
    let mut line = format!(
        "Still outstanding: {} commission(s) you opened have not answered yet.",
        outstanding.len()
    );
    for o in outstanding.iter().take(MAX_NAMED_OUTSTANDING) {
        line.push_str(&format!(
            "\n  {} — expects {}",
            o.thread_id,
            match o.expects.is_empty() {
                true => "nobody named".to_string(),
                false => o.expects.join(", "),
            }
        ));
    }
    if outstanding.len() > MAX_NAMED_OUTSTANDING {
        line.push_str(&format!(
            "\n  …and {} more — `nxc status` lists them all.",
            outstanding.len() - MAX_NAMED_OUTSTANDING
        ));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(thread: &str, message: &str, escalated: bool) -> Held {
        Held {
            thread_id: thread.to_string(),
            message_id: message.to_string(),
            sender: "local/coder".to_string(),
            body: "done".to_string(),
            escalated,
        }
    }

    #[test]
    fn a_held_answer_comes_back_whole_and_in_arrival_order() {
        let mut s = ChatStore::open_in_memory(1);
        assert!(s
            .hold_wake("s-1", &item("t-1", "m-1", false), "2026-09-07T10:00:00Z")
            .unwrap());
        assert!(s
            .hold_wake("s-1", &item("t-2", "m-2", true), "2026-09-07T10:00:01Z")
            .unwrap());
        let rows = s.held_wakes("s-1").unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.held.thread_id.as_str())
                .collect::<Vec<_>>(),
            vec!["t-1", "t-2"]
        );
        assert!(!rows[0].held.escalated);
        assert!(
            rows[1].held.escalated,
            "the bit that keeps it out of the batch"
        );
        assert_eq!(rows[0].arrived, "2026-09-07T10:00:00Z");
    }

    #[test]
    fn holding_the_same_message_twice_writes_one_row() {
        // A re-attempted wake — a second reply, a tick, a retried command — must not put the same
        // answer in front of a caller twice.
        let mut s = ChatStore::open_in_memory(1);
        assert!(s
            .hold_wake("s-1", &item("t-1", "m-1", false), "2026-09-07T10:00:00Z")
            .unwrap());
        assert!(
            !s.hold_wake("s-1", &item("t-1", "m-1", false), "2026-09-07T10:00:05Z")
                .unwrap(),
            "the second hold reports that it wrote nothing"
        );
        assert_eq!(s.held_wakes("s-1").unwrap().len(), 1);
    }

    #[test]
    fn a_queue_is_per_session_and_releasing_one_delivery_leaves_the_others() {
        let mut s = ChatStore::open_in_memory(1);
        s.hold_wake("s-1", &item("t-1", "m-1", false), "2026-09-07T10:00:00Z")
            .unwrap();
        s.hold_wake("s-1", &item("t-2", "m-2", false), "2026-09-07T10:00:01Z")
            .unwrap();
        s.hold_wake("s-2", &item("t-3", "m-3", false), "2026-09-07T10:00:02Z")
            .unwrap();
        assert_eq!(s.sessions_holding_wakes().unwrap(), vec!["s-1", "s-2"]);
        let first = s.held_wakes("s-1").unwrap()[0].id;
        assert_eq!(s.release_held_wakes(&[first]).unwrap(), 1);
        assert_eq!(s.held_wakes("s-1").unwrap().len(), 1);
        assert_eq!(s.held_wakes("s-2").unwrap().len(), 1);
        assert_eq!(s.sessions_holding_wakes().unwrap(), vec!["s-1", "s-2"]);
    }

    /// The addressee block a batched delivery carries — the guidance argument, as
    /// `deliver_held` composes it.
    const GUIDANCE: &str = "You may answer in one conversation — \"Fix the login redirect\" \
                            (t-owed) — and while that is so, `nxc reply` needs no thread id:";

    fn reply(sender: &str, body: &str) -> CollectedReply {
        CollectedReply::new(sender.to_string(), body.to_string())
    }

    #[test]
    fn the_batched_delivery_carries_the_addressee_block_ahead_of_the_untrusted_answers() {
        // This function had no test of its own at all (review of PR #452, Test Quality · Medium),
        // and the property it has to keep is a POSITION: every engine-written instruction stands
        // ahead of somebody else's words, so a reader never has to work out which of the two wrote
        // what it just read.
        let out = compose_collected_wake(
            &[],
            &[reply("local/coder", "the retry loop is fixed")],
            &[Outstanding {
                thread_id: "t-slow".into(),
                expects: vec!["local/frontend".into()],
            }],
            GUIDANCE,
        );
        assert!(out.contains("needs no thread id"), "{out}");
        let guidance_at = out.find("needs no thread id").expect("the block is in it");
        let answer_at = out
            .find("the retry loop is fixed")
            .expect("the answer is in it");
        assert!(
            guidance_at < answer_at,
            "the instruction stands ahead of the untrusted block: {out}"
        );
        assert!(
            out.contains("t-slow"),
            "and what is still outstanding is still named: {out}"
        );
    }

    #[test]
    fn a_delivery_with_nothing_open_says_nothing_about_answering() {
        // `reply_guidance` answers with an empty string when the caller owes and is owed nothing,
        // and an empty guidance must append NOTHING — not a heading, not a blank line. Same
        // contract as `wake_message`'s, one composer over.
        let out = compose_collected_wake(&[], &[reply("local/coder", "done")], &[], "");
        assert!(!out.contains("nxc reply"), "{out}");
        assert!(!out.contains("\n\n\n"), "no orphaned blank line: {out:?}");
        // …and the hand-back path composes the same way, because one boundary is chosen across both
        // groups.
        // `ESCALATION_NOTICE` names `nxc reply --escalate` itself, so what is asserted there is the
        // absence of the ADDRESSEE block rather than of the verb.
        let handed_back = compose_collected_wake(&[reply("local/coder", "I cannot")], &[], &[], "");
        assert!(handed_back.contains(ESCALATION_NOTICE), "{handed_back}");
        assert!(!handed_back.contains("needs no thread id"), "{handed_back}");
        assert!(
            !handed_back.contains("conversations are open"),
            "{handed_back}"
        );
    }

    #[test]
    fn a_released_queue_leaves_its_session_off_the_sweep() {
        let mut s = ChatStore::open_in_memory(1);
        s.hold_wake("s-1", &item("t-1", "m-1", false), "2026-09-07T10:00:00Z")
            .unwrap();
        let ids: Vec<i64> = s.held_wakes("s-1").unwrap().iter().map(|r| r.id).collect();
        s.release_held_wakes(&ids).unwrap();
        assert!(s.sessions_holding_wakes().unwrap().is_empty());
        assert!(s.held_wakes("s-1").unwrap().is_empty());
    }
}
