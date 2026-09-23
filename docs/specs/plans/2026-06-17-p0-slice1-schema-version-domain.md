# P0-Slice 1 — Foundation-Schema-Version + `domain`-Diskriminator (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-op `domain` discriminator column (`task`/`fact`/`message`) plus a `PRAGMA user_version` migration spine to `nexus-flow-core`, additively and behaviour-preservingly — every existing flow op stays a `task` op and all derivation is byte-identical.

**Architecture:** The op-log gains a migration spine: `schema::migrate()` reads `PRAGMA user_version`, applies ordered DDL migrations once, and stamps the new version on every `Store` open. Its first migration (`v0 → v1`) adds `ops.domain TEXT NOT NULL DEFAULT 'task'`. The `Op` type then carries `domain`, threaded through `emit`/`ingest`/`export`. Nothing dispatches on `domain` yet (that is Slice 2's reducer registry) — Slice 1 is the dormant groundwork.

**Tech Stack:** Rust, `rusqlite` (bundled SQLite), `ulid`. `automerge` is a dev-dependency only (differential oracle) and must stay that way.

**Context — where this sits:** This is **Slice 1 of 4** of P0 (Foundation-Extraktion). Spec: `docs/specs/nxs-platform-foundation.md` §4.1 (kind/domain axis), §4.3 (Foundation-Schema-Version). Later slices get their own plans: Slice 2 = reducer registry (dispatches on `domain`), Slice 3 = `nxs-foundation` crate extraction, Slice 4 = `.nexusflow/` → `.nxs/` migration.

**Naming decision (load-bearing):** the new column is **`domain`**, not `kind`. The `ops` table already has `target_kind` (`item`/`edge`/`note`) — the *intra-domain* axis. `domain` (`task`/`fact`/`message`) is the *product/reducer* axis, one level up. The spec calls it "kind"; in code it is `domain` to avoid collision with the existing `target_kind`.

## Global Constraints

- **Quality gates (all must pass, CI enforces the same):** `cargo test`, `cargo test --release` (catches side-effects compiled out of release, e.g. logic inside `debug_assert!`), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
- **TDD:** test-first for all behaviour.
- **Behaviour-preserving:** the existing CLI trycmd goldens and the differential oracle (`crates/core/tests/differential.rs`, `tests/crdt_properties.rs`) are the regression oracle — do **not** modify them; they must stay green/byte-unchanged.
- **`automerge` stays a dev-dependency** (never in the binary).
- **Changelog:** no `nxf` user-facing change (internal substrate) → apply the **`skip-changelog`** label on the PR; do not add a `changes/*.md` fragment (per `changelog-scope-cli-only`).
- **Migration correctness:** a column added to the baseline DDL with `CREATE TABLE IF NOT EXISTS` does **not** appear in pre-existing databases — `domain` is therefore added by `migrate()` (an `ALTER TABLE`), gated on `user_version`, never by the baseline DDL.

---

## File Structure

- `crates/core/src/schema.rs` — **modify.** Add `SCHEMA_VERSION`, `schema_version()`, `migrate()`. The baseline `ops` DDL stays at the v0 shape (no `domain`). Owns the migration spine.
- `crates/core/src/store.rs` — **modify.** Call `schema::migrate()` in both open paths (`open_in_memory`, `open`); stamp `domain` in `emit()`; add the column to the `ingest()` INSERT and the `export()` SELECT.
- `crates/core/src/model.rs` — **modify.** Add `pub domain: String` to `Op` and a `DOMAIN_TASK` constant.

No new files in Slice 1.

---

## Task 1: Migration spine + dormant `domain` column

Establishes `PRAGMA user_version` versioning and the first migration (adds the `domain` column). The column is **dormant**: no code reads or writes it yet, so existing INSERTs let it take its `DEFAULT 'task'`. Fully behaviour-preserving.

**Files:**
- Modify: `crates/core/src/schema.rs`
- Modify: `crates/core/src/store.rs:46-72` (the two open paths)
- Test: `crates/core/src/schema.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `schema::SCHEMA_VERSION: i64` (= 1); `schema::schema_version(conn: &Connection) -> rusqlite::Result<i64>`; `schema::migrate(conn: &Connection) -> rusqlite::Result<()>`.
- Consumes: existing `schema::apply` / `schema::try_apply`, `Store::open` / `Store::open_in_memory`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/core/src/schema.rs`:

```rust
    #[test]
    fn fresh_schema_is_at_current_version() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply(&conn);
        migrate(&conn).unwrap();
        migrate(&conn).unwrap(); // second run must be a no-op, not an error
        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn migrate_upgrades_a_legacy_v0_database() {
        // A pre-migration database: the v0 `ops` shape (no `domain`), user_version 0,
        // with one op already in the log.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE ops(
                 op_id TEXT PRIMARY KEY, lamport INTEGER NOT NULL, site INTEGER NOT NULL,
                 target_kind TEXT NOT NULL, target_id TEXT NOT NULL, field TEXT NOT NULL,
                 op_type TEXT NOT NULL, value TEXT, author TEXT, wall_clock TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ops(op_id,lamport,site,target_kind,target_id,field,op_type,value,author,wall_clock)
             VALUES('01','1','1','item','t.A','title','set','A','u','')",
            [],
        )
        .unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 0, "legacy db starts at v0");

        migrate(&conn).unwrap();

        assert_eq!(schema_version(&conn).unwrap(), SCHEMA_VERSION);
        let domain: String = conn
            .query_row("SELECT domain FROM ops WHERE op_id='01'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(domain, "task", "pre-existing ops backfill as flow ops");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p nexus-flow-core schema:: 2>&1 | tail -20`
Expected: FAIL — compile error `cannot find function 'migrate' in this scope` (and `schema_version`, `SCHEMA_VERSION`).

- [ ] **Step 3: Implement the migration spine**

In `crates/core/src/schema.rs`, immediately after the `ITEM_LWW_FIELDS` constant (after line 22), add:

```rust
/// The current foundation schema version, persisted per database via `PRAGMA user_version`.
/// Bump this whenever a migration is added to [`migrate`]. A database created before the
/// migration spine existed reports version 0.
pub const SCHEMA_VERSION: i64 = 1;

/// The database's persisted schema version (`PRAGMA user_version`; 0 for any database
/// created before the migration spine existed).
pub fn schema_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
}

/// Bring `conn` from its persisted [`schema_version`] up to [`SCHEMA_VERSION`], applying each
/// ordered migration exactly once, then stamping the new version. Idempotent: a database
/// already at the current version does nothing. Every `Store` open path calls this right after
/// the baseline DDL ([`try_apply`]). The baseline DDL stays at the historical v0 shape; column
/// additions live here so fresh and pre-existing databases converge on the same schema.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let from = schema_version(conn)?;
    if from < 1 {
        // v0 -> v1: add the product/reducer `domain` axis (task/fact/message). NOT NULL
        // DEFAULT 'task' backfills every pre-existing op as a flow op, so derivation is
        // byte-identical. Distinct from `target_kind` (item/edge/note within a domain).
        conn.execute_batch("ALTER TABLE ops ADD COLUMN domain TEXT NOT NULL DEFAULT 'task';")?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}
```

- [ ] **Step 4: Wire `migrate` into both `Store` open paths**

In `crates/core/src/store.rs`, `open_in_memory` (line 48), after `schema::apply(&conn);` add the migrate call:

```rust
    pub fn open_in_memory(site: i64) -> Store {
        let conn = Connection::open_in_memory().expect("open sqlite");
        schema::apply(&conn);
        schema::migrate(&conn).expect("migrate");
        Store {
            conn,
            site,
            clock: 0,
            wall_clock: String::new(),
        }
    }
```

In `open` (lines 60-65), insert `schema::migrate(&conn)?;` after `try_apply` and before the clock recovery query:

```rust
    pub fn open(path: &str, site: i64) -> rusqlite::Result<Store> {
        let conn = Connection::open(path)?;
        schema::try_apply(&conn)?;
        schema::migrate(&conn)?;
        let clock: i64 = conn.query_row("SELECT COALESCE(MAX(lamport), 0) FROM ops", [], |r| {
            r.get(0)
        })?;
        Ok(Store {
            conn,
            site,
            clock,
            wall_clock: String::new(),
        })
    }
```

- [ ] **Step 5: Run the new tests to verify they pass**

Run: `cargo test -p nexus-flow-core schema:: 2>&1 | tail -20`
Expected: PASS — `fresh_schema_is_at_current_version`, `migrate_is_idempotent`, `migrate_upgrades_a_legacy_v0_database`, plus the pre-existing `schema_creates_all_objects` / `schema_is_idempotent`.

- [ ] **Step 6: Run the full gate to confirm behaviour is preserved**

The dormant column must not change any derivation: existing INSERTs omit `domain`, so it takes its default; nothing reads it yet.

Run: `cargo test && cargo test --release && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all green; differential oracle and trycmd goldens unchanged.

- [ ] **Step 7: Commit**

```bash
git add crates/core/src/schema.rs crates/core/src/store.rs
git commit -m "feat(aye.1): foundation schema-version spine + dormant ops.domain column

PRAGMA user_version migration spine (schema_version/migrate); first
migration adds ops.domain NOT NULL DEFAULT 'task'. Column is dormant —
no read/write path uses it yet, derivation byte-identical. P0-Slice 1
task 1 of 2.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Surface `domain` on the `Op` type + read/write paths

Make the column non-dormant in the in-memory model: `Op` carries `domain`, `emit()` stamps it `task`, `ingest()` persists it, `export()` reads it back. Foreign ops carrying a non-`task` domain round-trip unchanged (forward-compat for future products).

**Files:**
- Modify: `crates/core/src/model.rs:83-96` (the `Op` struct) + add `DOMAIN_TASK`
- Modify: `crates/core/src/store.rs:106-119` (`emit` Op literal), `:171-177` (`ingest` INSERT), `:394-419` (`export` SELECT)
- Test: `crates/core/src/store.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `schema::SCHEMA_VERSION` / `migrate` from Task 1 (the column exists).
- Produces: `Op.domain: String`; `model::DOMAIN_TASK: &str` (= "task"). Later slices read `Op.domain` to dispatch reducers.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `crates/core/src/store.rs`:

```rust
    #[test]
    fn local_ops_are_stamped_with_the_task_domain() {
        let mut s = Store::open_in_memory(1);
        s.create_item("d.A", ItemType::Task, "T", "u");
        let ops = s.export();
        assert!(!ops.is_empty());
        assert!(
            ops.iter().all(|o| o.domain == "task"),
            "flow emits only task-domain ops"
        );
    }

    #[test]
    fn foreign_non_task_domain_ops_round_trip_and_defer() {
        let mut s = Store::open_in_memory(1);
        // A future product's op: domain 'fact', a target_kind this version cannot fold.
        let foreign = Op {
            op_id: "01J0FAKEULID0000000000000".into(),
            lamport: 5,
            site: 2,
            domain: "fact".into(),
            target_kind: "fact".into(),
            target_id: "f.1".into(),
            field: "body".into(),
            op_type: "set".into(),
            value: Some("hello".into()),
            author: "u".into(),
            wall_clock: String::new(),
        };
        let deferred = s.apply(&[foreign.clone()]);
        assert_eq!(
            deferred,
            vec![foreign.clone()],
            "an unknown shape is deferred, not folded"
        );
        let exported = s.export();
        assert!(
            exported
                .iter()
                .any(|o| o.domain == "fact" && o.op_id == foreign.op_id),
            "the foreign op is stored with its domain preserved"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p nexus-flow-core store:: 2>&1 | tail -20`
Expected: FAIL — compile error `struct 'Op' has no field named 'domain'` / `no field 'domain' on type '&Op'`.

- [ ] **Step 3a: Add `domain` to the `Op` type and a `DOMAIN_TASK` constant**

In `crates/core/src/model.rs`, immediately before the `Op` struct (before line 83), add:

```rust
/// The product/reducer domain an op belongs to (the spec's "kind" axis). Slice 1 ships it
/// dormant — flow emits only `task`; later products register `fact` / `message`. Distinct
/// from `Op::target_kind` (item/edge/note *within* a domain).
pub const DOMAIN_TASK: &str = "task";
```

Then add the field to `Op` (between `site` and `target_kind`):

```rust
pub struct Op {
    pub op_id: String,
    pub lamport: i64,
    pub site: i64,
    pub domain: String, // "task" | "fact" | "message" — the product/reducer axis
    pub target_kind: String, // "item" | "edge" | "note" — within a domain
    pub target_id: String,
    pub field: String,
    pub op_type: String, // "set" | "add" | "remove" | "note_add"
    pub value: Option<String>,
    pub author: String,
    pub wall_clock: String,
}
```

- [ ] **Step 3b: Stamp `domain` in `emit()`**

In `crates/core/src/store.rs`, the `Op` literal inside `emit` (lines 106-119), add the `domain` field after `site`:

```rust
        let op = Op {
            op_id: ulid::Ulid::new().to_string(),
            lamport,
            site: self.site,
            domain: crate::model::DOMAIN_TASK.to_string(),
            target_kind: target_kind.to_string(),
            target_id: target_id.to_string(),
            field: field.to_string(),
            op_type: op_type.to_string(),
            value,
            author: author.to_string(),
            wall_clock: self.wall_clock.clone(),
        };
```

- [ ] **Step 3c: Persist `domain` in the `ingest()` INSERT**

In `crates/core/src/store.rs`, the INSERT in `ingest` (lines 171-177), add the `domain` column and its param:

```rust
            .execute(
                "INSERT OR IGNORE INTO ops
                 (op_id, lamport, site, domain, target_kind, target_id, field, op_type, value, author, wall_clock)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    op.op_id, op.lamport, op.site, op.domain, op.target_kind, op.target_id,
                    op.field, op.op_type, op.value, op.author, op.wall_clock
                ],
            )
```

- [ ] **Step 3d: Read `domain` back in `export()`**

In `crates/core/src/store.rs`, the SELECT and `Op` construction in `export` (lines 394-419), add `domain` (note the shifted `r.get` indices):

```rust
    pub fn export(&self) -> Vec<Op> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT op_id, lamport, site, domain, target_kind, target_id, field, op_type,
                        value, author, wall_clock FROM ops ORDER BY lamport, site",
            )
            .unwrap();
        stmt.query_map([], |r| {
            Ok(Op {
                op_id: r.get(0)?,
                lamport: r.get(1)?,
                site: r.get(2)?,
                domain: r.get(3)?,
                target_kind: r.get(4)?,
                target_id: r.get(5)?,
                field: r.get(6)?,
                op_type: r.get(7)?,
                value: r.get(8)?,
                author: r.get(9)?,
                wall_clock: r.get(10)?,
            })
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }
```

- [ ] **Step 3e: Fix any remaining `Op { .. }` literals**

Adding a field breaks every other `Op` struct literal. Find them:

Run: `grep -rn "Op {" crates/core/src crates/core/tests`
For each literal that does not yet set `domain` (other than the two already edited in `emit` and the new test), add `domain: crate::model::DOMAIN_TASK.to_string(),` (or `domain: "task".into(),` inside test code) after the `site` field. The compiler lists every site if you skip one — `cargo build -p nexus-flow-core` until it is clean.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p nexus-flow-core store:: 2>&1 | tail -20`
Expected: PASS — `local_ops_are_stamped_with_the_task_domain`, `foreign_non_task_domain_ops_round_trip_and_defer`, plus all pre-existing store tests.

- [ ] **Step 5: Run the full gate**

Run: `cargo test && cargo test --release && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all green; differential oracle and trycmd goldens unchanged (every emitted op is `task`, so convergence and goldens are unaffected).

- [ ] **Step 6: Commit**

```bash
git add crates/core/src/model.rs crates/core/src/store.rs
git commit -m "feat(aye.1): carry domain on Op; thread through emit/ingest/export

Op gains a `domain` field (DOMAIN_TASK = \"task\"); emit stamps it, ingest
persists it, export reads it back. Foreign non-task ops round-trip with
their domain preserved and defer (forward-compat). P0-Slice 1 task 2 of 2.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Out of scope / follow-ups (not this plan)

- **Reducer dispatch on `domain`** — Slice 2 (reducer registry). Slice 1 leaves `fold` dispatching on `(target_kind, op_type)` unchanged.
- **Wire-format / serde representation of `Op`** — if a later crate (`sync`, `facade`) serialises `Op` over the wire, it must add `domain` to that representation. `nexus-flow-core` itself has no serde dependency, so Slice 1 touches only the in-memory type and the SQLite column. Track when the sync wire format is next revised.
- **`.nexusflow/` → `.nxs/`**, crate extraction — Slices 3–4.

---

## Self-Review

**Spec coverage (§4.1, §4.3):** §4.3 Foundation-Schema-Version → Task 1 (`SCHEMA_VERSION`/`user_version`/`migrate`). §4.1 kind-tagged op-log → Task 1 (column) + Task 2 (`Op.domain` threaded through emit/ingest/export). "Behaviour unchanged, flow is first/only domain" → dormant column + all-`task` emit + green oracle/goldens. ✓

**Placeholder scan:** no TBD/"handle errors"/"similar to" — every code step shows the exact code; the one enumerated step (3e) gives the exact field to add and the grep to find the sites. ✓

**Type consistency:** `migrate`/`schema_version`/`SCHEMA_VERSION` (Task 1) are consumed by name in Task 2's premise (column exists); `Op.domain: String` and `DOMAIN_TASK: &str` are defined in Task 2 step 3a and used consistently in emit (`DOMAIN_TASK.to_string()`), the INSERT/SELECT (`op.domain` / `r.get(3)`), and both tests. Column order in INSERT (`...site, domain, target_kind...`) matches SELECT. ✓
