//! Watching a conversation happen (`--stream`, nxf 6j6v.p6m1, surface draft §4.5).
//!
//! `send --to` and `reply --thread` acknowledge the moment the message is durably logged; the work
//! happens afterwards, elsewhere. `--stream` turns that into a wait: it follows the conversation
//! until the other side answers, and prints what it sees on the way.
//!
//! # Two streams, kept apart
//!
//! What a persona **said** is the thread; what it **did and thought** is the transcript. Both go to
//! the reader — without the transcript a hard task shows nothing at all for minutes, because only
//! the distilled result reaches the thread — but they are labelled differently, because for a human
//! who later quotes one of them the difference is the whole point.
//!
//! # A deadline that reacts to life
//!
//! A wait needs a bound, but a straight one breaks the case it exists for: an agent starts a shell
//! command that runs for an hour, the shell is perfectly healthy, and nothing appears in the
//! transcript. A flat thirty-minute timer would kill something that is working.
//!
//! So the clock is reset by ANY activity — a message or a transcript entry — and before it strikes
//! there is a **knock**: a report of what the transcript last showed, so a human decides on evidence
//! instead of a timeout. See [`IdleClock`] for the state machine and [`Knock`] for what the report
//! can and cannot say in this slice.
//!
//! # Human-only, and honestly so
//!
//! `--stream` is refused once a persona is registered (the CLI gates it on
//! [`crate::persona::Identity`]). That is **hygiene, not a boundary**: a process can simply not
//! register and be "a human", exactly as it can drop any other self-reported identity. It protects
//! against the accidental use — an agent that blocks for half an hour waiting on itself — which is
//! what it is for. Claiming more would be claiming a guarantee this layer cannot make.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::error::Result;
use crate::model::Refs;
use crate::store::ChatStore;

/// How long a follower waits, and how often it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamOptions {
    /// How long with NO activity at all before the wait ends. Reset by every event.
    pub idle: Duration,
    /// How far ahead of that the knock happens, so the reader hears something before the wait ends
    /// rather than at the same moment.
    pub knock_lead: Duration,
    /// How often the store is re-read.
    pub poll: Duration,
}

impl Default for StreamOptions {
    /// Thirty minutes of silence, knocking five minutes before — the draft's own reference numbers.
    /// The idle window is generous on purpose: it is measured from the last sign of life, not from
    /// the start, so a working agent never approaches it.
    fn default() -> Self {
        StreamOptions {
            idle: Duration::from_secs(30 * 60),
            knock_lead: Duration::from_secs(5 * 60),
            poll: Duration::from_secs(1),
        }
    }
}

/// What the follower should do next, given how long it has been quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Nothing is wrong — keep waiting.
    Wait,
    /// The wait is nearly over and nothing has happened: report what the transcript shows.
    Knock,
    /// Silent past the whole window. Stop waiting.
    GiveUp,
}

/// The deadline that reacts to life (see the module docs).
///
/// Kept as a pure state machine over "how long since the last event" so the behaviour is testable
/// without sleeping: the follower owns the clock, this owns the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleClock {
    idle: Duration,
    knock_lead: Duration,
    knocked: bool,
}

impl IdleClock {
    pub fn new(opts: &StreamOptions) -> IdleClock {
        IdleClock {
            idle: opts.idle,
            knock_lead: opts.knock_lead,
            knocked: false,
        }
    }

    /// Something happened: the window starts over, and so does the right to knock again.
    pub fn saw_activity(&mut self) {
        self.knocked = false;
    }

    /// What to do, `since` the last event. The knock fires at most once per quiet stretch — a
    /// reader who was told what the transcript shows does not need to be told once a second.
    pub fn decide(&mut self, since: Duration) -> Decision {
        if since >= self.idle {
            return Decision::GiveUp;
        }
        if !self.knocked && since + self.knock_lead >= self.idle {
            self.knocked = true;
            return Decision::Knock;
        }
        Decision::Wait
    }
}

/// One thing the follower saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A message in the thread — what somebody SAID.
    Said {
        message_id: String,
        sender: String,
        body: String,
        /// Whether this came from somebody other than the follower — i.e. the answer being waited
        /// for.
        foreign: bool,
    },
    /// A transcript entry — what a persona DID or THOUGHT.
    Did {
        session: String,
        seq: i64,
        kind: String,
        /// A one-line rendering of the entry's opaque payload; empty when it has nothing to show.
        detail: String,
    },
}

/// What the transcript last showed when the deadline came within reach — the knock.
///
/// **What it does NOT do in this slice:** the draft describes a fresh junior persona reading the
/// transcript and reporting in its own words ("started a shell, cannot tell what it is doing").
/// That is a spawn and a paid model call triggered by a command-line flag, needing a persona that
/// may not be declared — so this slice reports the same evidence deterministically instead, and the
/// judged version is filed as its own follow-up. The reader gets facts either way; only the
/// judgement is missing, and a wrong judgement would be worse than none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Knock {
    /// The sessions being followed, so a human can go look at their logs.
    pub sessions: Vec<String>,
    /// The last transcript entry seen, if any: `kind` plus its one-line detail.
    pub last: Option<(String, String)>,
    /// How long it has been quiet.
    pub quiet_for: Duration,
}

impl Knock {
    /// The knock as a human line, ending in where to look next.
    pub fn render(&self) -> String {
        let quiet = format!("{}m", self.quiet_for.as_secs() / 60);
        let seen = match &self.last {
            Some((kind, detail)) if !detail.is_empty() => format!("last: {kind} — {detail}"),
            Some((kind, _)) => format!("last: {kind}"),
            None => "nothing in the transcript at all".to_string(),
        };
        let where_to_look = if self.sessions.is_empty() {
            "no session is being followed".to_string()
        } else {
            format!(
                "logs: {}",
                self.sessions
                    .iter()
                    .map(|s| format!(".nxs/agent-logs/{s}.log"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!("quiet for {quiet} — {seen}; {where_to_look}")
    }
}

/// A live follow of one thread and the sessions taking part in it.
///
/// Built with [`Follow::start`], which reads once and remembers the thread as far as the follower's
/// OWN last message, so the first [`poll`](Follow::poll) reports what happened after that — never
/// the caller's own message back at it, and never silently swallowing an answer that beat the
/// follow to the store.
#[derive(Debug, Clone)]
pub struct Follow {
    thread: String,
    /// The follower's own qualified handle — what makes an event "somebody else's".
    me: String,
    seen_messages: BTreeSet<String>,
    /// session → the last transcript `seq` already reported.
    seen_seq: BTreeMap<String, i64>,
    answered: bool,
}

impl Follow {
    /// Begin following `thread` as `me`, watching `seed` (the session this send just started, when
    /// it started one) plus every session that turns up in the thread as it goes.
    ///
    /// **History ends at the follower's own last message, not at "everything that is there now"**
    /// (PR #295 review, Code Quality #2). The follow starts just after a write, so the caller's
    /// message is normally the newest one and the two readings coincide — but between the write and
    /// this read the other side can already have answered, and marking that answer as history would
    /// make the follow wait out the whole idle window for a reply sitting in the store. Anything
    /// after the follower's own last message is by definition part of what it is waiting for, so it
    /// is left unseen and the first [`poll`](Follow::poll) reports it.
    ///
    /// A follower with no message in the thread at all — nothing this crate does, since both verbs
    /// write first — has no such boundary, and everything present reads as history.
    pub fn start(store: &ChatStore, thread: &str, me: &str, seed: Option<&str>) -> Result<Follow> {
        let mut follow = Follow {
            thread: thread.to_string(),
            me: me.to_string(),
            seen_messages: BTreeSet::new(),
            seen_seq: BTreeMap::new(),
            answered: false,
        };
        if let Some(session) = seed {
            follow.watch(store, session)?;
        }
        let rows = store.messages_in_thread(thread)?;
        let mine = rows.iter().rposition(|r| r.sender == me);
        for (i, row) in rows.into_iter().enumerate() {
            // Watched regardless: a session already in the thread is one whose transcript should
            // stream from HERE, whether or not its message counts as history.
            if let Some(session) = session_of(&row.refs) {
                follow.watch(store, &session)?;
            }
            if mine.is_none_or(|last| i <= last) {
                follow.seen_messages.insert(row.message_id);
            }
        }
        Ok(follow)
    }

    /// Start watching a session's transcript from where it stands right now.
    fn watch(&mut self, store: &ChatStore, session: &str) -> Result<()> {
        if self.seen_seq.contains_key(session) {
            return Ok(());
        }
        let last = store
            .transcript_rows(session)?
            .last()
            .map(|r| r.seq)
            .unwrap_or(-1);
        self.seen_seq.insert(session.to_string(), last);
        Ok(())
    }

    /// Everything that happened since the last call, thread first and then each session's
    /// transcript — a deterministic order, so two readers of the same run see the same sequence.
    pub fn poll(&mut self, store: &ChatStore) -> Result<Vec<Event>> {
        let mut events = Vec::new();
        for row in store.messages_in_thread(&self.thread)? {
            if let Some(session) = session_of(&row.refs) {
                self.watch(store, &session)?;
            }
            if !self.seen_messages.insert(row.message_id.clone()) {
                continue;
            }
            let foreign = row.sender != self.me;
            self.answered |= foreign;
            events.push(Event::Said {
                message_id: row.message_id,
                sender: row.sender,
                body: row.body,
                foreign,
            });
        }
        // Collected first so the borrow of `seen_seq` ends before it is written back.
        let sessions: Vec<(String, i64)> = self
            .seen_seq
            .iter()
            .map(|(s, seq)| (s.clone(), *seq))
            .collect();
        for (session, after) in sessions {
            for row in store.transcript_rows_after(&session, after)? {
                self.seen_seq.insert(session.clone(), row.seq);
                events.push(Event::Did {
                    session: session.clone(),
                    seq: row.seq,
                    kind: row.entry.kind.clone(),
                    detail: summarize(&row.entry.data),
                });
            }
        }
        Ok(events)
    }

    /// Whether somebody else has posted into the thread since the follow began — the answer the
    /// wait exists for.
    pub fn answered(&self) -> bool {
        self.answered
    }

    /// The sessions currently being followed, for the knock.
    pub fn sessions(&self) -> Vec<String> {
        self.seen_seq.keys().cloned().collect()
    }
}

/// Whether the conversation `follow` is watching has come to rest.
///
/// A quorum board settles when its quorum completes — waiting for the FIRST of several expected
/// replies would stop a fan-out halfway and show the reader one opinion out of three. Everything
/// else — a direct conversation with one persona, a plain channel — settles when somebody else
/// posts, which is the answer that was waited for.
///
/// Which of the two a thread IS is TWO conditions, ANDed — both halves of the owner ruling on the
/// nxf 6j6v.jepk review (2026-08-12, round 2; the same fix [`crate::orchestration::reply`]'s
/// `is_quorum_thread` needed, for the same reason). It is a board only when it does NOT live in a
/// DM channel ([`crate::orchestration::is_dm_channel`]) AND it actually carries a non-empty
/// `expects_reply_from`. Neither half alone is enough: `expects_reply_from` alone made a `send --to
/// <persona>` DM (which now always declares one, nxf 6j6v.jepk) misread as a board, waiting for the
/// exact expected handle instead of settling on the first foreign message as the doc above promises
/// (round 1's regression) — and the CHANNEL alone, tried in round 1's fix, made a perfectly
/// ordinary non-DM thread with an EMPTY `expects_reply_from` (e.g. a plain `send --to
/// <substrate-channel>`) misread as a board too, where `quorum.complete` is permanently `false` for
/// an empty `expects` (`store.rs`'s `complete = !expects.is_empty() && …`), so `settled` could
/// never become `true` and `--stream` would hang to the idle give-up instead of returning the
/// moment an answer arrived (round 2's regression). `thread_channel` (M2 first-class threads) falls
/// back to `thread_channel_via_message` (M1: a thread only ever implied by a message stamped with
/// it — `send --thread`, until 6j6v.dvyq §3 removed that flag in favour of `send --to` minting the
/// thread and `reply --thread` posting into it) for the same reason `reply` and
/// [`crate::facade::thread_board`] both do: `expects_reply_from` can be set on a thread before it is
/// ever `open_thread`-ed.
pub fn settled(store: &ChatStore, thread: &str, follow: &Follow, now: &str) -> Result<bool> {
    let lives_in_dm = store
        .thread_channel(thread)
        .or_else(|| store.thread_channel_via_message(thread))
        .is_some_and(|c| crate::orchestration::is_dm_channel(&c));
    if !lives_in_dm {
        if let Some(quorum) = store.thread_quorum(thread, now)? {
            if !quorum.expects.is_empty() {
                return Ok(quorum.complete);
            }
        }
    }
    Ok(follow.answered())
}

/// The session a message carries as its return address, if any and if its refs are readable.
fn session_of(refs: &Option<String>) -> Option<String> {
    serde_json::from_str::<Refs>(refs.as_deref()?)
        .ok()?
        .session_id
}

/// One line out of a transcript entry's OPAQUE payload.
///
/// The payload's shape belongs to the sidecar, not to this crate (see [`crate::transcript`]), so
/// this peeks at the few keys the shipped one uses and says nothing rather than guessing when it
/// finds none of them. A newer sidecar with a new payload still streams — as its `kind`, with no
/// detail — which is the same "store what you do not recognise" discipline one layer down.
fn summarize(data: &serde_json::Value) -> String {
    for key in ["text", "message", "name"] {
        if let Some(value) = data.get(key).and_then(|v| v.as_str()) {
            return first_line(value);
        }
    }
    String::new()
}

/// The first line, clipped — a transcript entry can be a whole file, and this is a progress line.
fn first_line(text: &str) -> String {
    const MAX: usize = 160;
    let line = text.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Disposition, MessageEnvelope, MessageKind, Priority};
    use crate::transcript::TranscriptEntry;

    fn opts() -> StreamOptions {
        StreamOptions {
            idle: Duration::from_secs(600),
            knock_lead: Duration::from_secs(60),
            poll: Duration::from_millis(10),
        }
    }

    fn post(store: &mut ChatStore, sender: &str, body: &str, session: Option<&str>) -> String {
        store.set_channel_field("c-1", "kind", "group", "local/human");
        store.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-1".into(),
            sender: sender.into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-1".into()),
            refs: Refs {
                session_id: session.map(str::to_string),
                ..Refs::default()
            },
            body: body.into(),
        })
    }

    fn entry(kind: &str, data: serde_json::Value) -> TranscriptEntry {
        TranscriptEntry {
            kind: kind.into(),
            at: None,
            tool_use_id: None,
            parent_tool_use_id: None,
            subagent_type: None,
            data,
        }
    }

    // ---- the deadline ------------------------------------------------------------------------

    #[test]
    fn the_clock_knocks_once_before_it_gives_up() {
        let mut clock = IdleClock::new(&opts());
        assert_eq!(clock.decide(Duration::from_secs(0)), Decision::Wait);
        assert_eq!(clock.decide(Duration::from_secs(500)), Decision::Wait);
        assert_eq!(clock.decide(Duration::from_secs(545)), Decision::Knock);
        assert_eq!(
            clock.decide(Duration::from_secs(550)),
            Decision::Wait,
            "knocking once per quiet stretch — not once per poll"
        );
        assert_eq!(clock.decide(Duration::from_secs(600)), Decision::GiveUp);
    }

    #[test]
    fn activity_resets_the_window_and_re_arms_the_knock() {
        // The whole reason the clock exists: an agent that is working must never be given up on,
        // and a stretch of silence after work is a NEW stretch, worth its own knock.
        let mut clock = IdleClock::new(&opts());
        assert_eq!(clock.decide(Duration::from_secs(545)), Decision::Knock);
        clock.saw_activity();
        assert_eq!(clock.decide(Duration::from_secs(0)), Decision::Wait);
        assert_eq!(clock.decide(Duration::from_secs(545)), Decision::Knock);
    }

    // ---- the follow --------------------------------------------------------------------------

    #[test]
    fn a_follow_reports_only_what_happens_after_it_starts() {
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "local/human", "do the thing", None);
        let mut follow = Follow::start(&store, "t-1", "local/human", None).unwrap();
        assert!(
            follow.poll(&store).unwrap().is_empty(),
            "the caller's own message is not news to the caller"
        );

        post(&mut store, "local/pm", "on it", Some("s-pm"));
        let events = follow.poll(&store).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::Said {
                sender, foreign, ..
            } => {
                assert_eq!(sender, "local/pm");
                assert!(foreign, "somebody else answered");
            }
            other => panic!("expected a thread message, got {other:?}"),
        }
        assert!(follow.answered());
    }

    #[test]
    fn an_answer_that_beat_the_follow_to_the_store_is_still_reported() {
        // The narrow race between "the message is written" and "the follow reads the thread": the
        // answer is already there when the follow starts. Treating it as history would leave the
        // reader watching an empty stream until the idle window ran out, for a reply that is
        // sitting in the store — so anything after the follower's OWN last message is news.
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "local/human", "do the thing", None);
        post(&mut store, "local/pm", "already done", Some("s-pm"));

        let mut follow = Follow::start(&store, "t-1", "local/human", None).unwrap();
        let events = follow.poll(&store).unwrap();
        assert!(
            matches!(&events[0], Event::Said { sender, body, foreign, .. }
                if sender == "local/pm" && body == "already done" && *foreign),
            "the answer that landed first is reported, once: {events:?}"
        );
        assert_eq!(
            events.len(),
            1,
            "and the caller's own message is not: {events:?}"
        );
        assert!(follow.answered(), "so the wait ends instead of timing out");
    }

    #[test]
    fn an_earlier_turn_of_the_same_conversation_is_history_not_an_answer() {
        // The other half of the boundary: `reply --thread` follows a thread that ALREADY contains
        // the counterpart's turns. Those are behind the follower's own newest message, so they are
        // history — a follow that reported them would settle instantly on an old answer and print
        // the conversation back at the reader.
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "local/human", "do the thing", None);
        post(&mut store, "local/pm", "here it is", Some("s-pm"));
        post(&mut store, "local/human", "and now this", None);

        let mut follow = Follow::start(&store, "t-1", "local/human", None).unwrap();
        assert!(
            follow.poll(&store).unwrap().is_empty(),
            "all of it is history"
        );
        assert!(!follow.answered(), "the new turn has not been answered yet");

        post(&mut store, "local/pm", "on it", Some("s-pm"));
        assert_eq!(follow.poll(&store).unwrap().len(), 1);
        assert!(follow.answered());
    }

    #[test]
    fn the_transcript_streams_beside_the_thread_and_is_labelled_apart() {
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "local/human", "do it", None);
        store.create_pending_session("s-pm", "pm").unwrap();
        let mut follow = Follow::start(&store, "t-1", "local/human", Some("s-pm")).unwrap();

        store
            .append_transcript(
                "s-pm",
                &[
                    entry(
                        "thinking",
                        serde_json::json!({"text": "let me look\nsecond line"}),
                    ),
                    entry("tool_use", serde_json::json!({"name": "Bash", "input": {}})),
                ],
            )
            .unwrap();
        let events = follow.poll(&store).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            Event::Did {
                session: "s-pm".into(),
                seq: 0,
                kind: "thinking".into(),
                detail: "let me look".into(),
            },
            "the detail is one line, not the whole payload"
        );
        assert!(
            matches!(&events[1], Event::Did { kind, detail, .. } if kind == "tool_use" && detail == "Bash")
        );
        assert!(!follow.answered(), "doing is not answering");

        assert!(
            follow.poll(&store).unwrap().is_empty(),
            "an entry is reported exactly once"
        );
    }

    #[test]
    fn a_session_that_appears_in_the_thread_is_picked_up_without_being_seeded() {
        // The human seeds the persona it summoned; anyone the conversation later involves has to
        // be discovered, or their work is invisible.
        let mut store = ChatStore::open_in_memory(1);
        post(&mut store, "local/human", "do it", None);
        let mut follow = Follow::start(&store, "t-1", "local/human", None).unwrap();
        assert!(follow.sessions().is_empty());

        post(&mut store, "local/pm", "here", Some("s-pm"));
        follow.poll(&store).unwrap();
        assert_eq!(follow.sessions(), ["s-pm"]);

        store
            .append_transcript(
                "s-pm",
                &[entry("assistant", serde_json::json!({"text": "hi"}))],
            )
            .unwrap();
        let events = follow.poll(&store).unwrap();
        assert!(matches!(&events[0], Event::Did { session, .. } if session == "s-pm"));
    }

    #[test]
    fn a_direct_conversation_settles_on_the_first_answer_and_a_board_on_its_quorum() {
        let mut store = ChatStore::open_in_memory(1);
        let now = "2026-08-05T10:00:00Z";
        post(&mut store, "local/human", "do it", None);
        let mut follow = Follow::start(&store, "t-1", "local/human", None).unwrap();
        assert!(!settled(&store, "t-1", &follow, now).unwrap());
        post(&mut store, "local/pm", "done", Some("s-pm"));
        follow.poll(&store).unwrap();
        assert!(settled(&store, "t-1", &follow, now).unwrap());

        // The same thread, now a board expecting two: one reply must NOT end the watch, or a
        // fan-out shows the reader one opinion out of three and calls it the answer. Both answers
        // come AFTER the declaration — since 6j6v.cg8g the board is discharged per TURN, so pm's
        // earlier "done" answered the direct conversation above and not this new board (which is
        // also the stronger shape for this test's own claim: it now watches a two-handle board go
        // from nothing, through one of two, to answered).
        store.set_expects_reply_from("t-1", r#"["local/pm","local/coder"]"#, "local/human");
        assert!(!settled(&store, "t-1", &follow, now).unwrap());
        post(
            &mut store,
            "local/pm",
            "done, and here are the details",
            Some("s-pm"),
        );
        follow.poll(&store).unwrap();
        assert!(
            !settled(&store, "t-1", &follow, now).unwrap(),
            "one of two is not the answer"
        );
        post(&mut store, "local/coder", "also done", Some("s-coder"));
        follow.poll(&store).unwrap();
        assert!(settled(&store, "t-1", &follow, now).unwrap());
    }

    #[test]
    fn a_persona_dm_still_settles_on_the_first_foreign_message_once_it_expects_a_reply() {
        // Important regression (review of nxf 6j6v.jepk, owner ruling 2026-08-12): once `send_to`
        // started declaring `expects_reply_from` on a persona's DM thread, `settled`'s OLD
        // `!quorum.expects.is_empty()` gate started reading every persona DM as a quorum board too,
        // so a `--stream` follow would wait for the EXACT expected handle instead of settling the
        // moment somebody else posts — exactly the case this function's own doc says must settle on
        // "somebody else posts". Keying on the channel (a DM is never a quorum board) fixes it.
        let mut store = ChatStore::open_in_memory(1);
        let now = "2026-08-05T10:00:00Z";
        store.set_channel_field("dm:human~coder", "kind", "direct", "local/human");
        store.open_thread(
            "t-dm",
            &crate::model::ThreadRoot {
                origin: "local".into(),
                channel_id: "dm:human~coder".into(),
                opener: "local/human".into(),
                created: now.into(),
                parent: None,
            },
            "local/human",
        );
        store.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "dm:human~coder".into(),
            sender: "local/human".into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-dm".into()),
            refs: Refs::default(),
            body: "please help".into(),
        });
        // The trigger declares its own expectation (nxf 6j6v.jepk) — the thread now expects coder,
        // and never stops, exactly like a real `send --to` thread.
        store.set_expects_reply_from("t-dm", r#"["local/coder"]"#, "local/human");

        let mut follow = Follow::start(&store, "t-dm", "local/human", None).unwrap();
        assert!(!settled(&store, "t-dm", &follow, now).unwrap());

        // Somebody OTHER than the expected persona posts (a third party butting into the DM, or —
        // realistically — the persona replying under a handle this test deliberately does not
        // match, so the two mechanisms provably diverge). `follow.answered()` is satisfied by any
        // foreign post; the old `quorum.complete` reading would stay false forever since this
        // sender is not in `expects`.
        store.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "dm:human~coder".into(),
            sender: "local/somebody-else".into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-dm".into()),
            refs: Refs::default(),
            body: "butting in".into(),
        });
        follow.poll(&store).unwrap();

        assert!(
            settled(&store, "t-dm", &follow, now).unwrap(),
            "a DM settles on the first foreign message regardless of expects_reply_from"
        );
    }

    #[test]
    fn a_non_dm_thread_with_no_expects_still_settles_on_the_first_foreign_message() {
        // Round-2 regression (review of nxf 6j6v.jepk, owner ruling 2026-08-12): round 1's fix
        // keyed `is_quorum_thread` on the channel ALONE (`!is_dm_channel`), which correctly kept
        // DMs out of the quorum branch but, on its own, ALSO pulled every non-DM thread with an
        // EMPTY `expects_reply_from` INTO it — a completely ordinary shape, e.g. a plain
        // post into a substrate channel that opens a thread and declares nothing (until 6j6v.dvyq
        // §3 that was `send --to <substrate-channel>`, `surface.rs`'s `TargetKind::Conversation`;
        // the shape itself outlives that entrance — the channel supervisor and the DM path both
        // open threads whose `expects_reply_from` is empty at the moment they are opened). `quorum.complete` is permanently `false` for an empty
        // `expects` (`!expects.is_empty() && …`), so `settled` could never become `true` there and
        // `--stream` would hang to the idle give-up instead of returning when the answer arrives.
        // The owner's ruling ANDs both halves: a board is non-DM AND has a non-empty `expects`.
        let mut store = ChatStore::open_in_memory(1);
        let now = "2026-08-05T10:00:00Z";
        store.set_channel_field("c-plain", "kind", "group", "local/human");
        store.open_thread(
            "t-plain",
            &crate::model::ThreadRoot {
                origin: "local".into(),
                channel_id: "c-plain".into(),
                opener: "local/human".into(),
                created: now.into(),
                parent: None,
            },
            "local/human",
        );
        store.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-plain".into(),
            sender: "local/human".into(),
            kind: MessageKind::Info,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-plain".into()),
            refs: Refs::default(),
            body: "anyone there?".into(),
        });
        // Deliberately NO `set_expects_reply_from` — nothing ever declared a board on this thread.

        let mut follow = Follow::start(&store, "t-plain", "local/human", None).unwrap();
        assert!(!settled(&store, "t-plain", &follow, now).unwrap());

        store.post_message(&MessageEnvelope {
            origin: "local".into(),
            channel_id: "c-plain".into(),
            sender: "local/anyone".into(),
            kind: MessageKind::Report,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some("t-plain".into()),
            refs: Refs::default(),
            body: "yes".into(),
        });
        follow.poll(&store).unwrap();

        assert!(
            settled(&store, "t-plain", &follow, now).unwrap(),
            "a non-DM thread with no expects_reply_from settles on the first foreign post, exactly \
             like a plain channel always has"
        );
    }

    #[test]
    fn an_unknown_payload_still_streams_as_its_kind() {
        assert_eq!(summarize(&serde_json::json!({"future": "shape"})), "");
        assert_eq!(summarize(&serde_json::json!({"text": "a"})), "a");
        assert_eq!(summarize(&serde_json::json!({"message": "boom"})), "boom");
    }

    #[test]
    fn a_long_transcript_line_is_clipped_rather_than_dumped() {
        let long = "x".repeat(500);
        let clipped = first_line(&long);
        assert!(clipped.len() < long.len());
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn the_knock_says_what_was_last_seen_and_where_to_look() {
        let knock = Knock {
            sessions: vec!["s-pm".into()],
            last: Some(("tool_use".into(), "Bash".into())),
            quiet_for: Duration::from_secs(25 * 60),
        };
        let line = knock.render();
        assert!(line.contains("quiet for 25m"));
        assert!(line.contains("tool_use — Bash"));
        assert!(
            line.contains(".nxs/agent-logs/s-pm.log"),
            "a human must be told where to look: {line}"
        );

        let nothing = Knock {
            sessions: vec![],
            last: None,
            quiet_for: Duration::from_secs(60),
        };
        assert!(nothing.render().contains("nothing in the transcript"));
    }
}
