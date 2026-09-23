//! **Which version of its declaration a spawn ran under, and whether that is the version the last
//! spawn of the same role ran under** (nxf 6j6v.pkw9).
//!
//! # The measured incident
//!
//! `.nxs-personas/` is runtime configuration living in the working copy the declared agents are
//! told to work in. In the proving ground (4jgn.g90w, nxs 0.63.0) a rule added to `pm.yaml` was
//! never committed; a coder found the foreign uncommitted edit in its way and parked it in a
//! labelled stash — correctly — and a later coder read both stashes as leftovers. The PM then ran
//! without its answering rule, and **4 of 251 threads** ended with the sidecar answering in its
//! place. Three declarations (`pm.yaml`, `coder.yaml`, `finisher.yaml`) were rolled back that way.
//!
//! Nothing reported any of it. It was found by holding a stored `spec.json` against the file on
//! disk by hand, and only afterwards. That hand-work is what this module replaces.
//!
//! # What it does, and the two things it deliberately does not
//!
//! At every spawn the funnel ([`crate::orchestration::trigger_role`]) hashes the declaration the
//! prompt was composed from, writes it into the session's `spec.json` as `declarationHash`, and
//! asks this module whether the PREVIOUS spawn of the same role ran under a different one. If it
//! did, the caller's receipt carries a [`crate::orchestration::ConsequenceClass::DeclarationChanged`]
//! warning naming both versions.
//!
//! **Every receipt, not just the direct one** (review of PR #425). A session is put in motion by
//! five different shapes of call — a direct commission, a channel handing a member its next turn, a
//! chain hop resumed by a reply, a quorum completion waking its opener, and a parked commission
//! started later by whoever releases the working copy — and a finding that reached only the first
//! of them would have been loudest exactly where somebody is already watching and silent where
//! nobody is. The last of the five is the one that matters most: it runs inside `tick`'s sweep,
//! which the background service performs unattended, with no terminal for a breadcrumb to reach.
//!
//! * **It is not a refusal.** A change is usually intended; an unnoticed one never is. The spawn
//!   proceeds and the verb's exit code is unchanged (`crate::cli`'s `changes_the_exit_code`).
//! * **It is not a lock on the folder.** Owner's requirement, stated on the item: the OPERATION is
//!   to be stable, not the directory — the files stay editable and versionable at every moment, and
//!   nothing here writes to them or takes a handle on them. That is the same line
//!   [`crate::declaration_freeze`] draws, and this is the half that freeze cannot cover: a freeze
//!   makes an edit harmless for the operation ALREADY RUNNING, and says nothing about the next one.
//!
//! # What is hashed: the declaration as the ENGINE read it, not the file's bytes
//!
//! [`role_declaration_hash`] hashes the JSON projection of [`RoleDecl`], the same shape and for the
//! same reason [`crate::declaration_freeze`]'s `project` hashes its catalogue. Three consequences,
//! all of them wanted:
//!
//! 1. A reformatted or re-commented `.yaml` that declares exactly the same role is NOT a change,
//!    and would be one if the file's bytes were hashed. The point is to report a changed RULE, not
//!    a changed file.
//! 2. It works where there IS no file. A trigger re-fired out of the working-tree queue composes
//!    from its operation's FROZEN catalogue (`fire_queued_trigger`), which is a blob in the
//!    database; hashing "the file on disk" there would hash something the prompt was not built
//!    from.
//! 3. Nothing that steers a session can fall out of it. `handle`, `system_prompt`, `tools`,
//!    `model`, `prime`, `address_book`, `working_tree`, `permissions` — every one is a field of the
//!    struct, so every one is in the hash, with no second list of "the fields that matter" for
//!    somebody to forget to extend.
//!
//! **What it identifies is the DECLARATION, which is more than the prompt** (review of PR #425,
//! Code Quality #2). `reports_to` and `sub_agents` are declared and read by nothing
//! ([`crate::role`] says so), so editing one of them fires a warning for a change that steers no
//! session. That is the right side to be wrong on, and it is a choice rather than an oversight: the
//! alternative is a hand-maintained list of load-bearing fields, which drifts silently the first
//! time a field is added — and drifting silent is the entire failure this module exists to end.
//! The class says *the declaration changed*, and that is exactly what happened.
//!
//! It does **not** cover the other two layers a composed prompt has (the project's `CLAUDE.md` and
//! the prime block), and it does **not** cover the folder's OTHER declaration files — a
//! `channels.yaml` rolled back the same way steers just as much and is not watched here (nxf
//! 6j6v.x74v, which carries the three questions that half has to answer first). A reader of
//! `declarationHash` is told what it identifies, which is one role file.
//!
//! # Why a table of its own, device-local
//!
//! One row per role handle, holding the version its last spawn ran under. It is PLAIN and never an
//! op, for [`crate::declaration_freeze`]'s reason: `.nxs-personas/` is a folder on THIS machine's
//! disk, and a synced replica has its own — folding one device's reading of its own files into the
//! CRDT log would hand a remote device a version of a folder it does not have.

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::role::RoleDecl;
use crate::store::ChatStore;

/// **What the previous spawn of this role ran under** — returned by
/// [`ChatStore::note_spawn_declaration`] only when it is a DIFFERENT version from the one now being
/// spawned, which is the whole question this module answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviousSpawn {
    /// The content hash that spawn's declaration projected to.
    pub hash: String,
    /// When it was recorded — the engine's `now` at that spawn, so a reader can say how long the
    /// role had been running under the old version.
    pub seen: String,
}

impl ChatStore {
    /// **Record the version this spawn runs under, and report the previous one if it differed.**
    ///
    /// Called once per spawn, at the funnel every declared-role trigger passes through, and AFTER
    /// the worker has taken the trigger — the row means *the version the last spawn of this role
    /// ran under*, so neither a trigger that only joined the working-tree queue nor one the worker
    /// refused may advance it. Either would make the next spawn compare against a version nothing
    /// ever ran and swallow the change, which is the failure class this exists to end. See
    /// [`crate::orchestration::trigger_role`]'s call site for the race that placement costs.
    ///
    /// `None` is the ordinary answer, and it covers two cases on purpose — this role has never been
    /// spawned in this workspace, and it was spawned under exactly this version. Neither is worth a
    /// warning: the first has nothing to compare against, the second is the healthy steady state.
    ///
    /// **The read and the write are two statements, and that is a stated residual rather than an
    /// oversight.** [`ChatStore::freeze_declarations`] is deliberately ONE statement, so the
    /// difference is worth being precise about (review of PR #425, Integrity & Robustness #3).
    /// There, a lost race silently binds an operation to the wrong catalogue — a wrong ANSWER.
    /// Here, everything a lost race can cost is in the reporting:
    ///
    /// * two processes spawning the same role in the same instant both read the old version and
    ///   both report the change — a duplicated sentence;
    /// * under a three-writer interleave, the `previous_hash`/`previous_seen` a finding names can
    ///   come from a row that a third writer has already superseded — so the finding misstates *how
    ///   long* the role had been running under the old version, by one spawn.
    ///
    /// **What cannot happen is a missed detection**, and that is what makes the trade acceptable: a
    /// process reports whenever what it SPAWNS differs from what it READ, whichever row it read, so
    /// no change slips past unmentioned by the loser of any race. Closing the residual needs two
    /// more columns and a four-branch `CASE` in the upsert to answer, atomically, a question the
    /// two statements answer plainly — paid on every spawn, to sharpen a timestamp in a warning.
    ///
    /// If that ever stops being the right trade, the shape is known: it is
    /// [`ChatStore::freeze_declarations`]'s, one statement, with the previous values carried in
    /// columns of their own so `RETURNING` can hand them back.
    ///
    /// **It compares consecutive SPAWNS, not the folder's history, and one consequence of that is
    /// worth stating.** A trigger re-fired out of the working-tree queue composes from its own
    /// operation's FROZEN catalogue, so if the folder was edited while it waited, the sequence
    /// `old -> new -> old` can be recorded: the queued trigger's spawn genuinely does run under the
    /// older version, and it genuinely is not the version the spawn before it used. Reporting that
    /// is the contract the item states — *"aendert sich die Deklaration einer Rolle zwischen zwei
    /// Spawns, erfaehrt der Aufrufer das"* — and it is exactly the fact worth having: a session
    /// that starts under yesterday's rules is what pkw9 measured, whether it got there by a branch
    /// switch or by waiting in a queue.
    pub fn note_spawn_declaration(
        &mut self,
        role: &str,
        hash: &str,
        now: &str,
    ) -> Result<Option<PreviousSpawn>> {
        let conn = self.connection();
        let previous: Option<(String, String)> = conn
            .query_row(
                "SELECT hash, seen FROM role_declaration_version WHERE role = ?1",
                params![role],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        conn.execute(
            "INSERT INTO role_declaration_version(role, hash, seen) VALUES(?1, ?2, ?3)
             ON CONFLICT(role) DO UPDATE SET hash = excluded.hash, seen = excluded.seen",
            params![role, hash, now],
        )?;
        Ok(previous
            .filter(|(previous_hash, _)| previous_hash != hash)
            .map(|(hash, seen)| PreviousSpawn { hash, seen }))
    }
}

/// **One role declaration's content hash** — what a spawn records, what two spawns are compared on,
/// and what a session's `spec.json` carries as `declarationHash`. See this module's header for why
/// it hashes the projection rather than the file.
///
/// `serde_json` writes struct fields in declaration order, so one `RoleDecl` is one string and two
/// readings of an unchanged declaration collapse onto one hash — the same property
/// [`crate::declaration_freeze`] relies on for its catalogue blobs.
///
/// **`pub` because the VALUE is published**: it ships in every spec file this engine writes, and a
/// host that wants to answer "did that session run under this declaration?" has to be able to
/// compute the same number rather than re-derive the recipe from this source.
pub fn role_declaration_hash(decl: &RoleDecl) -> String {
    let projection = serde_json::to_string(decl).expect("a role declaration serializes");
    format!("{:x}", Sha256::digest(projection.as_bytes()))
}

/// **The short form a human reads**, and the only form that appears in prose: the first 12 hex
/// digits, which is what `git` and this project's own breadcrumbs use for the same job. The full
/// hash stays on the record ([`crate::orchestration::DeclarationChange`]) and in the `spec.json`,
/// so nothing a caller has to compare is ever truncated.
///
/// Cut on a CHARACTER boundary rather than at byte 12, because one of its two callers renders a
/// value a host supplied ([`crate::worker::RoleSpec::declaration_hash`] on a hand-built request).
/// Every value this crate produces is hex, where the two are the same; a `&s[..12]` would still be
/// a panic in the worker seam over a display detail, which is not a trade worth taking.
///
/// `pub(crate)`: it is how two in-crate sites RENDER a hash, not part of what the crate promises a
/// consumer. What a consumer compares is [`role_declaration_hash`], whole.
pub(crate) fn short(hash: &str) -> &str {
    match hash.char_indices().nth(12) {
        Some((at, _)) => &hash[..at],
        None => hash,
    }
}
