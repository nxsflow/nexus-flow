//! The per-member channel deadline, reset by transcript activity (nxf 6j6v.nf38).
//!
//! Since nxf 6j6v.pf6j a declared channel has one thread per member, and behind each of them stands
//! exactly ONE session whose transcript is observable. That is what finally lets the owner's rule
//! (6j6v.hq71, 2026-08-12) be implemented where it belongs: *"gilt pro Mitglied und wird bei jeder
//! Transkript-Ausgabe zurueckgesetzt"* — per member, restarted by every transcript write.
//!
//! # What is stored here, and what is NOT
//!
//! One row per MEMBER THREAD: the session to watch, the declared window, and the instant that
//! window currently falls due at. [`ChatStore::thread_quorum`] reads it through one `LEFT JOIN`
//! (see `quorum_sql`), so a member thread's `deadline` — and therefore its `stale` — is this row
//! whenever there is one, and the thread's own op-folded `deadline` register otherwise. There is
//! exactly one `stale` derivation in the crate and this does not add a second.
//!
//! # A PLAIN, device-local table — deliberately NOT op-folded
//!
//! Same argument as [`crate::liveness`] and [`crate::session_map`], and it decides the design rather
//! than merely accompanying it: the thing that resets this clock is `agent_transcript`, which is
//! device-local because "a transcript is by far the highest-volume thing chat produces" (the DDL
//! comment in `schema.rs` makes that case at length). A reset that rode the op log would emit one
//! op per sidecar flush — every 32 entries, for every member of every channel — which is precisely
//! the volume that table exists to keep out of the log. And a sync peer could not act on it anyway:
//! it holds no transcript for that session, so it can neither observe the activity nor decide the
//! member is alive.
//!
//! **The bound that follows, stated rather than left to be found.** A replica that did not run the
//! member's session still sees the thread's own op-folded `deadline` — the one stamped when the
//! channel opened — and will read the member as `stale` once it passes, however alive the member is
//! over here. That is not a regression (it is exactly what every replica did before this item), and
//! it is the same shape nxf 6j6v.2d5r already tracks for the consolidation claim: a decision that
//! needs local evidence cannot be made correctly by a replica that does not have it.
//!
//! # Growth
//!
//! One row per member thread, replaced in place — a member's second turn re-arms its existing row
//! rather than adding one. So this table is bounded by the number of channel members this device has
//! ever put to work, which is far below the message log it rides alongside.

use rusqlite::{params, OptionalExtension};

use crate::error::Result;
use crate::store::ChatStore;

/// One member thread's live deadline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MemberDeadline {
    /// The member thread this clock hangs on — one member, one thread, one session.
    pub(crate) thread_id: String,
    /// The internal session standing on that thread. Its transcript is the only thing that resets
    /// this clock; a sibling member's writes move the sibling's row and nothing here.
    pub(crate) session: String,
    /// The declared window, in seconds — what the clock is restarted BY.
    pub(crate) window_secs: i64,
    /// The absolute, UTC-normalized RFC3339 instant the window currently falls due at.
    pub(crate) deadline: String,
}

impl ChatStore {
    /// Start (or restart from scratch) `thread_id`'s clock: which session to watch, how long the
    /// declared window is, and when it currently falls due.
    ///
    /// Replaced wholesale rather than inserted beside, for [`crate::liveness::StepLiveness`]'s
    /// reason: a member thread has one clock at a time, so a second row could only ever describe a
    /// turn that is over.
    pub(crate) fn arm_member_deadline(
        &mut self,
        thread_id: &str,
        session: &str,
        window_secs: i64,
        deadline: &str,
    ) -> Result<()> {
        self.connection().execute(
            "INSERT OR REPLACE INTO channel_member_deadline
                (thread_id, session, window_secs, deadline)
             VALUES (?1, ?2, ?3, ?4)",
            params![thread_id, session, window_secs, deadline],
        )?;
        Ok(())
    }

    /// `thread_id`'s clock, or `None` when none was ever armed. Two shapes have none: any thread
    /// that is not a member of a declared channel with a declared `timeout`, and a member whose
    /// deadline was given as an absolute INSTANT — that names a point in time, so there is no window
    /// to restart it by and nothing should move it (`facade::DeadlineSpec` states the rule).
    pub(crate) fn member_deadline(&self, thread_id: &str) -> Result<Option<MemberDeadline>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT thread_id, session, window_secs, deadline
                   FROM channel_member_deadline WHERE thread_id = ?1",
                [thread_id],
                |r| {
                    Ok(MemberDeadline {
                        thread_id: r.get(0)?,
                        session: r.get(1)?,
                        window_secs: r.get(2)?,
                        deadline: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    /// **The reset**: restart the declared window from `now` for every member thread `session`
    /// stands on. Returns how many clocks moved — `0` for a session that is not a channel member,
    /// which is the ordinary case and not an error.
    ///
    /// This is what `ChatStore::append_transcript` (`transcript.rs`) calls on every flush.
    /// It is keyed on the SESSION because that is what the transcript is keyed on, and a member
    /// thread has exactly one — so "this member showed a sign of life" and "this clock restarts"
    /// are the same event by construction, with no run, no step and no board in between.
    pub(crate) fn reset_member_deadlines_of_session(
        &mut self,
        session: &str,
        now: &str,
    ) -> Result<usize> {
        let threads: Vec<String> = {
            let conn = self.connection();
            let mut st =
                conn.prepare("SELECT thread_id FROM channel_member_deadline WHERE session = ?1")?;
            let rows = st.query_map([session], |r| r.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut moved = 0;
        for thread in &threads {
            if self.reset_member_deadline(thread, now)?.is_some() {
                moved += 1;
            }
        }
        Ok(moved)
    }

    /// Restart `thread_id`'s declared window from `now`, returning the new deadline — or `None`
    /// when no clock is armed there.
    ///
    /// The second caller (beside the transcript flush) is the supervisor handing a member its NEXT
    /// TURN: a turn declared after the previous window has run out would otherwise be born `stale`,
    /// and `stale` is exactly what the settle decision and `nxc tick` both read as "due".
    pub(crate) fn reset_member_deadline(
        &mut self,
        thread_id: &str,
        now: &str,
    ) -> Result<Option<String>> {
        let Some(armed) = self.member_deadline(thread_id)? else {
            return Ok(None);
        };
        // Through the one place a duration becomes a deadline, so a clock restarted here sorts
        // against a reader's wall clock exactly like the one the channel stamped at fan-out.
        let deadline = crate::facade::instant_after(
            now,
            time::Duration::seconds(armed.window_secs),
            &format!("the declared timeout of member thread {thread_id}"),
        )?;
        self.connection().execute(
            "UPDATE channel_member_deadline SET deadline = ?1 WHERE thread_id = ?2",
            params![deadline, thread_id],
        )?;
        Ok(Some(deadline))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::TranscriptEntry;

    fn entry() -> TranscriptEntry {
        TranscriptEntry {
            kind: "assistant".into(),
            at: None,
            tool_use_id: None,
            parent_tool_use_id: None,
            subagent_type: None,
            data: serde_json::json!({"text": "working"}),
        }
    }

    #[test]
    fn nothing_is_armed_until_something_arms_it_and_resetting_it_is_a_clean_no_op() {
        let mut s = ChatStore::open_in_memory(1);
        assert_eq!(s.member_deadline("t-1").unwrap(), None);
        assert_eq!(
            s.reset_member_deadline("t-1", "2026-08-16T10:00:00Z")
                .unwrap(),
            None,
            "a thread with no declared window has no clock to restart"
        );
        assert_eq!(
            s.reset_member_deadlines_of_session("s-1", "2026-08-16T10:00:00Z")
                .unwrap(),
            0,
            "and a session that is nobody's channel member moves nothing"
        );
    }

    #[test]
    fn a_transcript_flush_restarts_the_declared_window_from_the_moment_of_the_flush() {
        // The whole item in one unit: the clock is armed for thirty minutes at 10:00, the session
        // writes at 10:25, and the clock now runs to 10:55 — not to 10:30.
        let mut s = ChatStore::open_in_memory(1);
        s.arm_member_deadline("t-a", "s-a", 30 * 60, "2026-08-16T10:30:00Z")
            .unwrap();
        s.append_transcript_with_retention("s-a", &[entry()], None, Some("2026-08-16T10:25:00Z"))
            .unwrap();
        assert_eq!(
            s.member_deadline("t-a").unwrap().unwrap().deadline,
            "2026-08-16T10:55:00Z"
        );
    }

    #[test]
    fn a_flush_moves_only_the_clock_of_the_session_that_flushed() {
        let mut s = ChatStore::open_in_memory(1);
        s.arm_member_deadline("t-a", "s-a", 30 * 60, "2026-08-16T10:30:00Z")
            .unwrap();
        s.arm_member_deadline("t-b", "s-b", 30 * 60, "2026-08-16T10:30:00Z")
            .unwrap();
        s.append_transcript_with_retention("s-a", &[entry()], None, Some("2026-08-16T10:25:00Z"))
            .unwrap();
        assert_eq!(
            s.member_deadline("t-b").unwrap().unwrap().deadline,
            "2026-08-16T10:30:00Z",
            "a sibling member's transcript is not a sign of life from this one"
        );
    }

    #[test]
    fn a_flush_with_no_readable_clock_leaves_the_deadline_where_it_was() {
        // `append_transcript` cannot fail a flush — the reference instant is an `Option` precisely
        // because resolving it may not work — so a flush with no clock behind it still lands and
        // costs the reset, never the turn's evidence.
        let mut s = ChatStore::open_in_memory(1);
        s.arm_member_deadline("t-a", "s-a", 30 * 60, "2026-08-16T10:30:00Z")
            .unwrap();
        s.append_transcript_with_retention("s-a", &[entry()], None, None)
            .unwrap();
        assert_eq!(
            s.member_deadline("t-a").unwrap().unwrap().deadline,
            "2026-08-16T10:30:00Z"
        );
        assert_eq!(s.transcript_progress("s-a").unwrap(), 1, "the entry landed");
    }

    #[test]
    fn arming_again_replaces_the_clock_rather_than_adding_a_second_one() {
        let mut s = ChatStore::open_in_memory(1);
        s.arm_member_deadline("t-a", "s-a", 30 * 60, "2026-08-16T10:30:00Z")
            .unwrap();
        s.arm_member_deadline("t-a", "s-a", 20 * 60, "2026-08-16T11:20:00Z")
            .unwrap();
        let rows: i64 = s
            .connection()
            .query_row("SELECT COUNT(*) FROM channel_member_deadline", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(rows, 1);
        let armed = s.member_deadline("t-a").unwrap().unwrap();
        assert_eq!(armed.window_secs, 20 * 60);
        assert_eq!(armed.deadline, "2026-08-16T11:20:00Z");
    }

    #[test]
    fn the_armed_clock_is_what_the_quorum_reads_as_the_member_threads_deadline() {
        // The one `stale` derivation in the crate answers from this row — no second staleness rule
        // was added anywhere.
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "local/sup");
        s.open_thread(
            "t-a",
            &crate::model::ThreadRoot {
                origin: "local".into(),
                channel_id: "c-1".into(),
                opener: "local/__channel__".into(),
                created: "2026-08-16T10:00:00Z".into(),
                parent: None,
            },
            "local/__channel__",
        );
        s.set_expects_reply_from("t-a", r#"["local/bob"]"#, "local/__channel__");
        s.set_deadline("t-a", "2026-08-16T10:30:00Z", "local/__channel__");
        s.arm_member_deadline("t-a", "s-a", 30 * 60, "2026-08-16T10:30:00Z")
            .unwrap();

        let past = "2026-08-16T10:45:00Z";
        assert!(s.thread_quorum("t-a", past).unwrap().unwrap().stale);
        s.append_transcript_with_retention("s-a", &[entry()], None, Some("2026-08-16T10:29:00Z"))
            .unwrap();
        let q = s.thread_quorum("t-a", past).unwrap().unwrap();
        assert_eq!(q.deadline.as_deref(), Some("2026-08-16T10:59:00Z"));
        assert!(
            !q.stale,
            "the restarted clock is what the quorum reads, not the op-folded register: {q:?}"
        );
    }
}
