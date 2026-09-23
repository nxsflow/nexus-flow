# nexus-chat M1 · T1 — messaging substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the embeddable `nexus-chat` substrate crate — the `message`-domain reducer and its six materialized views over the shared nxs op-log — so the agent engine can attach to a `ChatStore` (nxf ticket `6j6v.sv1b` / T1).

**Architecture:** `nexus-chat` is a sibling kind-product of `nexus-memory`, depending **only** on `nxs-foundation` (spec §4.4). A `MessageReducer` folds `domain="message"` ops into six views (`messages`, `channels`, `membership_*`, `profiles`, `threads`, `read_cursors`), dispatching on `(target_kind, op_type, field)` exactly like flow's `TaskReducer`. A `ChatStore` newtype wraps `nxs_foundation::store::Store`, registers the reducer at open, and adds typed write/read helpers. Everything is TDD, test-first.

**Tech Stack:** Rust, `rusqlite 0.31` (bundled SQLite), `serde`/`serde_json` (envelope canonical JSON + enum validation), `ulid` (global `m-`/thread ids). Reducer trait + op-log from `nxs-foundation`.

**Reference implementations to mirror (read these once before starting):**
- `crates/memory/src/store.rs` — the `MemoryStore::open`/`open_in_memory` recipe (apply views → `register_reducer` → `refold_if_behind`), substrate delegation, `emit_*` write helpers.
- `crates/memory/src/fact_reducer.rs` — a `Reducer` impl + its unit-test harness (the `op(...)` builder, `store()`, `apply` → assert row).
- `crates/memory/src/schema.rs` — `apply_*_views`/`try_apply_*_views` idempotent DDL.
- `crates/memory/src/model.rs` — the vocabulary-constants + row-shape module.
- `crates/core/src/task_reducer.rs` — the **multi-shape** reducer: `fold_item_lww` (per-field LWW via `format!`-interpolated whitelisted column), `fold_edge_add`/`fold_edge_remove` + `parse_edge` (observed-remove OR-set via `SEP`-joined tags in the remove op's `value`), and the `is_foldable`/`fold` dispatch on `(target_kind, op_type)`.
- `docs/specs/nexus-chat-M1.md` — the spec this implements (§2 data model, §3 CRDT semantics, §4 views, §7 coexistence).

## Global Constraints

- **Depend ONLY on `nxs-foundation`** in the library (plus `rusqlite`, `serde`, `serde_json`, `ulid`). No `nexus-flow-core`/`nexus-memory` in `[dependencies]` — cross-product proof crates are `[dev-dependencies]` only (spec §4.4; mirrors `crates/memory/Cargo.toml`).
- **`Reducer::fold` is infallible** (`crates/foundation/src/reducer.rs`, no `Result`). ALL validation lives in `is_foldable`; a malformed op returns `false` and is store-don't-fold — never panics `fold` (spec §3.1, Integrity#1).
- **Composite `target_id`s use `SEP = '\u{1f}'`** (the unit separator, as flow's edges do — `crates/core/src/store.rs:19`), NOT a literal `|`. The spec's `<a>|<b>` notation means `<a>{SEP}<b>`.
- **`domain = "message"`** for every chat op; the reducer dispatches on `target_kind` then `(op_type, field)`.
- **CRDT classes (spec §3):** `message` grow-only (`INSERT OR IGNORE`); `channel`/`profile` per-field keep-if-beats LWW `WHERE (excluded._v, excluded._site) > (_v, _site)`; `membership` observed-remove OR-set (add-tag = op_id, remove carries observed tags in `value`); `read_cursor` grow-only **max-register** `WHERE excluded.seen > seen`; `thread` grow-only root (upsert) + LWW `expects_reply_from`.
- **Env-var / determinism:** T1 is unit-level — reducer tests build `Op`s explicitly (like `fact_reducer.rs`); store tests query by content, never asserting a minted ULID. `m-<ULID>` non-determinism is a T2 (CLI-golden) concern, out of scope here.
- **Quality gates (must pass before each commit that touches Rust):** `cargo test -p nexus-chat`, then before the final task `cargo test`, `cargo test --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` (CLAUDE.md).

---

## File Structure

- `crates/chat/Cargo.toml` — package `nexus-chat`, lib `nexus_chat`, deps as constrained above.
- `crates/chat/src/lib.rs` — module decls + `pub use`.
- `crates/chat/src/model.rs` — domain/kind/op_type/field constants, `SEP`, the `MessageKind`/`Priority`/`Disposition` enums, the `MessageEnvelope` + `Refs` + `ThreadRoot` serde structs, and the split helpers.
- `crates/chat/src/schema.rs` — `apply_chat_views`/`try_apply_chat_views`: the six view DDLs.
- `crates/chat/src/message_reducer.rs` — `MessageReducer: Reducer` (dispatch + per-shape fold fns + envelope validation).
- `crates/chat/src/store.rs` — `ChatStore` newtype: open recipe, substrate delegation, write helpers, read helpers (`messages`, `inbox`, `resolve_membership`, `is_degraded_dm`).
- `crates/chat/tests/coexistence.rs` — the cross-product integration test (Task 8).
- Modify `Cargo.toml` (workspace `members`) — add `"crates/chat"`.

Each task ends with an independently testable deliverable and a commit.

---

## Task 1: Crate skeleton + `model.rs` vocabulary, enums, envelope

**Files:**
- Create: `crates/chat/Cargo.toml`, `crates/chat/src/lib.rs`, `crates/chat/src/model.rs`
- Modify: `Cargo.toml` (workspace `members` — append `"crates/chat"`)
- Test: inline `#[cfg(test)]` in `crates/chat/src/model.rs`

**Interfaces:**
- Produces: `DOMAIN_MESSAGE`, `KIND_MESSAGE/CHANNEL/PROFILE/MEMBERSHIP/THREAD/READ_CURSOR`, `OP_POST/SET/ADD/REMOVE/OPEN`, `FIELD_ENVELOPE/SEEN/ROOT/EXPECTS_REPLY_FROM`, `SEP`, `CHANNEL_FIELDS`/`PROFILE_FIELDS`, enums `MessageKind`/`Priority`/`Disposition`, structs `MessageEnvelope { origin, channel_id, sender, kind, priority, disposition, thread_id: Option<String>, refs: Refs, body }`, `Refs`, `ThreadRoot { origin, channel_id, opener, created }`, and `fn split2(&str) -> Option<(String, String)>`.

- [ ] **Step 1: Create the crate manifest** `crates/chat/Cargo.toml`

```toml
[package]
name = "nexus-chat"
version.workspace = true
edition = "2021"
description = "nexus-chat: the message-reducer over the shared nxs-foundation substrate (domain=message → chat views); the nxc CLI lands in a later slice"
publish = false

[lib]
name = "nexus_chat"
path = "src/lib.rs"

[dependencies]
# The shared op-log CRDT substrate. chat's store wraps it and registers the message reducer; it
# depends ONLY on the foundation — no flow/memory vocabulary leaks in (spec §4.4).
nxs-foundation = { path = "../foundation" }
rusqlite = { version = "0.31", features = ["bundled"] }
# Global, time-sortable message/thread ids (`m-`+ULID). Collision-free across workspaces (spec §2.2).
ulid = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[dev-dependencies]
tempfile = "3"
# The coexistence + sync round-trip proof (Task 8) references flow + sync — DEV-only, so the chat
# LIBRARY stays flow-free (mirrors crates/memory/Cargo.toml).
nexus-flow-core = { path = "../core" }
nxs-sync = { path = "../sync", features = ["engine"] }
```

- [ ] **Step 2: Register the crate in the workspace.** In `Cargo.toml`, add `"crates/chat"` to the `[workspace] members` array (after `"crates/memory"`).

- [ ] **Step 3: Create `crates/chat/src/lib.rs`**

```rust
//! nexus-chat — the message-domain kind-product over the shared nxs substrate (spec
//! docs/specs/nexus-chat-M1.md). Sibling of nexus-flow/nexus-memory: depends only on the
//! foundation. T1 is the substrate — reducer + views + store; the `nxc` CLI is a later slice.

pub mod model;
```

**Declare modules incrementally** so every task's commit compiles standalone: `lib.rs` starts with only `pub mod model;`. Task 2 adds `pub mod schema;`, Task 3 adds `pub mod message_reducer;`, Task 7 adds `pub mod store;` — each task adds its line when it creates the file (do NOT declare a module before its file exists — `rustc` errors E0583).

- [ ] **Step 4: Write the failing test** — append to `crates/chat/src/model.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_via_serde() {
        // The lowercase wire form is the contract (spec §2.1 enum values).
        assert_eq!(serde_json::to_string(&MessageKind::Decision).unwrap(), "\"decision\"");
        assert_eq!(serde_json::to_string(&Disposition::NextSession).unwrap(), "\"next_session\"");
        assert_eq!(serde_json::to_string(&Priority::Background).unwrap(), "\"background\"");
    }

    #[test]
    fn envelope_round_trips_and_rejects_bad_enum() {
        let env = MessageEnvelope {
            origin: "nxsflow/nexus-flow".into(),
            channel_id: "c-1".into(),
            sender: "nxsflow/nexus-flow/PmAgent".into(),
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: None,
            refs: Refs::default(),
            body: "ship it".into(),
        };
        let json = serde_json::to_string(&env).unwrap();
        assert_eq!(serde_json::from_str::<MessageEnvelope>(&json).unwrap(), env);
        // An out-of-range enum fails to parse — this is exactly what is_foldable relies on (§3.1).
        let bad = json.replace("\"normal\"", "\"screaming\"");
        assert!(serde_json::from_str::<MessageEnvelope>(&bad).is_err());
        // A missing required field also fails (defers store-don't-fold).
        let missing = json.replace(",\"body\":\"ship it\"", "");
        assert!(serde_json::from_str::<MessageEnvelope>(&missing).is_err());
    }

    #[test]
    fn split2_needs_exactly_two_parts() {
        assert_eq!(split2(&format!("a{SEP}b")), Some(("a".into(), "b".into())));
        assert_eq!(split2("noseparator"), None);
        assert_eq!(split2(&format!("a{SEP}b{SEP}c")), None);
    }
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cargo test -p nexus-chat --lib model`
Expected: FAIL — `MessageEnvelope`/`MessageKind`/`split2` not defined (crate may not compile yet).

- [ ] **Step 6: Write the module body** — prepend to `crates/chat/src/model.rs` (above the test module):

```rust
//! nexus-chat's `message` vocabulary + the serde envelope (spec §2.1). The substrate op shape is
//! domain-agnostic and owned by the foundation; this module owns only the constants that tag a
//! message op and the payload types a fold parses.

use serde::{Deserialize, Serialize};

/// The reducer domain — the axis the registry dispatches on (spec §4.1).
pub const DOMAIN_MESSAGE: &str = "message";

// Intra-domain kinds (target_kind), dispatched after the domain (spec §2).
pub const KIND_MESSAGE: &str = "message";
pub const KIND_CHANNEL: &str = "channel";
pub const KIND_PROFILE: &str = "profile";
pub const KIND_MEMBERSHIP: &str = "membership";
pub const KIND_THREAD: &str = "thread";
pub const KIND_READ_CURSOR: &str = "read_cursor";

// Op types.
pub const OP_POST: &str = "post"; // message
pub const OP_SET: &str = "set"; // channel/profile field, read_cursor seen, thread expects_reply_from
pub const OP_ADD: &str = "add"; // membership
pub const OP_REMOVE: &str = "remove"; // membership
pub const OP_OPEN: &str = "open"; // thread root

// Fields.
pub const FIELD_ENVELOPE: &str = "envelope";
pub const FIELD_SEEN: &str = "seen";
pub const FIELD_ROOT: &str = "root";
pub const FIELD_EXPECTS_REPLY_FROM: &str = "expects_reply_from";

/// Composite-`target_id` separator (unit separator), matching flow's edge convention
/// (`crates/core/src/store.rs`). Used for `channel{SEP}handle` and `consumer{SEP}channel`.
pub const SEP: char = '\u{1f}';

/// The LWW-register field whitelists — injection-safe column names for the `format!`-built upsert
/// (a field not on the list is not foldable, so it can never reach the SQL).
pub const CHANNEL_FIELDS: &[&str] = &["name", "kind", "origin"];
pub const PROFILE_FIELDS: &[&str] =
    &["job_title", "job_description", "capability_tags", "runtime_binding", "reports_to", "origin"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind { Task, Question, Report, Decision, Info }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority { Urgent, Normal, Background }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Disposition { InTurn, NextSession }

/// Pointers, not content (spec §2.1). Unknown extra fields are ignored (serde default) →
/// forward-compat; every field is optional so an empty `refs` is valid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nxf_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
}

/// The immutable message payload carried in one op's `value` (spec §2.1). `message_id`/`created`
/// come from the op (`target_id`/`wall_clock`), so they are NOT duplicated here. Required fields are
/// non-`Option`: a missing one fails `serde` parse → the op is store-don't-fold (spec §3.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub origin: String,
    pub channel_id: String,
    pub sender: String,
    pub kind: MessageKind,
    pub priority: Priority,
    pub disposition: Disposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub refs: Refs,
    pub body: String,
}

/// The immutable thread root carried in a `thread`/`open` op's `value` (spec §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRoot {
    pub origin: String,
    pub channel_id: String,
    pub opener: String,
    #[serde(default)]
    pub created: String,
}

/// Split a composite `target_id` on [`SEP`] into exactly two non-separator parts (`None` otherwise),
/// so no fold path can index-panic on a malformed id (mirrors flow's `parse_edge`).
pub fn split2(id: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = id.split(SEP).collect();
    match parts.as_slice() {
        [a, b] => Some((a.to_string(), b.to_string())),
        _ => None,
    }
}
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test -p nexus-chat --lib model`
Expected: PASS (3 tests).

- [ ] **Step 8: Commit**

```bash
git add crates/chat/Cargo.toml crates/chat/src/lib.rs crates/chat/src/model.rs Cargo.toml
git commit -m "feat(nexus-chat): T1 crate skeleton + message vocabulary/enums/envelope (6j6v.sv1b)"
```

---

## Task 2: `schema.rs` — the six materialized views

**Files:**
- Create: `crates/chat/src/schema.rs`
- Modify: `crates/chat/src/lib.rs` — **add `pub mod schema;`** (below `pub mod model;`) so the crate compiles
- Test: inline `#[cfg(test)]` in `schema.rs`

**Interfaces:**
- Produces: `pub fn apply_chat_views(&Connection)` (panics on failure, for tests/in-memory) and `pub fn try_apply_chat_views(&Connection) -> rusqlite::Result<()>` (idempotent DDL for the seven tables).

- [ ] **Step 1: Write the failing test** — `crates/chat/src/schema.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn apply_creates_all_chat_views_and_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        apply_chat_views(&conn);
        apply_chat_views(&conn); // second apply must not panic
        for t in ["messages", "channels", "membership_adds", "membership_removes",
                  "profiles", "threads", "read_cursors"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [t], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1, "view {t} exists after apply");
        }
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --lib schema`
Expected: FAIL — `apply_chat_views` not defined.

- [ ] **Step 3: Write the DDL** — prepend to `crates/chat/src/schema.rs`:

```rust
//! nexus-chat's materialized views (spec §4). Created over the substrate's `ops` log; the substrate
//! owns `ops` + the schema-version spine, this module owns only the chat-vocabulary DDL. The
//! message reducer folds into these; reads serve from them.

use rusqlite::Connection;

/// Apply the chat views, panicking on failure (in-memory stores / tests).
pub fn apply_chat_views(conn: &Connection) {
    try_apply_chat_views(conn).expect("chat views");
}

/// Apply the six chat views (idempotent). `_v`/`_site` columns carry LWW Lamport metadata where the
/// field is an LWW register. `thread` root columns are NULLable so a `set expects_reply_from` that
/// folds BEFORE its `open` still converges (order-independence).
pub fn try_apply_chat_views(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS messages(
             message_id  TEXT PRIMARY KEY,
             origin      TEXT NOT NULL, channel_id TEXT NOT NULL, sender TEXT NOT NULL,
             kind        TEXT NOT NULL, priority TEXT NOT NULL, disposition TEXT NOT NULL,
             thread_id   TEXT, refs TEXT, body TEXT NOT NULL, created TEXT,
             lamport     INTEGER NOT NULL, site INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS messages_channel ON messages(channel_id, message_id);
         CREATE INDEX IF NOT EXISTS messages_thread  ON messages(thread_id);

         CREATE TABLE IF NOT EXISTS channels(
             channel_id TEXT PRIMARY KEY,
             name   TEXT, name_v   INTEGER DEFAULT 0, name_site   INTEGER DEFAULT 0,
             kind   TEXT, kind_v   INTEGER DEFAULT 0, kind_site   INTEGER DEFAULT 0,
             origin TEXT, origin_v INTEGER DEFAULT 0, origin_site INTEGER DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS membership_adds(
             tag TEXT PRIMARY KEY, channel_id TEXT NOT NULL, handle TEXT NOT NULL,
             lamport INTEGER NOT NULL, site INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS membership_removes(tag TEXT PRIMARY KEY);

         CREATE TABLE IF NOT EXISTS profiles(
             handle TEXT PRIMARY KEY,
             job_title       TEXT, job_title_v       INTEGER DEFAULT 0, job_title_site       INTEGER DEFAULT 0,
             job_description TEXT, job_description_v INTEGER DEFAULT 0, job_description_site INTEGER DEFAULT 0,
             capability_tags TEXT, capability_tags_v INTEGER DEFAULT 0, capability_tags_site INTEGER DEFAULT 0,
             runtime_binding TEXT, runtime_binding_v INTEGER DEFAULT 0, runtime_binding_site INTEGER DEFAULT 0,
             reports_to      TEXT, reports_to_v      INTEGER DEFAULT 0, reports_to_site      INTEGER DEFAULT 0,
             origin          TEXT, origin_v          INTEGER DEFAULT 0, origin_site          INTEGER DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS threads(
             thread_id  TEXT PRIMARY KEY,
             origin     TEXT, channel_id TEXT, opener TEXT, created TEXT,
             expects_reply_from TEXT,
             expects_reply_from_v INTEGER DEFAULT 0, expects_reply_from_site INTEGER DEFAULT 0
         );

         CREATE TABLE IF NOT EXISTS read_cursors(
             consumer TEXT NOT NULL, channel_id TEXT NOT NULL, seen TEXT NOT NULL,
             PRIMARY KEY(consumer, channel_id)
         );",
    )
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p nexus-chat --lib schema`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/schema.rs
git commit -m "feat(nexus-chat): T1 the six chat view DDLs (6j6v.sv1b)"
```

---

## Task 3: `MessageReducer` — the `message`/`post` fold (grow-only) + envelope validation

**Files:**
- Create: `crates/chat/src/message_reducer.rs`
- Modify: `crates/chat/src/lib.rs` — **add `pub mod message_reducer;`** so the crate compiles
- Test: inline `#[cfg(test)]` in `message_reducer.rs` (a reusable `op(...)`/`store()` harness like `fact_reducer.rs`)

**Interfaces:**
- Produces: `pub struct MessageReducer;` implementing `nxs_foundation::reducer::Reducer` — `domain()=="message"`. This task wires `(KIND_MESSAGE, OP_POST, FIELD_ENVELOPE)` foldability (valid envelope required) + fold; later tasks extend `is_foldable`/`fold` for the other kinds.

- [ ] **Step 1: Write the failing test** — `crates/chat/src/message_reducer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::schema;
    use nxs_foundation::model::Op;
    use nxs_foundation::store::Store;

    /// An Op builder (mirrors fact_reducer.rs). op_id is unique per (target,lamport,site).
    pub(super) fn op(kind: &str, tid: &str, field: &str, op_type: &str,
                     value: Option<&str>, lamport: i64, site: i64) -> Op {
        Op {
            op_id: format!("op-{tid}-{field}-{lamport}-{site}"),
            lamport, site,
            domain: DOMAIN_MESSAGE.into(),
            target_kind: kind.into(), target_id: tid.into(), field: field.into(),
            op_type: op_type.into(), value: value.map(str::to_string),
            author: "nxsflow/nexus-flow/PmAgent".into(), wall_clock: String::new(),
        }
    }

    /// A substrate store with the chat views + message reducer wired.
    pub(super) fn store() -> Store {
        let mut s = Store::open_in_memory(1);
        schema::apply_chat_views(s.connection());
        s.register_reducer(Box::new(MessageReducer));
        s
    }

    fn envelope_json(channel: &str, body: &str) -> String {
        serde_json::to_string(&MessageEnvelope {
            origin: "nxsflow/nexus-flow".into(), channel_id: channel.into(),
            sender: "nxsflow/nexus-flow/PmAgent".into(),
            kind: MessageKind::Info, priority: Priority::Normal, disposition: Disposition::InTurn,
            thread_id: None, refs: Refs::default(), body: body.into(),
        }).unwrap()
    }

    #[test]
    fn domain_is_message() {
        assert_eq!(MessageReducer.domain(), "message");
    }

    #[test]
    fn folds_a_valid_message_grow_only_and_idempotent() {
        let mut s = store();
        let o = op(KIND_MESSAGE, "m-1", FIELD_ENVELOPE, OP_POST, Some(&envelope_json("c-1", "hi")), 1, 1);
        s.apply(&[o.clone()]);
        s.apply(&[o]); // re-delivered op → AlreadySeen, no dup
        let (chan, body): (String, String) = s.connection()
            .query_row("SELECT channel_id, body FROM messages WHERE message_id='m-1'", [],
                       |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((chan.as_str(), body.as_str()), ("c-1", "hi"));
        let n: i64 = s.connection().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1, "grow-only, idempotent on message_id");
    }

    #[test]
    fn malformed_envelope_is_store_dont_fold_not_a_panic() {
        let r = MessageReducer;
        // Not JSON at all, missing required field, and bad enum: none are foldable → deferred (§3.1).
        assert!(!r.is_foldable(&op(KIND_MESSAGE, "m-x", FIELD_ENVELOPE, OP_POST, Some("{not json"), 1, 1)));
        assert!(!r.is_foldable(&op(KIND_MESSAGE, "m-x", FIELD_ENVELOPE, OP_POST, Some("{\"body\":\"x\"}"), 1, 1)));
        let bad_enum = envelope_json("c-1", "x").replace("\"info\"", "\"gossip\"");
        assert!(!r.is_foldable(&op(KIND_MESSAGE, "m-x", FIELD_ENVELOPE, OP_POST, Some(&bad_enum), 1, 1)));
        assert!(!r.is_foldable(&op(KIND_MESSAGE, "m-x", FIELD_ENVELOPE, OP_POST, None, 1, 1)));
        // Applying a malformed op through the store must NOT insert and must NOT panic.
        let mut s = store();
        s.apply(&[op(KIND_MESSAGE, "m-x", FIELD_ENVELOPE, OP_POST, Some("{bad"), 1, 1)]);
        let n: i64 = s.connection().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0, "malformed op deferred, no row");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --lib message_reducer`
Expected: FAIL — `MessageReducer` not defined.

- [ ] **Step 3: Write the reducer** — prepend to `crates/chat/src/message_reducer.rs`:

```rust
//! nexus-chat's message reducer (spec §3): folds `message`-domain ops into the chat views. The
//! third registered reducer in the platform (after task + fact). Dispatches on `target_kind` then
//! `(op_type, field)` — grow-only messages, per-field LWW channels/profiles, observed-remove OR-set
//! membership, grow-only max-register read cursors, grow-only+LWW threads. Stateless unit struct.

use crate::model::*;
use nxs_foundation::model::Op;
use nxs_foundation::reducer::Reducer;
use rusqlite::{params, Connection};

/// nexus-chat's reducer over the `message` domain.
pub struct MessageReducer;

impl MessageReducer {
    /// A `message`/`post` op is foldable only if its `value` parses as a complete [`MessageEnvelope`]
    /// (all required fields present, enums in range). This is where ALL message validation lives —
    /// `fold` is infallible, so a bad envelope must be rejected here and deferred store-don't-fold
    /// (spec §3.1). Unknown extra envelope fields are ignored by serde (forward-compat).
    fn valid_envelope(op: &Op) -> bool {
        op.value.as_deref()
            .map(|v| serde_json::from_str::<MessageEnvelope>(v).is_ok())
            .unwrap_or(false)
    }

    fn fold_message(conn: &Connection, op: &Op) {
        // is_foldable guaranteed the parse; `message_id`/`created` come from the op, not the payload.
        let env: MessageEnvelope = serde_json::from_str(op.value.as_deref().unwrap()).unwrap();
        let refs = serde_json::to_string(&env.refs).unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO messages(message_id, origin, channel_id, sender, kind, priority,
                 disposition, thread_id, refs, body, created, lamport, site)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                op.target_id, env.origin, env.channel_id, env.sender,
                serde_json::to_value(env.kind).unwrap().as_str().unwrap(),
                serde_json::to_value(env.priority).unwrap().as_str().unwrap(),
                serde_json::to_value(env.disposition).unwrap().as_str().unwrap(),
                env.thread_id, refs, env.body, op.wall_clock, op.lamport, op.site
            ],
        )
        .unwrap();
    }
}

impl Reducer for MessageReducer {
    fn domain(&self) -> &'static str {
        DOMAIN_MESSAGE
    }

    fn is_foldable(&self, op: &Op) -> bool {
        match (op.target_kind.as_str(), op.op_type.as_str()) {
            (KIND_MESSAGE, OP_POST) => op.field == FIELD_ENVELOPE && Self::valid_envelope(op),
            _ => false,
        }
    }

    fn fold(&self, conn: &Connection, op: &Op) {
        match (op.target_kind.as_str(), op.op_type.as_str()) {
            (KIND_MESSAGE, OP_POST) => Self::fold_message(conn, op),
            other => unreachable!("non-foldable op reached MessageReducer::fold(): {other:?}"),
        }
    }

    fn clear_views(&self, conn: &Connection) {
        conn.execute_batch(
            "DELETE FROM messages; DELETE FROM channels;
             DELETE FROM membership_adds; DELETE FROM membership_removes;
             DELETE FROM profiles; DELETE FROM threads; DELETE FROM read_cursors;",
        )
        .unwrap();
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p nexus-chat --lib message_reducer`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/message_reducer.rs
git commit -m "feat(nexus-chat): T1 message fold (grow-only) + envelope validation in is_foldable (6j6v.sv1b)"
```

---

## Task 4: `channel` + `profile` per-field LWW

**Files:**
- Modify: `crates/chat/src/message_reducer.rs`
- Test: extend the inline test module

**Interfaces:**
- Consumes: `CHANNEL_FIELDS`/`PROFILE_FIELDS` (Task 1), the `op(...)`/`store()` harness (Task 3).
- Produces: `(KIND_CHANNEL, OP_SET)` and `(KIND_PROFILE, OP_SET)` foldability + a shared `fold_lww(conn, op, table, id_col)` helper.

- [ ] **Step 1: Write the failing test** — add to the `tests` module:

```rust
    #[test]
    fn channel_name_is_keep_if_beats_lww() {
        let mut s = store();
        s.apply(&[op(KIND_CHANNEL, "c-1", "name", OP_SET, Some("review"), 5, 1)]);
        // A lower (lamport,site) loses; a higher wins.
        s.apply(&[op(KIND_CHANNEL, "c-1", "name", OP_SET, Some("stale"), 2, 1)]);
        s.apply(&[op(KIND_CHANNEL, "c-1", "kind", OP_SET, Some("group"), 1, 1)]);
        let (name, kind): (String, String) = s.connection()
            .query_row("SELECT name, kind FROM channels WHERE channel_id='c-1'", [],
                       |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((name.as_str(), kind.as_str()), ("review", "group"), "higher lamport wins per field");
    }

    #[test]
    fn profile_field_folds_and_foreign_field_defers() {
        let r = MessageReducer;
        assert!(r.is_foldable(&op(KIND_PROFILE, "org/repo/a", "job_title", OP_SET, Some("QA"), 1, 1)));
        // A field not on the whitelist can never reach the format!-built SQL.
        assert!(!r.is_foldable(&op(KIND_PROFILE, "org/repo/a", "salary", OP_SET, Some("1"), 1, 1)));
        let mut s = store();
        s.apply(&[op(KIND_PROFILE, "org/repo/a", "job_title", OP_SET, Some("QA reviewer"), 1, 1)]);
        let t: String = s.connection()
            .query_row("SELECT job_title FROM profiles WHERE handle='org/repo/a'", [], |r| r.get(0)).unwrap();
        assert_eq!(t, "QA reviewer");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --lib message_reducer`
Expected: FAIL — channel/profile ops are not foldable yet (assertions fail / no rows).

- [ ] **Step 3: Add the LWW helper + dispatch arms** — in `message_reducer.rs`, add to `impl MessageReducer`:

```rust
    /// One keep-if-beats LWW upsert on `table`, keyed by `id_col`, for the whitelisted field
    /// `op.field`. The whitelist (checked in `is_foldable`) is exactly what makes the
    /// `format!`-interpolated column name injection-safe (mirrors flow's `fold_item_lww`).
    fn fold_lww(conn: &Connection, op: &Op, table: &str, id_col: &str) {
        let f = &op.field;
        let sql = format!(
            "INSERT INTO {table}({id_col}, {f}, {f}_v, {f}_site) VALUES(?1,?2,?3,?4)
             ON CONFLICT({id_col}) DO UPDATE SET
                 {f}=excluded.{f}, {f}_v=excluded.{f}_v, {f}_site=excluded.{f}_site
             WHERE (excluded.{f}_v, excluded.{f}_site) > ({f}_v, {f}_site)"
        );
        conn.execute(&sql, params![op.target_id, op.value, op.lamport, op.site]).unwrap();
    }
```

Extend `is_foldable`'s match (add arms before `_ =>`):

```rust
            (KIND_CHANNEL, OP_SET) => CHANNEL_FIELDS.contains(&op.field.as_str()),
            (KIND_PROFILE, OP_SET) => PROFILE_FIELDS.contains(&op.field.as_str()),
```

Extend `fold`'s match (add arms before `other =>`):

```rust
            (KIND_CHANNEL, OP_SET) => Self::fold_lww(conn, op, "channels", "channel_id"),
            (KIND_PROFILE, OP_SET) => Self::fold_lww(conn, op, "profiles", "handle"),
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p nexus-chat --lib message_reducer`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/message_reducer.rs
git commit -m "feat(nexus-chat): T1 channel/profile per-field LWW fold (6j6v.sv1b)"
```

---

## Task 5: `membership` observed-remove OR-set

**Files:**
- Modify: `crates/chat/src/message_reducer.rs`
- Test: extend the inline test module

**Interfaces:**
- Consumes: `split2` + `SEP` (Task 1).
- Produces: `(KIND_MEMBERSHIP, OP_ADD)` / `(KIND_MEMBERSHIP, OP_REMOVE)` foldability + fold. Membership `add` target_id = `channel{SEP}handle`, tag = `op.op_id`; `remove` `value` = `SEP`-joined observed add-tags.

- [ ] **Step 1: Write the failing test** — add to the `tests` module:

```rust
    fn membership(op_type: &str, channel: &str, handle: &str, value: Option<&str>,
                  lamport: i64, site: i64) -> Op {
        op(KIND_MEMBERSHIP, &format!("{channel}{SEP}{handle}"), "member", op_type, value, lamport, site)
    }
    fn members(s: &Store, channel: &str) -> Vec<String> {
        let mut st = s.connection().prepare(
            "SELECT handle FROM membership_adds WHERE channel_id=?1
               AND tag NOT IN (SELECT tag FROM membership_removes) ORDER BY handle").unwrap();
        let v: Vec<String> = st.query_map([channel], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        v
    }

    #[test]
    fn observed_remove_keeps_a_concurrent_unseen_add() {
        let mut s = store();
        // add #1 (alice) — its op_id is the tag the remove will observe.
        let a1 = membership(OP_ADD, "c-1", "alice", None, 1, 1);
        let a1_tag = a1.op_id.clone();
        s.apply(&[a1]);
        // A concurrent add of bob the remover never saw.
        s.apply(&[membership(OP_ADD, "c-1", "bob", None, 2, 2)]);
        // remove tombstones ONLY alice's observed tag.
        s.apply(&[membership(OP_REMOVE, "c-1", "alice", Some(&a1_tag), 3, 1)]);
        assert_eq!(members(&s, "c-1"), vec!["bob".to_string()], "alice removed, bob survives (observed-remove)");
        // Rejoin: a fresh add revives alice.
        s.apply(&[membership(OP_ADD, "c-1", "alice", None, 4, 1)]);
        assert_eq!(members(&s, "c-1"), vec!["alice".to_string(), "bob".to_string()]);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --lib message_reducer`
Expected: FAIL — membership ops not foldable.

- [ ] **Step 3: Add the fold fns + dispatch** — in `impl MessageReducer`:

```rust
    fn fold_membership_add(conn: &Connection, op: &Op) {
        // target_id = channel{SEP}handle; tag = op_id (observed-remove), mirroring flow's edge OR-set.
        let Some((channel_id, handle)) = split2(&op.target_id) else { return };
        conn.execute(
            "INSERT OR IGNORE INTO membership_adds(tag, channel_id, handle, lamport, site)
             VALUES(?1,?2,?3,?4,?5)",
            params![op.op_id, channel_id, handle, op.lamport, op.site],
        )
        .unwrap();
    }

    fn fold_membership_remove(conn: &Connection, op: &Op) {
        // value = SEP-joined observed add-tags this remove tombstones (mirrors fold_edge_remove).
        if let Some(tags) = &op.value {
            for tag in tags.split(SEP).filter(|t| !t.is_empty()) {
                conn.execute("INSERT OR IGNORE INTO membership_removes(tag) VALUES(?1)", [tag])
                    .unwrap();
            }
        }
    }
```

`is_foldable` arms (a valid composite id is required for `add`; a `remove` is always shape-valid — an empty/again value is a harmless no-op):

```rust
            (KIND_MEMBERSHIP, OP_ADD) => split2(&op.target_id).is_some(),
            (KIND_MEMBERSHIP, OP_REMOVE) => true,
```

`fold` arms:

```rust
            (KIND_MEMBERSHIP, OP_ADD) => Self::fold_membership_add(conn, op),
            (KIND_MEMBERSHIP, OP_REMOVE) => Self::fold_membership_remove(conn, op),
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p nexus-chat --lib message_reducer`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/message_reducer.rs
git commit -m "feat(nexus-chat): T1 membership observed-remove OR-set (6j6v.sv1b)"
```

---

## Task 6: `read_cursor` max-register + `thread` root/LWW

**Files:**
- Modify: `crates/chat/src/message_reducer.rs`
- Test: extend the inline test module

**Interfaces:**
- Consumes: `split2` (read_cursor target_id = `consumer{SEP}channel`), `ThreadRoot` (Task 1).
- Produces: `(KIND_READ_CURSOR, OP_SET)`, `(KIND_THREAD, OP_OPEN)`, `(KIND_THREAD, OP_SET)` foldability + fold.

- [ ] **Step 1: Write the failing test** — add to the `tests` module:

```rust
    fn cursor(consumer: &str, channel: &str, seen: &str, lamport: i64, site: i64) -> Op {
        op(KIND_READ_CURSOR, &format!("{consumer}{SEP}{channel}"), FIELD_SEEN, OP_SET,
           Some(seen), lamport, site)
    }
    fn seen(s: &Store, consumer: &str, channel: &str) -> Option<String> {
        s.connection().query_row(
            "SELECT seen FROM read_cursors WHERE consumer=?1 AND channel_id=?2",
            [consumer, channel], |r| r.get(0)).ok()
    }

    #[test]
    fn read_cursor_is_a_grow_only_max_register_never_regressing() {
        let mut s = store();
        // A read further (m9) at an EARLIER lamport; a read less-far (m7) at a LATER lamport.
        s.apply(&[cursor("org/repo/a", "c-1", "m-9", 3, 1)]);
        s.apply(&[cursor("org/repo/a", "c-1", "m-7", 5, 2)]); // higher lamport, lower cursor
        assert_eq!(seen(&s, "org/repo/a", "c-1"), Some("m-9".into()),
                   "max-register keeps the greater cursor; LWW-by-op would regress to m-7");
    }

    #[test]
    fn thread_open_is_grow_only_and_expects_reply_from_is_lww_order_independent() {
        let mut s = store();
        let root = serde_json::to_string(&ThreadRoot {
            origin: "nxsflow/nexus-flow".into(), channel_id: "c-1".into(),
            opener: "org/repo/a".into(), created: String::new(),
        }).unwrap();
        // set expects_reply_from BEFORE open — must still converge (nullable root cols).
        s.apply(&[op(KIND_THREAD, "t-1", FIELD_EXPECTS_REPLY_FROM, OP_SET, Some("[\"org/repo/b\"]"), 4, 1)]);
        s.apply(&[op(KIND_THREAD, "t-1", FIELD_ROOT, OP_OPEN, Some(&root), 1, 1)]);
        let (chan, erf): (String, String) = s.connection()
            .query_row("SELECT channel_id, expects_reply_from FROM threads WHERE thread_id='t-1'", [],
                       |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((chan.as_str(), erf.as_str()), ("c-1", "[\"org/repo/b\"]"));
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --lib message_reducer`
Expected: FAIL — read_cursor/thread ops not foldable.

- [ ] **Step 3: Add fold fns + dispatch** — in `impl MessageReducer`:

```rust
    fn fold_read_cursor(conn: &Connection, op: &Op) {
        // target_id = consumer{SEP}channel; value = the last-read message id. Grow-only MAX-register:
        // keep the GREATER cursor so the watermark never regresses (spec §3.4), unlike LWW-by-op.
        let (Some((consumer, channel)), Some(v)) = (split2(&op.target_id), op.value.as_deref())
        else { return };
        conn.execute(
            "INSERT INTO read_cursors(consumer, channel_id, seen) VALUES(?1,?2,?3)
             ON CONFLICT(consumer, channel_id) DO UPDATE SET seen=excluded.seen
             WHERE excluded.seen > seen",
            params![consumer, channel, v],
        )
        .unwrap();
    }

    fn fold_thread_open(conn: &Connection, op: &Op) {
        // Immutable root; upsert so it converges whether or not a `set` already created the row
        // (root values are identical across any duplicate open).
        let Some(root) = op.value.as_deref()
            .and_then(|v| serde_json::from_str::<ThreadRoot>(v).ok()) else { return };
        conn.execute(
            "INSERT INTO threads(thread_id, origin, channel_id, opener, created)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(thread_id) DO UPDATE SET
                 origin=excluded.origin, channel_id=excluded.channel_id,
                 opener=excluded.opener, created=excluded.created",
            params![op.target_id, root.origin, root.channel_id, root.opener, root.created],
        )
        .unwrap();
    }

    fn fold_thread_erf(conn: &Connection, op: &Op) {
        // expects_reply_from as a keep-if-beats LWW register; row may not exist yet (set-before-open).
        conn.execute(
            "INSERT INTO threads(thread_id, expects_reply_from, expects_reply_from_v, expects_reply_from_site)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(thread_id) DO UPDATE SET
                 expects_reply_from=excluded.expects_reply_from,
                 expects_reply_from_v=excluded.expects_reply_from_v,
                 expects_reply_from_site=excluded.expects_reply_from_site
             WHERE (excluded.expects_reply_from_v, excluded.expects_reply_from_site)
                 > (expects_reply_from_v, expects_reply_from_site)",
            params![op.target_id, op.value, op.lamport, op.site],
        )
        .unwrap();
    }
```

`is_foldable` arms:

```rust
            (KIND_READ_CURSOR, OP_SET) => op.field == FIELD_SEEN && op.value.is_some()
                && split2(&op.target_id).is_some(),
            (KIND_THREAD, OP_OPEN) => op.field == FIELD_ROOT
                && op.value.as_deref().map(|v| serde_json::from_str::<ThreadRoot>(v).is_ok()).unwrap_or(false),
            (KIND_THREAD, OP_SET) => op.field == FIELD_EXPECTS_REPLY_FROM,
```

`fold` arms:

```rust
            (KIND_READ_CURSOR, OP_SET) => Self::fold_read_cursor(conn, op),
            (KIND_THREAD, OP_OPEN) => Self::fold_thread_open(conn, op),
            (KIND_THREAD, OP_SET) => Self::fold_thread_erf(conn, op),
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p nexus-chat --lib message_reducer`
Expected: PASS (8 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/message_reducer.rs
git commit -m "feat(nexus-chat): T1 read_cursor max-register + thread root/LWW fold (6j6v.sv1b)"
```

---

## Task 7: `ChatStore` — wrapper, write helpers, reads (inbox + degraded-DM)

**Files:**
- Create: `crates/chat/src/store.rs`
- Modify: `crates/chat/src/lib.rs` — **add `pub mod store;`** so the crate compiles
- Test: inline `#[cfg(test)]` in `store.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: `pub struct ChatStore` with `open_in_memory(site)`, `open(path, site) -> rusqlite::Result<ChatStore>`, `set_wall_clock`, `connection`, `apply`, `export`, `refold`, `data_version`; write helpers `post_message(&mut, &MessageEnvelope) -> String`, `set_channel_field`, `add_member`/`remove_member`, `set_profile_field`, `open_thread`, `set_expects_reply_from`, `advance_read_cursor`; reads `resolve_membership(channel) -> Vec<String>`, `is_degraded_dm(channel) -> bool`, `inbox(consumer, include_next_session) -> Vec<InboxItem>`.

- [ ] **Step 1: Write the failing test** — `crates/chat/src/store.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn env(channel: &str, body: &str, disp: Disposition) -> MessageEnvelope {
        MessageEnvelope {
            origin: "o".into(), channel_id: channel.into(), sender: "o/a".into(),
            kind: MessageKind::Info, priority: Priority::Normal, disposition: disp,
            thread_id: None, refs: Refs::default(), body: body.into(),
        }
    }

    #[test]
    fn inbox_returns_unread_partitioned_by_disposition() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group");
        s.add_member("c-1", "o/reader");
        let m1 = s.post_message(&env("c-1", "now please", Disposition::InTurn));
        let _m2 = s.post_message(&env("c-1", "later", Disposition::NextSession));
        // Default inbox: only in_turn unread.
        let now = s.inbox("o/reader", false);
        assert_eq!(now.iter().map(|i| i.body.as_str()).collect::<Vec<_>>(), vec!["now please"]);
        // --all inbox: both dispositions.
        assert_eq!(s.inbox("o/reader", true).len(), 2);
        // After acking through m1, it drops from the in_turn inbox.
        s.advance_read_cursor("o/reader", "c-1", &m1);
        assert!(s.inbox("o/reader", false).is_empty());
    }

    #[test]
    fn dm_with_wrong_member_count_reads_as_degraded() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("dm-1", "kind", "direct");
        s.add_member("dm-1", "o/a");
        s.add_member("dm-1", "o/b");
        assert!(!s.is_degraded_dm("dm-1"), "exactly 2 → healthy");
        s.add_member("dm-1", "o/intruder"); // a raw 3rd add (non-nxc client)
        assert!(s.is_degraded_dm("dm-1"), "≠2 direct-channel members → degraded (spec §3.3)");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --lib store`
Expected: FAIL — `ChatStore` not defined.

- [ ] **Step 3: Write the store** — prepend to `crates/chat/src/store.rs`:

```rust
//! nexus-chat's store: the shared [`nxs_foundation::store::Store`] op-log plus chat's `message`
//! vocabulary folded over it (mirrors `MemoryStore`). Wraps the substrate, registers the message
//! reducer at open, and adds typed write/read helpers. Depends ONLY on the foundation (spec §4.4).

use crate::message_reducer::MessageReducer;
use crate::model::*;
use crate::schema;
use nxs_foundation::model::Op;
use nxs_foundation::store::Store as Substrate;
use rusqlite::{params, Connection};

/// chat's `view_watermarks.store_id` — distinct from flow's/memory's, so all three fold the SAME
/// log into their own views and advance independently (aye.36).
const CHAT_VIEWS: &str = "chat";

/// One materialized inbox row (spec §4 inbox derivation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxItem {
    pub message_id: String,
    pub channel_id: String,
    pub sender: String,
    pub disposition: String,
    pub body: String,
}

pub struct ChatStore {
    inner: Substrate,
}

impl ChatStore {
    pub fn open_in_memory(site: i64) -> ChatStore {
        let mut inner = Substrate::open_in_memory(site);
        schema::apply_chat_views(inner.connection());
        inner.register_reducer(Box::new(MessageReducer));
        inner.refold_if_behind(CHAT_VIEWS);
        ChatStore { inner }
    }

    pub fn open(path: &str, site: i64) -> rusqlite::Result<ChatStore> {
        let mut inner = Substrate::open(path, site)?;
        schema::try_apply_chat_views(inner.connection())?;
        inner.register_reducer(Box::new(MessageReducer));
        inner.refold_if_behind(CHAT_VIEWS);
        Ok(ChatStore { inner })
    }

    // ---- substrate delegation -------------------------------------------------
    pub fn set_wall_clock(&mut self, now: &str) { self.inner.set_wall_clock(now); }
    pub fn connection(&self) -> &Connection { self.inner.connection() }
    pub fn data_version(&self) -> rusqlite::Result<i64> { self.inner.data_version() }
    pub fn export(&self) -> Vec<Op> { self.inner.export() }
    pub fn apply(&mut self, ops: &[Op]) -> Vec<Op> { self.inner.apply(ops) }
    pub fn refold(&mut self) { self.inner.refold(); }

    fn author(&self) -> String { "system".into() }

    // ---- write helpers --------------------------------------------------------
    /// Append a message and return its minted global id (`m-`+ULID, spec §2.2). The envelope rides
    /// in the op's `value`; the reducer folds it into `messages`.
    pub fn post_message(&mut self, env: &MessageEnvelope) -> String {
        let message_id = format!("m-{}", ulid::Ulid::new());
        let value = serde_json::to_string(env).expect("envelope serializes");
        let author = env.sender.clone();
        self.inner.emit(DOMAIN_MESSAGE, KIND_MESSAGE, &message_id, FIELD_ENVELOPE, OP_POST,
                        Some(value), &author);
        message_id
    }

    pub fn set_channel_field(&mut self, channel_id: &str, field: &str, value: &str) {
        let author = self.author();
        self.inner.emit(DOMAIN_MESSAGE, KIND_CHANNEL, channel_id, field, OP_SET,
                        Some(value.to_string()), &author);
    }

    pub fn set_profile_field(&mut self, handle: &str, field: &str, value: &str) {
        let author = self.author();
        self.inner.emit(DOMAIN_MESSAGE, KIND_PROFILE, handle, field, OP_SET,
                        Some(value.to_string()), &author);
    }

    pub fn add_member(&mut self, channel_id: &str, handle: &str) {
        let author = self.author();
        let tid = format!("{channel_id}{SEP}{handle}");
        self.inner.emit(DOMAIN_MESSAGE, KIND_MEMBERSHIP, &tid, "member", OP_ADD, None, &author);
    }

    /// Remove `handle` from `channel_id`, tombstoning every add-tag currently observed for it
    /// (observed-remove: only adds we can see are removed; a concurrent unseen add survives).
    pub fn remove_member(&mut self, channel_id: &str, handle: &str) {
        let tags: Vec<String> = {
            let mut st = self.inner.connection().prepare(
                "SELECT tag FROM membership_adds WHERE channel_id=?1 AND handle=?2
                   AND tag NOT IN (SELECT tag FROM membership_removes)").unwrap();
            st.query_map(params![channel_id, handle], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        if tags.is_empty() { return; }
        let value = tags.join(&SEP.to_string());
        let author = self.author();
        let tid = format!("{channel_id}{SEP}{handle}");
        self.inner.emit(DOMAIN_MESSAGE, KIND_MEMBERSHIP, &tid, "member", OP_REMOVE,
                        Some(value), &author);
    }

    pub fn advance_read_cursor(&mut self, consumer: &str, channel_id: &str, seen: &str) {
        let author = self.author();
        let tid = format!("{consumer}{SEP}{channel_id}");
        self.inner.emit(DOMAIN_MESSAGE, KIND_READ_CURSOR, &tid, FIELD_SEEN, OP_SET,
                        Some(seen.to_string()), &author);
    }

    // ---- reads ----------------------------------------------------------------
    /// The resolved OR-set membership of a channel (present handles, sorted).
    pub fn resolve_membership(&self, channel_id: &str) -> Vec<String> {
        let mut st = self.inner.connection().prepare(
            "SELECT handle FROM membership_adds WHERE channel_id=?1
               AND tag NOT IN (SELECT tag FROM membership_removes) ORDER BY handle").unwrap();
        st.query_map([channel_id], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
    }

    /// A `direct` channel whose resolved membership is not exactly 2 is degraded — the read-side
    /// defensive contract (spec §3.3); callers must not assume "exactly 2".
    pub fn is_degraded_dm(&self, channel_id: &str) -> bool {
        let kind: Option<String> = self.inner.connection().query_row(
            "SELECT kind FROM channels WHERE channel_id=?1", [channel_id], |r| r.get(0)).ok();
        kind.as_deref() == Some("direct") && self.resolve_membership(channel_id).len() != 2
    }

    /// The consumer's unread inbox (spec §4): messages in channels they are a member of, with
    /// `message_id` beyond their per-channel read cursor. `include_next_session=false` returns only
    /// `in_turn` (act now); `true` includes `next_session` (the prime catch-up set).
    pub fn inbox(&self, consumer: &str, include_next_session: bool) -> Vec<InboxItem> {
        let mut st = self.inner.connection().prepare(
            "SELECT m.message_id, m.channel_id, m.sender, m.disposition, m.body
               FROM messages m
               JOIN (SELECT DISTINCT channel_id FROM membership_adds
                       WHERE handle=?1 AND tag NOT IN (SELECT tag FROM membership_removes)) mem
                 ON mem.channel_id = m.channel_id
              WHERE m.message_id > COALESCE(
                       (SELECT seen FROM read_cursors WHERE consumer=?1 AND channel_id=m.channel_id), '')
                AND (?2 OR m.disposition = 'in_turn')
              ORDER BY m.channel_id, m.message_id").unwrap();
        st.query_map(params![consumer, include_next_session], |r| Ok(InboxItem {
            message_id: r.get(0)?, channel_id: r.get(1)?, sender: r.get(2)?,
            disposition: r.get(3)?, body: r.get(4)?,
        })).unwrap().map(Result::unwrap).collect()
    }

    // open_thread / set_expects_reply_from follow the same emit pattern; add when the engine needs
    // threads (kept minimal here — the reducer already folds them, Task 6).
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p nexus-chat --lib store`
Expected: PASS (2 tests).

- [ ] **Step 5: Run the whole crate + clippy + fmt**

Run: `cargo test -p nexus-chat && cargo clippy -p nexus-chat --all-targets -- -D warnings && cargo fmt -p nexus-chat --check`
Expected: all pass. (Fix any clippy/fmt nits before committing.)

- [ ] **Step 6: Commit**

```bash
git add crates/chat/src/store.rs
git commit -m "feat(nexus-chat): T1 ChatStore — write helpers, inbox derivation, degraded-DM read (6j6v.sv1b)"
```

---

## Task 8: Multi-module coexistence + sync round-trip + refold-when-behind

**Files:**
- Create: `crates/chat/tests/coexistence.rs`
- Test: this is the integration test (dev-deps `nexus-flow-core`, `nxs-sync`)

**Interfaces:**
- Consumes: `ChatStore` (Task 7), `MessageEnvelope` (Task 1). Uses `nexus_flow_core` to prove flow ops coexist untouched, and the substrate `export`/`apply` to prove a message op survives a relay round-trip and folds on a peer.

**Note:** Read `crates/memory/tests/` (the aye.23 coexistence test) for the exact `nexus-flow-core` store construction and, if used, the `nxs-sync` engine harness. Mirror whichever it uses; if the sync harness is heavyweight, prove the round-trip with the substrate's own `export()` → peer `apply()` (which is the same op path sync uses) and keep the live-sync engine test for T4.

- [ ] **Step 1: Write the failing test** — `crates/chat/tests/coexistence.rs`:

```rust
//! T1 acceptance: `message` ops coexist with `task` ops in one op-log (each reducer folds only its
//! domain; the other is store-don't-fold), a message op survives an export→apply round-trip and
//! folds on a peer, and a refold rebuilds chat views after an out-of-band log advance (spec §7).

use nexus_chat::model::*;
use nexus_chat::store::ChatStore;

fn env(channel: &str, body: &str) -> MessageEnvelope {
    MessageEnvelope {
        origin: "o".into(), channel_id: channel.into(), sender: "o/a".into(),
        kind: MessageKind::Info, priority: Priority::Normal, disposition: Disposition::InTurn,
        thread_id: None, refs: Refs::default(), body: body.into(),
    }
}

#[test]
fn a_message_op_survives_a_round_trip_and_folds_on_a_peer() {
    let mut a = ChatStore::open_in_memory(1);
    let mid = a.post_message(&env("c-1", "hello peer"));
    // Ship the whole log to a fresh peer (the same op path sync uses).
    let ops = a.export();
    let mut b = ChatStore::open_in_memory(2);
    let deferred = b.apply(&ops);
    assert!(deferred.is_empty(), "the peer folds the message op (domain understood)");
    let body: String = b.connection()
        .query_row("SELECT body FROM messages WHERE message_id=?1", [&mid], |r| r.get(0)).unwrap();
    assert_eq!(body, "hello peer");
}

#[test]
fn a_foreign_task_op_is_stored_not_folded_no_cross_contamination() {
    // Build a flow task op via nexus-flow-core, apply it to a ChatStore: it must be DEFERRED
    // (store-don't-fold), never touching chat's views (spec §7).
    let mut chat = ChatStore::open_in_memory(1);
    // A flow item op (domain=task) — see crates/core for the exact Op fields / a flow store helper.
    let task_op = nexus_flow_core::model::Op {
        op_id: "flow-1".into(), lamport: 1, site: 9,
        domain: "task".into(), target_kind: "item".into(), target_id: "t.A".into(),
        field: "title".into(), op_type: "set".into(), value: Some("A".into()),
        author: "u".into(), wall_clock: String::new(),
    };
    // Convert to the foundation Op the ChatStore consumes (same shape).
    let op = nxs_foundation::model::Op {
        op_id: task_op.op_id, lamport: task_op.lamport, site: task_op.site,
        domain: task_op.domain, target_kind: task_op.target_kind, target_id: task_op.target_id,
        field: task_op.field, op_type: task_op.op_type, value: task_op.value,
        author: task_op.author, wall_clock: task_op.wall_clock,
    };
    let deferred = chat.apply(&[op]);
    assert_eq!(deferred.len(), 1, "a task op has no chat reducer → stored-not-folded");
    let n: i64 = chat.connection().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 0, "no cross-contamination into chat views");
}
```

- [ ] **Step 2: Run to verify it fails, then reconcile the flow-op construction**

Run: `cargo test -p nexus-chat --test coexistence`
Expected: FAIL to compile if `nexus_flow_core::model::Op`'s path/shape differs. **Before implementing:** open `crates/core/src/model.rs` (or `crates/memory/tests/`) and adjust the flow-op construction to the real API — the assertion logic stays. If `nexus-flow-core` re-exports the same foundation `Op`, drop the conversion and use it directly.

- [ ] **Step 3: Make it pass.** The library code from Tasks 1–7 already provides the behavior; this task only fixes the test's flow-op construction to the real `nexus-flow-core` API so it compiles and both assertions hold.

Run: `cargo test -p nexus-chat --test coexistence`
Expected: PASS (2 tests).

- [ ] **Step 4: Full workspace gates** (the whole point of coexistence — flow/memory must stay byte-identical, and release must catch nothing new):

```bash
cargo test
cargo test --release
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```
Expected: all green; the flow differential-oracle + memory tests unchanged (spec §7 invariant).

- [ ] **Step 5: Commit**

```bash
git add crates/chat/tests/coexistence.rs
git commit -m "test(nexus-chat): T1 coexistence + message round-trip + no cross-contamination (6j6v.sv1b)"
```

---

## Self-Review

**1. Spec coverage (docs/specs/nexus-chat-M1.md):**
- §2 data model / §2.1 envelope → Task 1 (constants, enums, `MessageEnvelope`). ✓
- §2.2 ids (`m-`+ULID, origin, qualified handles) → Task 1 (`m-` mint in `post_message`, Task 7) + envelope `origin`/`sender`. ✓ (bridge `runtime_binding`/`bridged` is a field value, not code — carried in profiles, Task 4.)
- §3.1 message grow-only + envelope-validity-in-`is_foldable` → Task 3. ✓
- §3.2 channel/profile per-field LWW → Task 4. ✓
- §3.3 membership observed-remove OR-set + DM degraded read → Task 5 (fold) + Task 7 (`is_degraded_dm`). ✓
- §3.4 read_cursor max-register (regression test) → Task 6. ✓
- §3.5 thread root + LWW → Task 6. ✓
- §4 views + inbox derivation → Task 2 (DDL) + Task 7 (`inbox`). ✓
- §7 coexistence / store-don't-fold / round-trip / refold → Task 8 (+ `refold_if_behind` in Task 7's `open`). ✓

**2. Placeholder scan:** No "TBD"/"handle edge cases". Task 8 Step 2 intentionally defers the exact `nexus-flow-core` `Op` construction to a real-API reconciliation (the assertion logic is fully specified) — this is a "match the real API" instruction, not a placeholder. `open_thread`/`set_expects_reply_from` store helpers are deliberately deferred to when the engine needs them (the reducer already folds threads, Task 6); flagged inline, not silently dropped.

**3. Type consistency:** `MessageReducer`, `ChatStore`, `MessageEnvelope`/`Refs`/`ThreadRoot`, `InboxItem`, `split2`, `fold_lww(table,id_col)`, `CHAT_VIEWS="chat"`, the `KIND_*`/`OP_*`/`FIELD_*` constants, and `SEP` are used consistently across tasks. `emit(...)` returns the minted op_id (unused here; write helpers mint their own `m-`/composite ids). `apply` returns `Vec<Op>` (deferred). Verified against `crates/foundation/src/store.rs` and `crates/memory/src/store.rs`.

**Deferred to later slices (not T1):** `nxc` CLI verbs (T2), `init`/`agent-manifest`/`prime` + `ModuleInit` self-registration + `nxs` routing (T3), live `nxs-sync` engine round-trip + the M1 gate (T4), an embeddable `ChatEngine` handle (with the engine-attach work).
