//! **Whom this replica believes** — the local trust list, and the one check an agent action passes
//! before it follows an op (nxf 6j6v.pzkb; the design is
//! `docs/specs/E4-auth-identity-and-signed-ops.md` §2.3–2.4).
//!
//! # Two halves, asked at two moments
//!
//! Where an op came from is settled when it enters the log and never changes: its
//! [`Provenance`] — `own`, `verified`, `invalid` or `unsigned` — is recorded beside it
//! (`ops.provenance`). Whether this replica BELIEVES the key that signed it changes whenever
//! somebody edits the trust list, so it is asked at the moment an action is decided, never stored
//! on the op. That is what makes revocation immediate: take a key off the list and its past ops stop
//! carrying actions at once.
//!
//! # The check
//!
//! An op carries an agent action iff it is this replica's own, or verified by a key on the trust
//! list. One definition, in SQL — the view `acting_ops`, which the decision reads that are SQL join
//! — and exposed to code as [`Store::acts_on`]. The board's convergence never asks: every op is
//! stored and folded whatever its provenance.
//!
//! # The list is local
//!
//! `trusted_keys` is a table no op writes, so no relay and no peer can extend it, and it is in no
//! image. This replica's own key is on it from the first open and cannot be taken off. There is no
//! MCP tool for it, on purpose: an agent's instruction must not be able to widen whom a machine
//! believes.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::error::{NxfError, Result};
use crate::signing::{parse_key_id, Provenance};
use crate::store::Store;

/// The longest name a trusted key may carry, in characters — the machine-name bound
/// (`nxs_service::machine::MAX_NAME_CHARS`), since the name is usually the other machine's.
pub const MAX_NAME_CHARS: usize = 64;

/// One key on the trust list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrustedKey {
    pub key_id: String,
    /// What the person who added it called it — usually the other machine's name. Empty for this
    /// replica's own key, which a reader shows as "this replica".
    pub name: String,
    /// This replica's own key: on the list from the first open, and it cannot be removed.
    pub own: bool,
    /// When it was added, as the caller's clock said; empty for the own key.
    pub added: String,
    /// How many ops in this log verified against this key (6j6v.pzkb) — for another replica's key,
    /// the receiver's own view of whether signatures arrive at all. A trusted key that signed
    /// nothing here while that machine's ops keep arriving is what a relay older than nxs 0.58
    /// looks like from the receiving end: it drops every signature, so no op is ever verified.
    /// (This replica's own ops are `own`, not `verified`, so for the own key it counts only its
    /// ops that came back through a snapshot or a rebuilt log.)
    pub verified_ops: i64,
}

/// A key that signed ops in this log but is not on the trust list — what `nxs sync trust list`
/// points at when somebody asks "which key is my other machine?". A pointer to what to COMPARE,
/// not a shortcut past comparing it: the authors are what the ops claim, and anybody who can reach
/// the relay can sign ops claiming any author.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UntrustedSigner {
    pub key_id: String,
    /// The distinct authors its verified ops name, sorted.
    pub authors: Vec<String>,
    /// How many verified ops it signed in this log.
    pub ops: i64,
}

/// Where one op came from, and whether an action may follow it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpProvenance {
    pub op_id: String,
    /// The name the op carries — for display; it proves nothing.
    pub author: String,
    pub provenance: Provenance,
    /// The key the op names, signed or not.
    pub key_id: Option<String>,
    /// Whether that key is on this replica's trust list.
    pub trusted: bool,
    /// The trust list's name for it, when it is on the list and has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_as: Option<String>,
    /// Whether an agent action may follow this op here and now.
    pub acts: bool,
}

/// The pending mark the v7 migration leaves (see `schema`): the ops already in the log still need
/// their verdict.
const LEGACY_OWN_PENDING: &str = "legacy_own_pending";

/// The two local tables of the signing schema (6j6v.pzkb), additive baseline DDL like
/// `view_watermarks` — created with the substrate and again by the v7 step, for a legacy db that
/// is migrated without the baseline having run first.
///
/// - `trusted_keys` — the keys this replica believes: whose signed ops may carry an agent action
///   here. LOCAL: no op writes it, so no relay and no peer can extend it, and it is in no image.
///   `own` marks this replica's own key(s), on it from the first open and never removable.
/// - `replica_meta` — small local facts about this replica's copy of the log that a migration
///   leaves for the store open to finish (see the v7 step). Never synced.
pub(crate) const TRUST_TABLES: &str = "
    CREATE TABLE IF NOT EXISTS trusted_keys(
        key_id TEXT PRIMARY KEY,
        name   TEXT NOT NULL DEFAULT '',
        own    INTEGER NOT NULL DEFAULT 0,
        added  TEXT NOT NULL DEFAULT ''
    );
    CREATE TABLE IF NOT EXISTS replica_meta(
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );";

/// The signing schema's derived objects, re-attempted on every open like the coordinate index
/// (6j6v.pzkb): the view every decision read joins, the partial index the clock's own-op
/// high-water mark reads, and the one a sync pass asks "does this site sign?" through.
pub(crate) fn ensure_acting_ops(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(TRUST_TABLES)?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS ops_own_lamport ON ops(lamport) WHERE provenance = 'own';
         CREATE INDEX IF NOT EXISTS ops_signed_site ON ops(site) WHERE key_id IS NOT NULL;
         CREATE VIEW IF NOT EXISTS acting_ops AS
             SELECT op_id, lamport, site FROM ops
              WHERE provenance = 'own'
                 OR (provenance = 'verified'
                     AND key_id IN (SELECT key_id FROM trusted_keys));",
    )
}

/// Seat this replica in its own log, on every store open: its key on the trust list, and — once,
/// after the v7 migration — the verdict on every op that was already there.
///
/// **The old ops.** Nothing was signed before v7, so the log cannot tell this replica's own ops
/// from received ones by their signature. Those on this replica's `site` are taken as its own; a
/// received op that claimed that site before the upgrade is taken as own too — nothing was checked
/// before, so this makes nothing worse, and everything received from here on is judged by its
/// signature. The mark is consumed in the same transaction that stamps, so it happens exactly once.
///
/// Reads first and takes the write lock only when there is something to write, so the ordinary
/// open of a seated replica costs two indexed lookups and no lock.
pub(crate) fn seat_replica(conn: &Connection, site: i64, key_id: &str) -> rusqlite::Result<()> {
    let seated = |conn: &Connection| -> rusqlite::Result<bool> {
        let own = conn
            .query_row(
                "SELECT 1 FROM trusted_keys WHERE key_id = ?1 AND own = 1",
                [key_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let pending = conn
            .query_row(
                "SELECT 1 FROM replica_meta WHERE key = ?1",
                [LEGACY_OWN_PENDING],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(own && !pending)
    };
    if seated(conn)? {
        return Ok(());
    }
    fn seat(conn: &Connection, site: i64, key_id: &str) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO trusted_keys(key_id, own) VALUES(?1, 1)
             ON CONFLICT(key_id) DO UPDATE SET own = 1",
            [key_id],
        )?;
        let pending = conn.execute(
            "DELETE FROM replica_meta WHERE key = ?1",
            [LEGACY_OWN_PENDING],
        )?;
        if pending > 0 {
            conn.execute(
                "UPDATE ops SET provenance = 'own'
                  WHERE site = ?1 AND provenance = 'unsigned' AND key_id IS NULL AND sig IS NULL",
                [site],
            )?;
        }
        Ok(())
    }
    conn.execute_batch("BEGIN IMMEDIATE")?;
    match seat(conn, site, key_id) {
        Ok(()) => conn.execute_batch("COMMIT"),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

impl Store {
    /// The trust list: this replica's own key first, then the others by key id.
    pub fn trusted_keys(&self) -> Vec<TrustedKey> {
        let mut stmt = self
            .connection()
            .prepare(
                "SELECT t.key_id, t.name, t.own, t.added,
                        (SELECT COUNT(*) FROM ops o
                          WHERE o.provenance = 'verified' AND o.key_id = t.key_id)
                   FROM trusted_keys t ORDER BY t.own DESC, t.key_id",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok(TrustedKey {
                key_id: r.get(0)?,
                name: r.get(1)?,
                own: r.get::<_, i64>(2)? != 0,
                added: r.get(3)?,
                verified_ops: r.get(4)?,
            })
        })
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect()
    }

    /// Put `key_id` on the trust list under `name`, as of `now` — from here on, the ops it signed
    /// (past ones included) may carry agent actions on this replica. `true` when it was not on the
    /// list before; adding a key that is already there only renames it (to a non-empty `name`).
    ///
    /// Refuses a key id that names no Ed25519 public key, and a name that is not one line of at
    /// most [`MAX_NAME_CHARS`] characters. The key id is the FULL key, never a prefix of one: the
    /// point of comparing it out of band is that nobody can produce another key that matches it.
    pub fn trust_key(&mut self, key_id: &str, name: &str, now: &str) -> Result<bool> {
        let key_id = key_id.trim();
        if parse_key_id(key_id).is_none() {
            return Err(NxfError::validation(format!(
                "{key_id:?} is not a key id — expected the full `ed25519:…` string another replica \
                 prints with `nxs sync key`"
            )));
        }
        let name = valid_name(name)?;
        let known = self
            .connection()
            .query_row(
                "SELECT 1 FROM trusted_keys WHERE key_id = ?1",
                [key_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if known {
            if !name.is_empty() {
                self.connection().execute(
                    "UPDATE trusted_keys SET name = ?2 WHERE key_id = ?1 AND own = 0",
                    params![key_id, name],
                )?;
            }
            return Ok(false);
        }
        self.connection().execute(
            "INSERT INTO trusted_keys(key_id, name, own, added) VALUES(?1, ?2, 0, ?3)",
            params![key_id, name, now],
        )?;
        Ok(true)
    }

    /// Take `key_id` off the trust list — revocation. Effective at once: trust is asked when an
    /// action is decided, so none of the ops it signed carries one from here on. `true` when it was
    /// on the list. Refuses this replica's own key.
    pub fn distrust_key(&mut self, key_id: &str) -> Result<bool> {
        let key_id = key_id.trim();
        let own: Option<i64> = self
            .connection()
            .query_row(
                "SELECT own FROM trusted_keys WHERE key_id = ?1",
                [key_id],
                |r| r.get(0),
            )
            .optional()?;
        match own {
            None => Ok(false),
            Some(own) if own != 0 => Err(NxfError::validation(
                "that is this replica's own key — it cannot be taken off its own trust list",
            )),
            Some(_) => {
                self.connection()
                    .execute("DELETE FROM trusted_keys WHERE key_id = ?1", [key_id])?;
                Ok(true)
            }
        }
    }

    /// The keys that signed verified ops in this log without being on the trust list, by key id.
    pub fn untrusted_signers(&self) -> Vec<UntrustedSigner> {
        let mut stmt = self
            .connection()
            .prepare(
                "SELECT key_id, COUNT(*) FROM ops
                  WHERE provenance = 'verified'
                    AND key_id NOT IN (SELECT key_id FROM trusted_keys)
                  GROUP BY key_id ORDER BY key_id",
            )
            .unwrap();
        let signers: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        let mut authors = self
            .connection()
            .prepare(
                "SELECT DISTINCT author FROM ops
                  WHERE provenance = 'verified' AND key_id = ?1 ORDER BY author",
            )
            .unwrap();
        signers
            .into_iter()
            .map(|(key_id, ops)| UntrustedSigner {
                authors: authors
                    .query_map([&key_id], |r| r.get(0))
                    .unwrap()
                    .map(std::result::Result::unwrap)
                    .collect(),
                key_id,
                ops,
            })
            .collect()
    }

    /// Where the op `op_id` came from and whether an action may follow it, or `None` for an op this
    /// log does not hold.
    pub fn op_provenance(&self, op_id: &str) -> Option<OpProvenance> {
        self.connection()
            .query_row(
                "SELECT o.op_id, o.author, o.provenance, o.key_id,
                        t.key_id IS NOT NULL, NULLIF(t.name, ''),
                        EXISTS(SELECT 1 FROM acting_ops a WHERE a.op_id = o.op_id)
                   FROM ops o LEFT JOIN trusted_keys t ON t.key_id = o.key_id
                  WHERE o.op_id = ?1",
                [op_id],
                |r| {
                    let provenance: String = r.get(2)?;
                    Ok(OpProvenance {
                        op_id: r.get(0)?,
                        author: r.get(1)?,
                        provenance: Provenance::parse(&provenance).unwrap_or(Provenance::Invalid),
                        key_id: r.get(3)?,
                        trusted: r.get(4)?,
                        trusted_as: r.get(5)?,
                        acts: r.get(6)?,
                    })
                },
            )
            .optional()
            .unwrap()
    }

    /// **The check before an action** — whether an agent action may follow the op `op_id` on this
    /// replica now: it is this replica's own, or its signature verified against a key on the trust
    /// list. `false` for an unsigned, invalid or untrusted op, and for an op this log does not hold.
    pub fn acts_on(&self, op_id: &str) -> bool {
        self.connection()
            .query_row("SELECT 1 FROM acting_ops WHERE op_id = ?1", [op_id], |_| {
                Ok(())
            })
            .optional()
            .unwrap()
            .is_some()
    }
}

fn valid_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(NxfError::validation(format!(
            "a trusted key's name is one line of at most {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Op;
    use crate::reducer::Reducer;

    /// One LWW register per id, like every real register — enough to see that an op of any
    /// provenance folds.
    struct Tally;
    impl Reducer for Tally {
        fn domain(&self) -> &'static str {
            "tally"
        }
        fn is_foldable(&self, op: &Op) -> bool {
            op.target_kind == "tally"
        }
        fn fold(&self, conn: &Connection, op: &Op) {
            conn.execute(
                "INSERT INTO tally(id, v, v_l, v_s) VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET v=excluded.v, v_l=excluded.v_l, v_s=excluded.v_s
                 WHERE (excluded.v_l, excluded.v_s) > (v_l, v_s)",
                params![op.target_id, op.value, op.lamport, op.site],
            )
            .unwrap();
        }
    }

    fn replica(site: i64) -> Store {
        let mut s = Store::open_in_memory(site);
        s.connection()
            .execute_batch(
                "CREATE TABLE tally(id TEXT PRIMARY KEY, v TEXT, v_l INTEGER, v_s INTEGER)",
            )
            .unwrap();
        s.register_reducer(Box::new(Tally));
        s
    }

    fn write(s: &mut Store, id: &str, value: &str) -> String {
        s.emit(
            "tally",
            "tally",
            id,
            "v",
            "set",
            Some(value.into()),
            "alice",
        )
    }

    fn value(s: &Store, id: &str) -> Option<String> {
        s.connection()
            .query_row("SELECT v FROM tally WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .unwrap()
    }

    fn provenance(s: &Store, op_id: &str) -> Provenance {
        s.op_provenance(op_id).unwrap().provenance
    }

    #[test]
    fn a_local_op_is_signed_with_the_replica_key_and_is_its_own() {
        let mut a = replica(1);
        let op_id = write(&mut a, "t.1", "hello");
        let op = a.export().pop().unwrap();
        assert_eq!(op.key_id.as_deref(), Some(a.key_id()));
        assert!(crate::signing::verify(
            a.key_id(),
            op.sig.as_deref().unwrap(),
            &op.canonical_bytes()
        ));
        let seen = a.op_provenance(&op_id).unwrap();
        assert_eq!(seen.provenance, Provenance::Own);
        assert!(seen.trusted && seen.acts);
        assert!(a.acts_on(&op_id));
    }

    #[test]
    fn a_received_op_acts_only_while_its_key_is_trusted() {
        let (mut a, mut b) = (replica(1), replica(2));
        let op_id = write(&mut a, "t.1", "hello");
        b.apply(&a.export());
        assert_eq!(provenance(&b, &op_id), Provenance::Verified);
        assert_eq!(
            value(&b, "t.1").as_deref(),
            Some("hello"),
            "folded regardless"
        );
        assert!(
            !b.acts_on(&op_id),
            "verified, but nobody here trusts that key"
        );

        assert!(b
            .trust_key(a.key_id(), "laptop", "2026-09-22T10:00:00Z")
            .unwrap());
        assert!(b.acts_on(&op_id), "trusting the key lets its past ops act");
        let seen = b.op_provenance(&op_id).unwrap();
        assert_eq!(seen.trusted_as.as_deref(), Some("laptop"));

        assert!(b.distrust_key(a.key_id()).unwrap());
        assert!(
            !b.acts_on(&op_id),
            "revocation is immediate, past ops included"
        );
        assert!(!b.distrust_key(a.key_id()).unwrap(), "already off the list");
    }

    /// A relay that alters an op or drops its signature cannot make it act — and the board still
    /// converges on what the relay delivered, because the fold never asks.
    #[test]
    fn an_altered_or_stripped_op_is_stored_and_folded_but_never_acts() {
        let (mut a, mut b) = (replica(1), replica(2));
        b.trust_key(a.key_id(), "laptop", "").unwrap();
        write(&mut a, "t.1", "hello");
        write(&mut a, "t.2", "hello");
        let mut ops = a.export();
        ops[0].value = Some("altered".into());
        ops[1].key_id = None;
        ops[1].sig = None;
        b.apply(&ops);
        assert_eq!(provenance(&b, &ops[0].op_id), Provenance::Invalid);
        assert_eq!(provenance(&b, &ops[1].op_id), Provenance::Unsigned);
        assert_eq!(value(&b, "t.1").as_deref(), Some("altered"), "folded");
        assert_eq!(value(&b, "t.2").as_deref(), Some("hello"), "folded");
        assert!(!b.acts_on(&ops[0].op_id));
        assert!(!b.acts_on(&ops[1].op_id), "a stripped signature never acts");
    }

    /// Replay (spec §2.5): the same op again is the idempotent union's no-op — and an altered copy
    /// of an op already held is a re-delivery of a known id, which changes neither the value nor
    /// the verdict.
    #[test]
    fn a_replayed_or_altered_redelivery_changes_nothing_not_even_the_verdict() {
        let (mut a, mut b) = (replica(1), replica(2));
        b.trust_key(a.key_id(), "laptop", "").unwrap();
        let op_id = write(&mut a, "t.1", "hello");
        let genuine = a.export();
        b.apply(&genuine);
        b.apply(&genuine);
        let mut altered = genuine.clone();
        altered[0].value = Some("altered".into());
        b.apply(&altered);
        assert_eq!(b.op_count(), 1);
        assert_eq!(value(&b, "t.1").as_deref(), Some("hello"));
        assert_eq!(provenance(&b, &op_id), Provenance::Verified);
        assert!(b.acts_on(&op_id));
    }

    /// The known limit, pinned so it stays a stated one (spec §2.5; review of PR #489, Test Quality
    /// #1): a relay that serves its ALTERED copy of an op FIRST wins that op id on this replica. The
    /// genuine op arriving afterwards is a re-delivery of a known id and is dropped — the log keeps
    /// the altered copy, which never acts. A denial (the genuine instruction never acts here), never
    /// a forgery (the altered one does not act either).
    #[test]
    fn an_altered_copy_delivered_first_keeps_the_op_id_and_acts_nowhere() {
        let (mut a, mut b) = (replica(1), replica(2));
        b.trust_key(a.key_id(), "laptop", "").unwrap();
        let op_id = write(&mut a, "t.1", "deploy the docs");
        let genuine = a.export();
        let mut altered = genuine.clone();
        altered[0].value = Some("delete the release branch".into());

        b.apply(&altered);
        b.apply(&genuine);
        assert_eq!(b.op_count(), 1, "one op id, one row");
        assert_eq!(
            value(&b, "t.1").as_deref(),
            Some("delete the release branch"),
            "the first copy stands"
        );
        assert_eq!(provenance(&b, &op_id), Provenance::Invalid);
        assert!(
            !b.acts_on(&op_id),
            "and it acts nowhere, trusted key or not"
        );
    }

    /// Replay into ANOTHER workspace (spec §2.1.1): the key is the replica's, so a validly signed op
    /// copied into a stream where that replica does not take part arrives from a key nobody there
    /// trusts — even on a machine that trusts the SAME machine's key in the op's own workspace.
    #[test]
    fn a_signed_op_replayed_into_another_workspace_does_not_act_there() {
        let (mut laptop_a, mut desk_a) = (replica(1), replica(2));
        let (laptop_b, mut desk_b) = (replica(3), replica(4));
        desk_a.trust_key(laptop_a.key_id(), "laptop", "").unwrap();
        desk_b.trust_key(laptop_b.key_id(), "laptop", "").unwrap();
        let op_id = write(&mut laptop_a, "t.1", "deploy");
        desk_a.apply(&laptop_a.export());
        desk_b.apply(&laptop_a.export());
        assert!(desk_a.acts_on(&op_id), "in its own workspace it acts");
        assert_eq!(provenance(&desk_b, &op_id), Provenance::Verified);
        assert!(!desk_b.acts_on(&op_id), "copied into another it does not");
    }

    #[test]
    fn the_own_key_is_on_the_list_from_the_start_and_cannot_be_taken_off() {
        let mut a = replica(1);
        let keys = a.trusted_keys();
        assert_eq!(keys.len(), 1);
        assert!(keys[0].own && keys[0].key_id == a.key_id());
        let own = a.key_id().to_string();
        assert!(a.distrust_key(&own).is_err());
        assert!(!a.trust_key(&own, "me", "").unwrap(), "already there");
        assert_eq!(
            a.trusted_keys()[0].name,
            "",
            "and the own key is not renamed"
        );
    }

    #[test]
    fn trusting_refuses_what_is_not_a_full_key_id_or_a_one_line_name() {
        let mut a = replica(1);
        let other = crate::signing::ReplicaKey::generate();
        for bad in [
            "",
            "laptop",
            &other.key_id()[..20],
            &other.key_id()["ed25519:".len()..],
        ] {
            assert!(a.trust_key(bad, "x", "").is_err(), "{bad:?}");
        }
        assert!(a.trust_key(other.key_id(), "two\nlines", "").is_err());
        assert!(a.trust_key(other.key_id(), &"x".repeat(65), "").is_err());
        assert!(a
            .trust_key(&format!("  {}  ", other.key_id()), "ok", "")
            .unwrap());
        assert_eq!(a.trusted_keys().len(), 2);
    }

    #[test]
    fn untrusted_signers_name_the_keys_a_person_would_compare() {
        let (mut a, mut b) = (replica(1), replica(2));
        write(&mut a, "t.1", "x");
        a.emit("tally", "tally", "t.2", "v", "set", None, "bob");
        b.apply(&a.export());
        let seen = b.untrusted_signers();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].key_id, a.key_id());
        assert_eq!(seen[0].authors, ["alice", "bob"]);
        assert_eq!(seen[0].ops, 2);
        b.trust_key(a.key_id(), "a", "").unwrap();
        assert!(b.untrusted_signers().is_empty());
        let listed = b.trusted_keys();
        assert_eq!((listed[0].own, listed[0].verified_ops), (true, 0));
        assert_eq!(
            listed[1].verified_ops, 2,
            "what the trusted key signed here"
        );
    }

    /// The v7 migration gives the ops already in the log their verdict: this replica's own site is
    /// its own, everything else stays unsigned — exactly once.
    #[test]
    fn the_ops_of_an_upgraded_log_are_own_on_this_site_and_unsigned_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE ops(
                     op_id TEXT PRIMARY KEY, lamport INTEGER NOT NULL, site INTEGER NOT NULL,
                     target_kind TEXT NOT NULL, target_id TEXT NOT NULL, field TEXT NOT NULL,
                     op_type TEXT NOT NULL, value TEXT, author TEXT, wall_clock TEXT,
                     domain TEXT NOT NULL DEFAULT 'task');
                 INSERT INTO ops VALUES('mine', 1, 1, 'item', 'x', 'title', 'set', 'a', 'u', '', 'task');
                 INSERT INTO ops VALUES('theirs', 2, 9, 'item', 'x', 'title', 'set', 'b', 'v', '', 'task');
                 PRAGMA user_version = 6;",
            )
            .unwrap();
        }
        let path = path.to_str().unwrap();
        let s = Store::open(path, 1).unwrap();
        assert_eq!(provenance(&s, "mine"), Provenance::Own);
        assert_eq!(provenance(&s, "theirs"), Provenance::Unsigned);
        assert!(s.acts_on("mine") && !s.acts_on("theirs"));
        drop(s);

        // A received op claiming this site AFTER the upgrade is judged by its signature, not its site.
        let mut s = Store::open(path, 1).unwrap();
        let mut forged = s.export().pop().unwrap();
        forged.op_id = "forged".into();
        forged.lamport = 3;
        forged.site = 1;
        s.apply(&[forged]);
        drop(s);
        let s = Store::open(path, 1).unwrap();
        assert_eq!(provenance(&s, "forged"), Provenance::Unsigned);
    }

    /// The key lives beside the db: every handle on one workspace signs with ONE key, and the key a
    /// handle holds after a reopen is the one its earlier ops name.
    #[test]
    fn every_handle_on_one_workspace_signs_with_the_one_key_beside_its_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        let path = path.to_str().unwrap();
        let first = Store::open(path, 1).unwrap();
        let second = Store::open(path, 1).unwrap();
        assert_eq!(first.key_id(), second.key_id());
        assert!(dir.path().join(crate::signing::KEY_FILE).is_file());
        drop((first, second));
        assert_eq!(
            Store::open(path, 1).unwrap().trusted_keys().len(),
            1,
            "one own key, however often opened"
        );
    }

    /// An image is received: every op is verified by the LOADING store, whatever verdict the image
    /// carries, and the trust list does not travel with it.
    #[test]
    fn an_image_is_judged_by_the_store_that_loads_it_and_carries_no_trust() {
        use crate::image::Cell;
        let (mut a, mut b) = (replica(1), replica(2));
        a.trust_key(b.key_id(), "b", "").unwrap();
        let signed = write(&mut a, "t.1", "x");
        let mut unsigned = b.export();
        write(&mut b, "t.2", "y");
        unsigned = b
            .export()
            .into_iter()
            .filter(|o| !unsigned.contains(o))
            .collect();
        unsigned[0].key_id = None;
        unsigned[0].sig = None;
        a.apply(&unsigned);

        let mut image = a.image().unwrap();
        let col = |n: &str| image.ops.columns.iter().position(|c| c == n).unwrap();
        let (prov, op_id) = (col("provenance"), col("op_id"));
        for row in &mut image.ops.rows {
            row[prov] = Cell::Text("own".into()); // a crafted image claiming everything is own
        }
        assert!(image
            .ops
            .rows
            .iter()
            .any(|r| r[op_id] == Cell::Text(signed.clone())));

        let mut c = replica(3);
        c.load_image(&image).unwrap();
        assert_eq!(provenance(&c, &signed), Provenance::Verified);
        assert_eq!(provenance(&c, &unsigned[0].op_id), Provenance::Unsigned);
        assert_eq!(c.trusted_keys().len(), 1, "only its own key");
        assert!(!c.acts_on(&signed));
    }

    /// An image taken before ops were signed has no signature columns; it still loads, every op
    /// unsigned.
    #[test]
    fn an_image_from_before_signing_loads_with_every_op_unsigned() {
        let mut a = replica(1);
        let op_id = write(&mut a, "t.1", "x");
        let mut image = a.image().unwrap();
        for name in ["key_id", "sig", "provenance"] {
            let at = image.ops.columns.iter().position(|c| c == name).unwrap();
            image.ops.columns.remove(at);
            for row in &mut image.ops.rows {
                row.remove(at);
            }
        }
        let mut c = replica(3);
        c.load_image(&image).unwrap();
        assert_eq!(provenance(&c, &op_id), Provenance::Unsigned);
    }
}
