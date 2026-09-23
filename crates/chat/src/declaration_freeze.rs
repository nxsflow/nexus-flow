//! **A running operation is bound to ONE version of the declarations** (nxf 6j6v.n92p, answering
//! the shape of nxf 6j6v.nby8) — taken when the operation is opened, read back by every step of it.
//!
//! # Why an operation has to be stable
//!
//! Personas and channels are declared as FILES so they can be versioned and reviewed. The price is
//! that a file can change in the middle of a chain: `#coding -> coder -> #reviewing -> coder` runs
//! for tens of minutes, and nothing stopped somebody editing a role description, a member list, a
//! `visibility`, or a `working_tree` half way through — the later steps then ran under different
//! rules from the earlier ones, nobody was warned, and afterwards it was not reconstructable which
//! version a step had actually run under. That is nby8, in the owner's words: *"Ich finde, dass ein
//! einzelner Vorgang stabil sein muss und sich nicht ändern darf."*
//!
//! nxf 6j6v.n92p made it urgent rather than merely untidy. A declared PRECONDITION is executable
//! code in one of those files, and the files sit in the very working copy the declared agents are
//! told to work in. A hurdle its own subject can switch off mid-chain is theatre.
//!
//! # The decision, and the two it beat
//!
//! **CHOSEN (owner, 2026-08-29): the declaration is frozen when an operation is OPENED.** A running
//! operation checks against the version read at its opening until it ends; an edit made while it
//! runs has no effect on it and takes effect in the NEXT operation.
//!
//! Rejected, with the reason each was rejected for:
//!
//! * **Read hurdles only from a file OUTSIDE the working copy.** The stronger guarantee — the
//!   hurdled could not reach it even between two operations — but it splits a channel declaration
//!   across two places. Somebody reading a channel would no longer see its hurdles standing beside
//!   it, and "the file is the authority" would stop holding throughout.
//! * **Only REPORT an edit made during a run.** Reporting is not preventing: whoever switches the
//!   hurdle off gets through, and it is in the log afterwards. That is precisely the "looks healthy
//!   from the outside" class this epic is built against.
//!
//! **The limit that comes with the choice, accepted deliberately**: an agent that slips BETWEEN two
//! operations is not caught by freezing. That is the price of keeping the declaration in one place.
//!
//! # It is done for the catalogue as a WHOLE
//!
//! Not for `preconditions:` alone. It is the same decision nby8 has to make anyway, and two notions
//! of "the version in force" standing beside each other would be worse than none: a channel's
//! members, its `visibility`, its `working_tree` and a persona's whole prompt are all read during a
//! chain, and freezing one field of one of them would leave a reader unable to say what "this
//! operation's declaration" meant.
//!
//! # What is stored, and where the boundary comes from
//!
//! nby8 §Design asks three questions before anything is built. Answered here:
//!
//! 1. **What is the frozen version — a hash, a copy, or a git ref?** A **copy, deduplicated by
//!    content hash**. A bare hash is cheap and makes drift visible but does not CARRY the content,
//!    so after an edit nobody can read what was in force. A copy carries it and would cost a
//!    catalogue per operation — except that the catalogue changes far more rarely than operations
//!    begin, so hashing the copy makes every operation under one unchanged folder share one blob. A
//!    git ref was rejected outright: `.nxs-personas/` is not guaranteed to be committed at all.
//! 2. **What if an operation needs a declaration that existed in its version and does not exist
//!    now?** It resolves, from the copy. That case — someone deletes a persona while it stands in a
//!    running chain — is the reason a bare hash was not enough.
//! 3. **Where does an operation begin?** At its ROOT THREAD, [`ChatStore::thread_root`], which is
//!    the boundary that function's own doc asks this to take rather than invent a second one. A
//!    `send --to` opens one; a re-commission out of a review runs inside the same one, because it
//!    hangs under the same tree.
//!
//! # What this is NOT
//!
//! It is not a lock on the folder. The owner's requirement is that the OPERATION is stable, not the
//! directory — the files stay editable and versionable at every moment, and nothing here writes to
//! them or takes a handle on them.

use rusqlite::params;
use sha2::{Digest, Sha256};

use crate::channel::ChannelDecl;
use crate::definitions::Definitions;
use crate::error::Result;
use crate::role::RoleDecl;
use crate::store::ChatStore;

/// The JSON projection one snapshot stores: the catalogue's two lists, and nothing derived.
///
/// [`crate::definitions::DeclarationSource`] is deliberately NOT in it. It says WHERE the folder is
/// and how many declarations it held, which is a fact about this machine's filesystem now rather
/// than about the version in force — and the reader re-attaches the live one, so a snapshot taken
/// before a workspace moved does not report a stale path.
#[derive(serde::Serialize, serde::Deserialize)]
struct Snapshot {
    roles: Vec<RoleDecl>,
    channels: Vec<ChannelDecl>,
}

impl ChatStore {
    /// **Bind this operation to the declarations as they are NOW** — called once, where an
    /// operation is opened.
    ///
    /// First write wins, and it has to be a single statement to mean that: `nxc send` from two
    /// processes can reach the start of one chain at once, and a read-then-write would let both
    /// bind it — to two readings of the folder, of which the loser's would silently be the one every
    /// later step used. `INSERT OR IGNORE` is what decides that race in SQLite rather than in
    /// whichever process happened to be scheduled second, the same reason
    /// [`ChatStore::claim_consolidation`] and `acquire_working_tree` are each one statement.
    ///
    /// Re-binding is therefore impossible by construction: a second call for the same root is a
    /// no-op, whatever the folder says by then. That is the whole guarantee, in one SQL keyword.
    ///
    /// `root_thread` must already BE a root ([`ChatStore::thread_root`]); this does not resolve it,
    /// because the two callers have it in hand at the moment they open the operation and resolving
    /// it again could only disagree with them.
    pub fn freeze_declarations(
        &mut self,
        root_thread: &str,
        now: &str,
        defs: &Definitions,
    ) -> Result<()> {
        let (hash, catalogue) = project(defs);
        let conn = self.connection();
        conn.execute(
            "INSERT OR IGNORE INTO declaration_snapshot(hash, taken, catalogue) VALUES(?1, ?2, ?3)",
            params![hash, now, catalogue],
        )?;
        conn.execute(
            "INSERT OR IGNORE INTO declaration_freeze(root_thread, hash, taken) VALUES(?1, ?2, ?3)",
            params![root_thread, hash, now],
        )?;
        Ok(())
    }

    /// **The declarations this operation is bound to**, or `None` when it is bound to none.
    ///
    /// `None` is the ordinary answer for a thread no operation ever opened — a plain conversation
    /// between two people, a thread that arrived over sync — and it means "read the folder", which
    /// is what every caller did before this existed. It is not a failure and not a hole: freezing
    /// belongs to operations the engine drives, and binding every thread anybody ever replied to
    /// would store a catalogue per conversation for a guarantee nothing there needs.
    ///
    /// The live `defs` is passed in for two things it alone can supply: the
    /// [`source`](Definitions::source) to re-attach, and the fallback below.
    ///
    /// **A snapshot that cannot be read falls back to the LIVE catalogue, loudly** — a stated
    /// residual rather than an oversight. The blob is this code's own JSON and tolerates a field
    /// added or retired by a later version exactly as a `.yaml` written by an older one does, so the
    /// reachable causes are a hand-edited row and a truncated database. Both are outside the model
    /// this protects (a careless or hurried EDIT to a declaration file), and the alternative —
    /// erroring — would wedge every remaining step of a chain that is otherwise perfectly healthy.
    /// It is a breadcrumb on stderr and not a silent fallback, because a workspace where this
    /// happens has a real problem somewhere else.
    ///
    /// **And it is not the weakest link, which is the part worth writing down** (PR #391 review,
    /// Integrity & Robustness #3 asked whether this should fail loudly instead). Reaching this arm
    /// needs WRITE ACCESS TO THE DATABASE — and anyone who has that can simply DELETE the
    /// `declaration_freeze` row, which lands on the `None` above and reads as "no operation bound
    /// this thread": the live catalogue, with no breadcrumb at all, because that is the ordinary
    /// shape of an untracked conversation and cannot be told apart from one. So hardening the
    /// corrupt-parse path buys nothing against the only actor who can reach it, while costing a
    /// healthy chain its remaining steps. Closing the class means authenticating the store, which is
    /// nxf 6j6v.6aza and not a decision this function may take on its own.
    pub fn frozen_declarations(
        &self,
        root_thread: &str,
        defs: &Definitions,
    ) -> Result<Option<Definitions>> {
        let stored: Option<String> = self
            .connection()
            .query_row(
                "SELECT s.catalogue
                   FROM declaration_freeze f JOIN declaration_snapshot s ON s.hash = f.hash
                  WHERE f.root_thread = ?1",
                params![root_thread],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })?;
        let Some(json) = stored else {
            return Ok(None);
        };
        let snapshot: Snapshot = match serde_json::from_str(&json) {
            Ok(snapshot) => snapshot,
            Err(e) => {
                eprintln!(
                    "warning: the frozen declarations of operation {root_thread} could not be read \
                     ({e}) — this step falls back to the declarations on disk, which may not be the \
                     ones the operation started under"
                );
                return Ok(None);
            }
        };
        let frozen = Definitions::new(snapshot.roles, snapshot.channels)?
            .with_source(defs.source().cloned());
        Ok(Some(frozen))
    }

    /// **Which version an operation ran under, for reading back afterwards** — the content hash and
    /// when it was taken, `None` for an operation that never bound one.
    ///
    /// nby8's acceptance asks for the version to be legible after the fact. This is the identifying
    /// half of that; the content itself comes back through
    /// [`frozen_declarations`](Self::frozen_declarations), which resolves the same row.
    pub fn declaration_freeze_of(&self, root_thread: &str) -> Result<Option<(String, String)>> {
        self.connection()
            .query_row(
                "SELECT hash, taken FROM declaration_freeze WHERE root_thread = ?1",
                params![root_thread],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(Into::into)
    }
}

/// One catalogue as (content hash, JSON). The hash is over the JSON, so two readings of an
/// unchanged folder collapse onto one blob and a changed one never can — `serde_json` writes struct
/// fields in declaration order and the two lists keep the loader's order, so the projection of one
/// catalogue is one string.
fn project(defs: &Definitions) -> (String, String) {
    let snapshot = Snapshot {
        roles: defs.roles().to_vec(),
        channels: defs.channels().to_vec(),
    };
    let catalogue = serde_json::to_string(&snapshot).expect("a declaration catalogue serializes");
    let hash = format!("{:x}", Sha256::digest(catalogue.as_bytes()));
    (hash, catalogue)
}
