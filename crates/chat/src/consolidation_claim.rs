//! The CONSOLIDATION claim (nxf 6j6v.296n, widened to both output forms by 6j6v.e9qj): who gets to
//! consolidate when two callers complete the SAME declared-channel thread at nearly the same moment.
//!
//! `on_channel_complete` is a check-then-act in BOTH of a consolidator's output forms — read the
//! quorum, decide it just completed, re-target `expects_reply_from` to the form's own marker, then
//! act — and the store's writes are one autocommit transaction per op, so nothing serializes that
//! sequence. A completing reply landing exactly as `nxc tick`'s scheduled timer fires for the
//! same thread gives two callers the same pre-retarget state, and:
//!
//! * under a FOLD, each spawns its own synthesizer for one board: two paid sessions, two possible
//!   `nxc reply`s, of which phase-2 recognition only ever consults the last;
//! * under PASS-THROUGH, each delivers the same answers to the requester and wakes it twice.
//!
//! **The second one is why this guard is no longer the synthesizer's own** (nxf 6j6v.e9qj). Until
//! that item the pass-through form was gated only on "the channel thread still expects the
//! supervisor", which makes a LATE second completion a clean no-op — good — but leaves two
//! SIMULTANEOUS ones both consolidating. Once the two forms are one mechanism with a declared
//! difference, one of them being unprotected is not a defensible asymmetry, so both take this claim
//! at the same point, keyed on their own retarget marker (`__synth__` / `__delivered__`).
//!
//! The TABLE is still called `synthesis_claims`: it is device-local and not op-folded, so renaming
//! it would be a schema migration bought for a word. The rows mean what they always meant — "this
//! device has begun consolidating this round".
//!
//! The guard is a **single-statement compare-and-swap**, so the database decides the winner rather
//! than whichever process happens to write first. One statement is what makes it atomic: SQLite runs
//! an `INSERT … SELECT` inside one implicit transaction, so no second connection — no second
//! PROCESS, which is the real case here (`nxc reply` and `nxc tick` are separate
//! invocations against the same file) — can interleave between the read and the insert.
//!
//! **What the claim is keyed on, and why it is not just the thread id.** A thread can legitimately
//! need a SECOND synthesis: the opener re-declares `expects` back to real members
//! (`facade::set_expects`, the seam write that opens the next turn — its CLI verb went with
//! 6j6v.dvyq) and they reply again. A claim keyed on the thread alone would
//! read that as a duplicate and spawn nothing at all — trading a double spawn for a silent
//! never-spawn, which is strictly worse. So the key carries the LWW version of the very register the
//! retarget overwrites, `(expects_reply_from_v, expects_reply_from_site)`: the pair the racers both
//! observed. Any re-declare emits a new op with a higher Lamport, so the next round has a fresh key
//! and claims cleanly, while two racers inside ONE round collide on the same key by construction.
//!
//! **Scope: one device** (PR #271 review, Integrity & Robustness #1 — stated here rather than left
//! for a reader to infer). The table is plain and device-local, NOT op-folded, for the same reason
//! `session_map` and `liveness` are: it records that THIS machine started a session. What that buys
//! is the race the timer and a reply actually run into — both live on the machine holding the board's
//! `at` job. What it does NOT close is the same collision across two SYNCED replicas: each device's
//! claims table starts empty, so two devices that both observe the completion can each spawn.
//!
//! That gap is not closable by simply folding the claim into the op log. The log is an
//! eventually-consistent CRDT: two replicas would each write a claim, and the merge would pick a
//! winner AFTER both synthesizers had already started — a decision that arrives too late to prevent
//! anything. Deciding a single winner up front across devices needs synchronous agreement, which an
//! offline-first substrate deliberately does not offer; the honest options are a server-side lease or
//! making a duplicate synthesis harmless (idempotent delivery). Tracked as nxf 6j6v.2d5r.

use rusqlite::{params, OptionalExtension};

use crate::error::Result;
use crate::store::ChatStore;

impl ChatStore {
    /// Claim the right to CONSOLIDATE `thread_id`'s current completion round. `true` = this caller
    /// won and must go on; `false` = someone else already did, and this caller must not.
    ///
    /// Fails the claim in both losing shapes, in one statement:
    /// - the thread's `expects_reply_from` is ALREADY this form's marker — a prior caller completed
    ///   the whole retarget, so this is a stale duplicate;
    /// - a row for this thread's current expects version already exists — a racer claimed the same
    ///   round between this caller's read of the quorum and now.
    ///
    /// `marker_qualified` is the QUALIFIED identity the winner will retarget the thread to — the
    /// consolidator's own output form decides which (`<origin>/__synth__` for a fold,
    /// `<origin>/__delivered__` for a pass-through) — and it is the same value `facade::set_expects`
    /// will write, encoded here exactly as that path encodes it (a one-element JSON array), because
    /// the comparison happens against the stored register text.
    ///
    /// An unknown `thread_id` selects no row and therefore claims nothing (`false`) — a caller
    /// holding a quorum for a thread that is not in the view has nothing to consolidate.
    /// **Which ROUND this thread is currently in**, as the LWW position of the very register a
    /// retarget overwrites — `(expects_reply_from_v, expects_reply_from_site)` (nxf 6j6v.gn8b;
    /// review of that branch).
    ///
    /// The same pair [`claim_consolidation`](Self::claim_consolidation) keys on, and for the same
    /// reason its own module doc gives: a thread can legitimately need a SECOND consolidation — the
    /// opener re-declares `expects` back to real members and they reply again — so the thread id
    /// alone does not identify a round, and anything keyed on it alone reads two rounds as one.
    ///
    /// It is read here rather than derived inside SQL because it has a second consumer now: the
    /// coordinator's held delivery. A channel round's composed answer is held for a requester that
    /// was mid-turn, and two rounds of ONE thread that both complete while that same session is
    /// still working must not collide in the queue — `INSERT OR IGNORE` would silently drop the
    /// second, which is precisely the loss the queue exists to prevent.
    ///
    /// `None` for a thread this workspace has no row for.
    pub fn expects_version(&self, thread_id: &str) -> Result<Option<(i64, i64)>> {
        Ok(self
            .connection()
            .query_row(
                "SELECT expects_reply_from_v, expects_reply_from_site FROM threads
                  WHERE thread_id = ?1",
                [thread_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn claim_consolidation(
        &mut self,
        thread_id: &str,
        marker_qualified: &str,
        now: &str,
    ) -> Result<bool> {
        let marker = serde_json::to_string(&[marker_qualified]).expect("marker serializes");
        let claimed = self.connection().execute(
            "INSERT OR IGNORE INTO synthesis_claims(thread_id, expects_v, expects_site, claimed)
             SELECT thread_id, expects_reply_from_v, expects_reply_from_site, ?3
               FROM threads
              WHERE thread_id = ?1
                AND (expects_reply_from IS NULL OR expects_reply_from <> ?2)",
            params![thread_id, marker, now],
        )?;
        Ok(claimed == 1)
    }

    /// Hand back a claim whose work never happened — the other half of
    /// [`claim_consolidation`](Self::claim_consolidation) (PR #271 review, found independently
    /// by all three reviews).
    ///
    /// Winning the claim and doing the work are two steps, and the step between them — retargeting
    /// `expects` — can fail. A claim left standing after that is indistinguishable from a
    /// consolidation that really did run: every later completion of the round is refused, and the
    /// board recovers only when a human re-declares it by hand. That is a worse failure than the
    /// double consolidation this guard exists to prevent, so a caller that claims and then cannot
    /// proceed MUST release.
    ///
    /// Releases the row for the thread's CURRENT expects version — the same key the claim used — so
    /// a late release can never re-open a round that has already moved on. A no-op when there is
    /// nothing to release.
    pub fn release_consolidation_claim(&mut self, thread_id: &str) -> Result<()> {
        self.connection().execute(
            "DELETE FROM synthesis_claims
              WHERE thread_id = ?1
                AND EXISTS (SELECT 1 FROM threads t
                             WHERE t.thread_id = synthesis_claims.thread_id
                               AND t.expects_reply_from_v = synthesis_claims.expects_v
                               AND t.expects_reply_from_site = synthesis_claims.expects_site)",
            params![thread_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::facade;
    use crate::model::{Disposition, MessageKind, Priority, Refs};
    use crate::store::ChatStore;

    const NOW: &str = "2026-08-02T00:00:00Z";
    const SYNTH: &str = "local/__synth__";
    /// The pass-through form's own retarget marker — the second identity this one claim guards.
    const DELIVERED: &str = "local/__delivered__";

    /// A thread expecting `bob`, with his reply already in — the state both racers observe.
    fn seeded_thread(store: &mut ChatStore) -> String {
        store.set_channel_field("decl:review", "kind", "group", "local/pm");
        let tid = facade::ask(
            store,
            facade::AskRequest {
                now: NOW,
                origin: "local",
                actor: "pm",
                channel: "decl:review",
                body: "please review",
                expect: &["local/bob".to_string()],
                deadline: None,
                kind: MessageKind::Question,
                priority: Priority::Normal,
                refs: Refs::default(),
            },
        )
        .expect("ask succeeds")
        .thread_id;
        facade::reply(
            store,
            facade::ReplyRequest {
                now: NOW,
                origin: "local",
                actor: "bob",
                target: &tid,
                body: "lgtm",
                kind: MessageKind::Report,
                priority: Priority::Normal,
                disposition: Disposition::InTurn,
                refs: Refs::default(),
                if_unanswered: false,
            },
        )
        .expect("reply succeeds");
        tid
    }

    #[test]
    fn exactly_one_racer_claims_a_round_and_the_next_round_re_arms() {
        let mut store = ChatStore::open_in_memory(1);
        let tid = seeded_thread(&mut store);

        // Two callers holding the same pre-retarget state: the first wins, the second loses.
        assert!(store.claim_consolidation(&tid, SYNTH, NOW).unwrap());
        assert!(
            !store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "a second claimant in the SAME round is refused"
        );

        // The winner now does what it won the right to do — retarget. A late third caller still
        // holding the stale snapshot is refused on the marker itself, not just on the key.
        facade::set_expects(&mut store, NOW, "local/pm", &tid, &[SYNTH.to_string()]).unwrap();
        assert!(
            !store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "a thread already retargeted to the synthesis marker admits no further claim"
        );

        // A genuinely NEW round: the opener re-declares real members (`facade::set_expects`, the
        // seam write that opens the next turn — its CLI verb went with 6j6v.dvyq) and they reply
        // again. This must claim — refusing it would trade the double spawn for no spawn at all.
        facade::set_expects(
            &mut store,
            NOW,
            "local/pm",
            &tid,
            &["local/bob".to_string()],
        )
        .unwrap();
        assert!(
            store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "a re-declared thread is a new round and claims cleanly"
        );
        assert!(
            !store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "…and that new round is itself single-claimant"
        );
    }

    #[test]
    fn a_released_claim_re_arms_the_same_round() {
        // PR #271 review (Code Quality #1 / Test Quality #1 / Integrity #2, all three reviews):
        // winning the claim is not the same as having done the work. The caller retargets AFTER
        // claiming, and that retarget can fail — so a won-but-unused claim has to be handed back, or
        // the round is refused forever and the thread only recovers via a manual re-declare.
        let mut store = ChatStore::open_in_memory(1);
        let tid = seeded_thread(&mut store);

        assert!(store.claim_consolidation(&tid, SYNTH, NOW).unwrap());
        store.release_consolidation_claim(&tid).unwrap();
        assert!(
            store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "a released round is claimable again — by this caller or by the racer that lost"
        );
        assert!(
            !store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "…and re-claiming it does not weaken the single-claimant rule"
        );
    }

    #[test]
    fn releasing_frees_only_the_round_that_is_current() {
        // The release must name the SAME key the claim did, or a caller that fails late could hand
        // back a round that has since moved on and let a second synthesizer in for it.
        let mut store = ChatStore::open_in_memory(1);
        let tid = seeded_thread(&mut store);
        assert!(store.claim_consolidation(&tid, SYNTH, NOW).unwrap());

        // The round moves on (the winner retargeted, then the opener re-declared real members).
        facade::set_expects(&mut store, NOW, "local/pm", &tid, &[SYNTH.to_string()]).unwrap();
        facade::set_expects(
            &mut store,
            NOW,
            "local/pm",
            &tid,
            &["local/bob".to_string()],
        )
        .unwrap();
        assert!(
            store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "the new round claims cleanly"
        );

        store.release_consolidation_claim(&tid).unwrap();
        assert!(
            store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "the release freed the CURRENT round"
        );
        assert!(
            !store.claim_consolidation(&tid, SYNTH, NOW).unwrap(),
            "and that is still the only claim it admits"
        );
    }

    #[test]
    fn the_claim_is_decided_by_the_database_not_by_a_process_local_memory() {
        // The real race is between two PROCESSES (`nxc reply` and `nxc tick`), so the guard
        // has to hold across connections. Two stores over the same file, each with its own
        // connection and its own site id, both holding the same pre-retarget state.
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite");
        let path = path.to_str().unwrap();
        let tid = {
            let mut seeder = ChatStore::open(path, 1).unwrap();
            seeded_thread(&mut seeder)
        };
        let mut a = ChatStore::open(path, 1).unwrap();
        let mut b = ChatStore::open(path, 2).unwrap();

        let a_won = a.claim_consolidation(&tid, SYNTH, NOW).unwrap();
        let b_won = b.claim_consolidation(&tid, SYNTH, NOW).unwrap();
        assert!(a_won, "the first connection to claim wins");
        assert!(
            !b_won,
            "the second connection is refused by the table, not by anything a in-memory"
        );
    }

    #[test]
    fn only_one_of_eight_genuinely_concurrent_racers_wins() {
        // Driven for BOTH output forms' markers (nxf 6j6v.e9qj): the claim is what makes a fold and
        // a pass-through equally single-claimant, so proving it for one marker only would leave the
        // half that was actually unprotected untested.
        for marker in [SYNTH, DELIVERED] {
            only_one_of_eight_concurrent_racers_wins_for(marker);
        }
    }

    fn only_one_of_eight_concurrent_racers_wins_for(marker: &str) {
        // PR #271 review, Test Quality #3: the two-connection test above proves the DATABASE decides,
        // but issues its claims one after the other from a single thread — it never actually lets two
        // writers collide. This one does: eight threads, eight connections, all released at once on a
        // barrier, every one of them holding the same pre-retarget state.
        //
        // The assertion is scheduling-INDEPENDENT, which is what keeps a concurrency test out of the
        // flaky column: whatever order the threads reach the statement in, the count of winners is
        // exactly one. (`Store::open` sets `PRAGMA busy_timeout=5000`, so a writer that meets a
        // locked db waits rather than erroring — the collision resolves, it does not fail.)
        use std::sync::{Arc, Barrier};

        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite").to_str().unwrap().to_string();
        let tid = {
            let mut seeder = ChatStore::open(&path, 1).unwrap();
            seeded_thread(&mut seeder)
        };

        const RACERS: usize = 8;
        let barrier = Arc::new(Barrier::new(RACERS));
        let handles: Vec<_> = (0..RACERS)
            .map(|i| {
                let (path, tid, barrier, marker) = (
                    path.clone(),
                    tid.clone(),
                    Arc::clone(&barrier),
                    marker.to_string(),
                );
                std::thread::spawn(move || {
                    let mut store = ChatStore::open(&path, i as i64 + 1).unwrap();
                    barrier.wait();
                    store.claim_consolidation(&tid, &marker, NOW).unwrap()
                })
            })
            .collect();
        let won = handles
            .into_iter()
            .map(|h| h.join().expect("no racer panics"))
            .filter(|w| *w)
            .count();

        assert_eq!(
            won, 1,
            "exactly one of {RACERS} concurrent racers may consolidate one board ({marker})"
        );
    }

    #[test]
    fn an_unknown_thread_claims_nothing() {
        let mut store = ChatStore::open_in_memory(1);
        assert!(!store.claim_consolidation("t-nope", SYNTH, NOW).unwrap());
    }
}
