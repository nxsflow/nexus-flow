//! A v1 workspace on disk, written the way v1 at schema 9 leaves one: `.nxs/` with `replica.toml`,
//! `config.toml`, `signing.key` and a `db.sqlite` in WAL mode, stamped `user_version` 9 and
//! `application_id` 4. Written by hand, without v1's crates, so that the reader is tested against
//! the format and not against the code that writes it.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::Connection;

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// v1's `ops` table at schema 9: the baseline columns, `domain` (v1), and the signing columns (v7).
const SCHEMA_9: &str = "
    CREATE TABLE ops(
        op_id TEXT PRIMARY KEY,
        lamport INTEGER NOT NULL,
        site INTEGER NOT NULL,
        target_kind TEXT NOT NULL,
        target_id TEXT NOT NULL,
        field TEXT NOT NULL,
        op_type TEXT NOT NULL,
        value TEXT,
        author TEXT,
        wall_clock TEXT,
        domain TEXT NOT NULL DEFAULT 'task',
        key_id TEXT,
        sig TEXT,
        provenance TEXT NOT NULL DEFAULT 'unsigned'
    );
    CREATE INDEX ops_target ON ops(target_id);
    CREATE INDEX ops_lamport_site ON ops(lamport, site);
    CREATE TABLE view_watermarks(store TEXT PRIMARY KEY, folded_through INTEGER NOT NULL);
    CREATE TABLE replica_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE trusted_keys(key_id TEXT PRIMARY KEY, public_key TEXT NOT NULL);
    CREATE TABLE log_base(
        id INTEGER PRIMARY KEY CHECK(id = 1),
        relay_position INTEGER NOT NULL,
        clock INTEGER NOT NULL,
        residue_count INTEGER NOT NULL DEFAULT 0,
        anchor TEXT,
        dropped INTEGER NOT NULL DEFAULT 0,
        dropped_digest TEXT NOT NULL DEFAULT ''
    );
";

/// One op row, as v1 stores it.
pub struct Op<'a> {
    pub op_id: &'a str,
    pub lamport: i64,
    pub site: i64,
    pub domain: &'a str,
    pub target_kind: &'a str,
    pub target_id: &'a str,
    pub field: &'a str,
    pub op_type: &'a str,
    pub value: Option<&'a str>,
    pub author: Option<&'a str>,
    pub wall_clock: Option<&'a str>,
    pub key_id: Option<&'a str>,
    pub sig: Option<&'a str>,
    pub provenance: &'a str,
}

impl<'a> Op<'a> {
    /// A flow `item` `set` of `title`, with every other column filled in.
    pub fn title(op_id: &'a str, lamport: i64, site: i64) -> Self {
        Op {
            op_id,
            lamport,
            site,
            domain: "task",
            target_kind: "item",
            target_id: "6j6v.pzkb",
            field: "title",
            op_type: "set",
            value: Some("\"A title\""),
            author: Some("carsten"),
            wall_clock: Some("2026-11-20T21:00:00Z"),
            key_id: Some("k1"),
            sig: Some("c2ln"),
            provenance: "own",
        }
    }
}

/// A repository root holding a v1 workspace in `.nxs/`, removed again when dropped.
pub struct Fixture {
    root: PathBuf,
}

impl Fixture {
    /// An empty directory, with no `.nxs/` in it.
    pub fn empty() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nex177 #{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Fixture { root }
    }

    /// An `issue-tracker` workspace at schema 9, bound to a sync stream, with no ops yet.
    pub fn v1() -> Self {
        let f = Fixture::empty();
        std::fs::create_dir_all(f.nxs()).unwrap();
        f.write(
            "replica.toml",
            "site_id = 1\nprefix = \"6j6v\"\nreplica_uuid = \"0b5d6c1e-replica\"\n",
        );
        f.write(
            "config.toml",
            "active_modules = [\"flow\"]\n\n[flow]\nplugin = \"issue-tracker\"\n\n[workspace]\nname = \"nexus-flow\"\n",
        );
        f.write(
            "sync.toml",
            "stream_id = \"nxsflow-nexus-flow\"\npushed_through = 12\npulled_through = 40\nendpoint = \"https://user:hunter2@relay.example/\"\n",
        );
        f.write("signing.key", "PRIVATE KEY BYTES");
        let conn = Connection::open(f.db()).unwrap();
        conn.execute_batch(SCHEMA_9).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        conn.pragma_update(None, "user_version", 9).unwrap();
        conn.pragma_update(None, "application_id", 4).unwrap();
        drop(conn);
        f
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn nxs(&self) -> PathBuf {
        self.root.join(".nxs")
    }

    pub fn db(&self) -> PathBuf {
        self.nxs().join("db.sqlite")
    }

    pub fn write(&self, file: &str, contents: &str) {
        std::fs::write(self.nxs().join(file), contents).unwrap();
    }

    pub fn remove(&self, file: &str) {
        std::fs::remove_file(self.nxs().join(file)).unwrap();
    }

    /// Run `sql` against the database with an ordinary connection, closed again before it returns.
    pub fn sql(&self, sql: &str) {
        let conn = Connection::open(self.db()).unwrap();
        conn.execute_batch(sql).unwrap();
    }

    pub fn insert(&self, ops: &[Op]) {
        let conn = Connection::open(self.db()).unwrap();
        for op in ops {
            conn.execute(
                "INSERT INTO ops(op_id, lamport, site, domain, target_kind, target_id, field,
                                 op_type, value, author, wall_clock, key_id, sig, provenance)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                rusqlite::params![
                    op.op_id,
                    op.lamport,
                    op.site,
                    op.domain,
                    op.target_kind,
                    op.target_id,
                    op.field,
                    op.op_type,
                    op.value,
                    op.author,
                    op.wall_clock,
                    op.key_id,
                    op.sig,
                    op.provenance
                ],
            )
            .unwrap();
        }
    }

    /// Every file under `.nxs/`, by its path relative to it, with its bytes.
    pub fn snapshot(&self) -> Vec<(String, Vec<u8>)> {
        fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            let mut entries: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let name = path.strip_prefix(base).unwrap().display().to_string();
                    out.push((name, std::fs::read(&path).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        if self.nxs().is_dir() {
            walk(&self.nxs(), &self.nxs(), &mut out);
        }
        out
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
