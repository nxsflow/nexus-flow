//! **A planned service restart secures every running session of the instance first** (nxf
//! 6j6v.wxz0, section B — the field case 6j6v.8n9a).
//!
//! `nxs self-update` and `nxs sync daemon install` restart the background service with launchd's
//! `bootout` + `bootstrap`. The bootout SIGTERMs the service's whole process group and escalates to
//! SIGKILL, and every agent session the service started is in that group — the sidecar is spawned
//! without `process_group(0)`. Before this, a restart killed those sessions mid-turn with nothing on
//! record: the thread read "nothing is coming", `nxc resume` found nothing on hold, and the
//! uncommitted work sat in the tree with no way back but a person.
//!
//! [`secure_the_instance`] is what runs before the bootout: each live session of the instance is put
//! on a [`SERVICE_RESTART_LIMIT`] hold and sent a stop (the sidecar aborts its turn through the
//! SDK's own door, binds, flushes and announces its end), and the hook waits a grace for them to
//! go. What is still alive at the grace is killed by the bootout itself — the group kill IS the
//! kill. The new service takes each hold up at its `until`, in place.
//!
//! **The shared process group is relied on twice**: it is what lets the bootout kill a straggler,
//! and it is what says which sessions are this instance's ([`crate::worker::live_sessions_in_group`]).
//!
//! [`SERVICE_RESTART_LIMIT`]: crate::interruption::SERVICE_RESTART_LIMIT

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use nxs_service::restart::{PreparedSession, RestartPreparation, SessionTrouble};

use crate::worker::Worker;

use crate::timer::TimerConfig;

/// Everything one preparation needs. The clock, the sleep and the poll hook are injected so a test
/// controls time and can start a session between two polls; [`secure_the_instance`]'s caller in
/// the binary passes the real ones.
pub struct SecureTheInstance<'a> {
    /// The roots of the workspaces the restarted service attends.
    pub workspaces: &'a [PathBuf],
    /// The restarted service's process group.
    pub pgid: i32,
    /// The hold's detail — the restart's cause, in words.
    pub detail: &'a str,
    /// How long to wait for the instance's sessions to go
    /// ([`crate::interruption::SERVICE_RESTART_GRACE_SECS`] in the binary).
    pub grace_secs: i64,
    /// What a hold's `until` adds to the grace
    /// ([`crate::interruption::SERVICE_RESTART_MARGIN_SECS`] in the binary).
    pub margin_secs: i64,
    /// How long to wait between two looks ([`crate::interruption::SERVICE_RESTART_POLL_SECS`]).
    pub poll: Duration,
    /// Which timer arms each hold's resume — the service's, so the NEW service fires it.
    pub timer: TimerConfig,
    /// Now, in UTC.
    pub clock: &'a dyn Fn() -> time::OffsetDateTime,
    /// Wait `poll`.
    pub sleep: &'a dyn Fn(Duration),
    /// Called at the start of every look with its number, from 1.
    pub on_poll: &'a dyn Fn(u32),
}

/// **Hold, stop and wait out the instance's sessions** — see the module doc. Never fails: what it
/// could not do is in the report, and the restart proceeds either way.
///
/// Once a poll, until no session of the instance is live or the grace has run out: every session of
/// the instance not seen before is put on hold through the Engine seam
/// ([`crate::engine::Engine::session_interrupted`], limit
/// [`SERVICE_RESTART_LIMIT`](crate::interruption::SERVICE_RESTART_LIMIT)) and THEN sent its stop,
/// so the sidecar's teardown finds the hold. Holding newcomers at every poll is what keeps a resume
/// the old service starts at second 50 of the grace from reaching the bootout with nothing on
/// record — the 8n9a shape again.
///
/// **A hard deadline, not a generation check** (round-2 IR #6): the clock is read before each
/// workspace and before each hold, and no hold is recorded once the grace has run out — so the
/// bound on `until` (the grace plus one poll, then launchd's exit timeout, inside the margin)
/// holds whatever the store's writes cost. A session first seen past it is still stopped and is
/// reported `unrecorded`; the stranded sweep is its cover.
///
/// Every hold of one restart shares one `until` — `started_at` (to the second) plus the grace plus
/// the margin.
pub fn secure_the_instance(req: SecureTheInstance<'_>) -> RestartPreparation {
    let start = (req.clock)()
        .replace_nanosecond(0)
        .unwrap_or_else(|_| (req.clock)());
    let deadline = start + time::Duration::seconds(req.grace_secs);
    let until = start + time::Duration::seconds(req.grace_secs + req.margin_secs);
    let until_text = rfc3339(until);
    let mut report = RestartPreparation {
        started_at: Some(rfc3339(start)),
        until: Some(until_text.clone()),
        ..RestartPreparation::default()
    };
    // Per session: the workspace it runs in and whether its hold was recorded.
    let mut seen: BTreeMap<(String, String), bool> = BTreeMap::new();
    let mut engines: BTreeMap<PathBuf, crate::error::Result<crate::engine::Engine>> =
        BTreeMap::new();
    let mut polls = 0u32;
    let mut first = true;
    let stragglers = loop {
        polls += 1;
        (req.on_poll)(polls);
        let mut live: Vec<(String, String)> = Vec::new();
        for root in req.workspaces {
            let workspace = root.display().to_string();
            let grouped = crate::worker::live_sessions_in_group(root, req.pgid);
            if first {
                let prepared = |sessions: &[String], into: &mut Vec<PreparedSession>| {
                    into.extend(sessions.iter().map(|session| PreparedSession {
                        workspace: workspace.clone(),
                        session: session.clone(),
                    }));
                };
                prepared(&grouped.others, &mut report.left_alone);
                prepared(&grouped.unverifiable, &mut report.unverifiable);
                report
                    .unreadable
                    .extend(
                        grouped
                            .unreadable
                            .iter()
                            .map(|(session, errno)| SessionTrouble {
                                workspace: workspace.clone(),
                                session: session.clone(),
                                error: format!(
                                    "its process group could not be read: {}",
                                    std::io::Error::from_raw_os_error(*errno)
                                ),
                            }),
                    );
            }
            for session in grouped.ours {
                live.push((workspace.clone(), session.clone()));
                let key = (workspace.clone(), session.clone());
                if seen.contains_key(&key) {
                    continue;
                }
                let recorded = if (req.clock)() >= deadline {
                    Err(
                        "the grace had run out before the session was seen, so no hold was \
                         recorded for it"
                            .to_string(),
                    )
                } else {
                    let engine = engines
                        .entry(root.clone())
                        .or_insert_with(|| open_engine(root, &req.timer));
                    hold(engine, &session, &until_text, req.detail, (req.clock)())
                };
                // The stop is sent either way: the bootout would kill the session regardless, and a
                // stop at least lets it tear down.
                let stopped = crate::worker::SidecarWorker {
                    sidecar: PathBuf::new(),
                    cwd: root.clone(),
                }
                .stop_session(&session);
                if let Err(e) = recorded {
                    report.unrecorded.push(SessionTrouble {
                        workspace: workspace.clone(),
                        session: session.clone(),
                        error: match stopped {
                            Err(stop) => format!("{e}; and its stop failed: {stop}"),
                            Ok(_) => e,
                        },
                    });
                    seen.insert(key, false);
                } else {
                    if let Err(stop) = stopped {
                        eprintln!(
                            "warning: session {session} is on the restart's hold, but its stop \
                             could not be sent ({stop}); the restart's bootout ends it"
                        );
                    }
                    seen.insert(key, true);
                }
            }
        }
        first = false;
        if live.is_empty() {
            break Vec::new();
        }
        if (req.clock)() >= deadline {
            break live;
        }
        (req.sleep)(req.poll);
    };
    for (key, recorded) in seen {
        if !recorded {
            continue;
        }
        let prepared = PreparedSession {
            workspace: key.0.clone(),
            session: key.1.clone(),
        };
        match stragglers.contains(&key) {
            true => report.stopped.push(prepared),
            false => report.held.push(prepared),
        }
    }
    report.returned_at = Some(rfc3339((req.clock)()));
    report
}

/// Record one session's hold through the Engine seam the runtime callback refuses for this limit.
fn hold(
    engine: &mut crate::error::Result<crate::engine::Engine>,
    session: &str,
    until: &str,
    detail: &str,
    now: time::OffsetDateTime,
) -> std::result::Result<(), String> {
    let engine = engine
        .as_ref()
        .map_err(|e| format!("the workspace could not be opened: {e}"))?;
    let now = rfc3339(now);
    let receipt = engine
        .session_interrupted(
            crate::orchestration::Caller {
                session: None,
                actor: Some("service"),
                now: Some(&now),
            },
            session,
            crate::interruption::SERVICE_RESTART_LIMIT,
            Some(until),
            detail,
        )
        .map_err(|e| e.to_string())?;
    for w in receipt.warnings {
        eprintln!("warning: session {session}'s restart hold: {}", w.detail);
    }
    Ok(())
}

fn open_engine(
    root: &std::path::Path,
    timer: &TimerConfig,
) -> crate::error::Result<crate::engine::Engine> {
    crate::engine::Engine::open_with(
        None,
        root,
        crate::engine::EngineConfig {
            timer: timer.clone(),
            ..crate::engine::EngineConfig::default()
        },
    )
}

/// RFC3339 in UTC, to the second — the receipt's resolution.
fn rfc3339(t: time::OffsetDateTime) -> String {
    let t = t
        .to_offset(time::UtcOffset::UTC)
        .replace_nanosecond(0)
        .unwrap_or(t);
    t.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| t.to_string())
}
