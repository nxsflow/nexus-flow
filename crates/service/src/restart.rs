//! **What a planned service restart did to the sessions it would have killed** (nxf 6j6v.wxz0).
//!
//! `nxs self-update` and `nxs sync daemon install` restart the background service with launchd's
//! `bootout` + `bootstrap`, and the bootout SIGTERMs — then SIGKILLs — the service's whole process
//! group. Every agent session the service started is in that group. So before the bootout, the
//! restart puts each of them on a `service_restart` hold, sends it a stop, and waits a grace for it
//! to wind down; the new service takes each one up again in place.
//!
//! This is the report of that preparation, in the one crate both sides link: the hook that runs it
//! lives with the chat engine, and the receipt that prints it lives with `self-update`.

use serde::Serialize;

/// One session the preparation saw, and the workspace it runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedSession {
    /// The workspace root.
    pub workspace: String,
    /// The internal session id.
    pub session: String,
}

/// A session the preparation saw and could not do its whole job for, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionTrouble {
    /// The workspace root.
    pub workspace: String,
    /// The internal session id.
    pub session: String,
    /// What went wrong — an error's text, or the `errno` of a process-group read.
    pub error: String,
}

/// **What the preparation did, per session** — the `prepared` object of the restart's receipt.
/// Declared field order = the JSON contract.
///
/// There is no refusal anywhere in it: the restart proceeds whatever it says, because a missed
/// session is caught by the backstops (the ladder, the stranded sweep) and refusing a planned
/// restart over one bad session would block it for every other.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RestartPreparation {
    /// When the preparation started, RFC3339 in UTC at second resolution. Absent when there was no
    /// instance to prepare (see [`note`](Self::note)).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// When it returned, at second resolution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returned_at: Option<String>,
    /// **The instant every hold of this restart falls due**: `started_at` plus the grace plus the
    /// margin. The old service is dead before it, so it can never fire a resume into its own group.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Put on hold, stopped, and gone within the grace.
    pub held: Vec<PreparedSession>,
    /// Put on hold and stopped, and still alive when the grace ran out — left to the bootout.
    pub stopped: Vec<PreparedSession>,
    /// Stopped, but its hold could not be recorded (or the grace had already run out when it was
    /// first seen). Nothing was retried here; the stranded sweep is its cover.
    pub unrecorded: Vec<SessionTrouble>,
    /// Live in an attended workspace but NOT in the restarted service's process group — started by
    /// a terminal or by another instance. The bootout does not reach it, so it was neither held nor
    /// stopped.
    pub left_alone: Vec<PreparedSession>,
    /// Live in the service's group, but its claim cannot prove the process is the session's: no
    /// hold recorded, no signal sent.
    pub unverifiable: Vec<PreparedSession>,
    /// Live, but its process group could not be read — the `errno`, in `error`.
    pub unreadable: Vec<SessionTrouble>,
    /// Why nothing was prepared at all, when nothing was — for instance, no running service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl RestartPreparation {
    /// A preparation that found no instance to work on, saying why.
    pub fn nothing(note: impl Into<String>) -> RestartPreparation {
        RestartPreparation {
            note: Some(note.into()),
            ..RestartPreparation::default()
        }
    }

    /// Whether it held, stopped or left alone anything at all.
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
            && self.stopped.is_empty()
            && self.unrecorded.is_empty()
            && self.left_alone.is_empty()
            && self.unverifiable.is_empty()
            && self.unreadable.is_empty()
    }
}
