# nexus-chat M1 · T2 — `nxc` messaging verbs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the `nxc` messaging CLI (`send`/`reply`/`inbox`/`read`/`channels`/`agents`/`search`) — a pure pull consumer of the T1 chat views — with byte-stable `--json`, deterministic output, and the `not_found`/`forbidden` failure paths (nxf ticket `6j6v.z263` / T2, spec `docs/specs/nexus-chat-M1.md` §2.1/§5.1/§6).

**Architecture:** `nxc` is an `argv[0]` persona of the one `nxs` multicall binary, exactly like `nxm`. Its CLI (`crates/chat/src/cli.rs`) is a thin, deterministic clap surface over the merged T1 `ChatStore`: each verb resolves a `.nxs/` workspace, opens the chat store, and appends an op or reads a view. `--json` everywhere is the agent contract; declared struct field order is the byte-stable contract. The verbs embed the engine directly (architecture decision `6j6v.fnn1`). Membership checks live in the CLI (the friendly `not_found`/`forbidden` message); the store stays pure.

**Tech Stack:** Rust, `clap 4` (derive+env), `serde`/`serde_json` (`--json` records), `sha2` (deterministic DM channel id), `time` (RFC3339 `created`), `ulid` (global ids). Store/reducer from the merged T1 `nexus-chat` crate; substrate + error + workspace from `nxs-foundation`.

**Reference implementations to mirror (read once before starting):**
- `crates/memory/src/cli.rs` — the `nxm` CLI: the clap tree, `run`/`run_from` multicall seam, `actor()`/`resolve_now()`/`open()` helpers, `--json` record printing, the structured-error dispatch.
- `crates/memory/src/error.rs` + `crates/memory/src/workspace.rs` — the CLI error `emit` + the `WorkspaceExt::open_*_store` recipe (WAL pragmas).
- `crates/memory/tests/verbs.rs` — the black-box `assert_cmd` verb-test pattern (`Command::cargo_bin`, seed workspace, assert JSON fields + exact `.stdout` bytes, assert error `kind`).
- `crates/chat/src/store.rs` — the merged T1 `ChatStore` (write helpers + `inbox`/`resolve_membership`/`is_degraded_dm` reads) this CLI drives.
- `crates/nxs/src/cli.rs` (`run`, persona match) + `crates/nxs/build.rs` (persona symlinks) — where the `nxc` routing arm + symlink land.
- `docs/specs/nexus-chat-M1.md` §2.1 (envelope), §5.1 (verb contract), §5.2 (routing note), §6 (pull-only delivery).

## Global Constraints

- **Scope is T2 only** (spec §10): the messaging verbs + the minimal `nxs` routing to make `nxc` runnable. The `ModuleInit` self-registration (`inventory::submit!`), the contract verbs `init`/`agent-manifest`/`prime`, and the changelog fragment are **T3** (`6j6v.rtgh`). The sync round-trip + the flow/memory byte-identity gate are **T4** (`6j6v.j25s`).
- **The verb contract is the spec (§5.1) — deviations are spec findings, never silent adjustments.** Two contract-driven decisions are already locked in for this plan (user-approved 2026-07-09):
  1. **Minimal `nxs` routing lands in T2** (persona symlink + `Some("nxc")`/`Some("chat")` routing arms + `nexus-chat` dep) — the 2-line routing arms that §10 nominally lists under T3 — because "byte-stable Goldens" is impossible without a runnable binary. The **`ModuleInit` self-registration + the contract verbs stay T3**; T2 does **not** add the `use nexus_chat as _;` inventory link line (there is no `inventory::submit!` yet).
  2. **`ErrorKind::Forbidden` is added** to the shared foundation error set (additive; only `as_str()` gains an arm). Failure mapping: **unknown channel → `not_found`; existing channel, caller is not a member → `forbidden`**.
- **`--json` byte-stable everywhere.** Deterministic field order (serde declaration order). Minted ids (`message`/`thread`/group-`channel` = `m-<ULID>`) are non-deterministic in production; under `nxs_foundation::workspace::deterministic_ids_enabled()` (env `NXF_DETERMINISTIC_IDS`) they mint a **deterministic, lexicographically-sortable** `m-<26-digit-seq>` so goldens are byte-stable. DM channel ids (`dm:` + `sha256`) are deterministic by construction.
- **Identity model (M1, spec §2.2).** `origin()` = env `NXC_ORIGIN`, else `"local"` (a stable per-workspace local identity; federation `org/repo` is `6j6v.kz8p`). `actor()` = env `NXC_ACTOR`, else `USER`, else `"nxc"` (mirrors `nxm`). The caller's **qualified handle** (default `--consumer` and message `sender`) is `format!("{}/{}", origin(), actor())`. Handles passed explicitly (`agents register <handle>`, `channels join <ch> <handle>`, `--consumer <handle>`) are used **verbatim**.
- **Bound `LIKE ?` only** for `search`/`agents search` — never string-interpolated (spec §5.1, review `3cz4`). Case-insensitive substring.
- **Global ids are never shortened** (guardrail): the CLI passes full `message_id`/`thread_id`/`channel_id` through unchanged.
- **Store stays pure**: the CLI resolves membership for the friendly error, then calls the store (which just emits the op). Actor is threaded **explicitly** into every write helper — the T1 `author="system"` placeholder is removed (review `T1 #179`).
- **Quality gates (must pass before each Rust commit):** `cargo test -p nexus-chat` (+ the whole-workspace `cargo test` for the `assert_cmd` verb tests, which need the `nxc` symlink the `nxs` build produces). Before the final commit: `cargo test`, `cargo test --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` (CLAUDE.md).
- **No release cut.** T2 carries **no** `changes/*.md` fragment; the PR gets the `skip-changelog` label (the nexus-chat changelog entry lands with T3, per spec decision). Note this in the PR body.

---

## File Structure

- Modify `crates/foundation/src/error.rs` — add `ErrorKind::Forbidden` + `forbidden()` constructor.
- Modify `crates/chat/src/store.rs` — thread `author: &str` through the write helpers (drop the `"system"` placeholder), deterministic minted-id helper, add CLI read helpers (`channel_exists`, `is_member`, `message_channel`, `thread_channel`, `list_member_channels`, `profiles`/`profile`/`search_profiles`, `search_messages`, `channel_kind`). Update the T1 store/reducer/coexistence tests to the new write-helper signatures.
- Create `crates/chat/src/error.rs` — the CLI error re-export + `emit` (mirror `memory/src/error.rs`).
- Create `crates/chat/src/workspace.rs` — `CHAT_MODULE`, `ChatWorkspaceExt::open_chat_store`, `chat_config` (mirror `memory/src/workspace.rs`).
- Create `crates/chat/src/model_cli.rs` — CLI-facing serde output structs + the `Refs`/enum arg parsing helpers (kept out of `model.rs` so the substrate model stays CLI-free). *(Or inline in `cli.rs`; this plan inlines them in `cli.rs` for locality.)*
- Create `crates/chat/src/cli.rs` — the `nxc` clap tree + verb bodies + `run`/`run_from`.
- Modify `crates/chat/src/lib.rs` — add `pub mod cli; pub mod error; pub mod workspace;` + `pub use cli::{run, run_from};`.
- Modify `crates/chat/Cargo.toml` — add `clap`, `sha2`, `time` deps; `assert_cmd` dev-dep (trycmd already unused in T2).
- Create `crates/chat/tests/verbs.rs` — the black-box `assert_cmd` verb suite (byte-stable `--json` + failure paths).
- Modify `crates/nxs/Cargo.toml` — add `nexus-chat = { path = "../chat" }`.
- Modify `crates/nxs/src/cli.rs` — `Some("nxc")` persona arm + `Some("chat")` umbrella arm.
- Modify `crates/nxs/build.rs` — add `"nxc"` to the persona symlink list.

Each task ends with an independently testable deliverable and a commit.

---

## Task 1: `ErrorKind::Forbidden` in the shared error set

**Files:**
- Modify: `crates/foundation/src/error.rs`
- Test: inline `#[cfg(test)]` in `crates/foundation/src/error.rs` (add if absent)

**Interfaces:**
- Produces: `ErrorKind::Forbidden` (string form `"forbidden"`), `NxfError::forbidden(msg)`. Consumed by the CLI (Task 5/6) for the non-member path.

- [ ] **Step 1: Write the failing test** — append (or add) a test module to `crates/foundation/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_kind_has_the_stable_string_form_and_constructor() {
        // §5.1 pairs not_found (unknown channel) with forbidden (existing channel, non-member).
        // The string form is the agent contract, so it must be exactly "forbidden".
        assert_eq!(ErrorKind::Forbidden.as_str(), "forbidden");
        let e = NxfError::forbidden("not a member of channel c-1");
        assert_eq!(e.kind, ErrorKind::Forbidden);
        assert_eq!(e.msg, "not a member of channel c-1");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nxs-foundation --lib error::`
Expected: FAIL — `ErrorKind::Forbidden` / `NxfError::forbidden` not defined.

- [ ] **Step 3: Add the variant + constructor.** In `crates/foundation/src/error.rs`:

Add `Forbidden` to the enum (after `NotFound`):

```rust
pub enum ErrorKind {
    NoWorkspace,
    NotFound,
    Forbidden,
    Validation,
    Cycle,
    Conflict,
    Io,
    Verification,
}
```

Add the `as_str` arm (after the `NotFound` arm):

```rust
            ErrorKind::Forbidden => "forbidden",
```

Add the constructor (after `not_found`):

```rust
    pub fn forbidden(msg: impl Into<String>) -> NxfError {
        NxfError::new(ErrorKind::Forbidden, msg)
    }
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p nxs-foundation --lib error::`
Expected: PASS.

- [ ] **Step 5: Confirm nothing else breaks.** `ErrorKind::as_str` is the only exhaustive match on the enum (verified: every other use is variant construction/comparison), so this is purely additive.

Run: `cargo build -p nxs-foundation && cargo clippy -p nxs-foundation --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/foundation/src/error.rs
git commit -m "feat(foundation): add ErrorKind::Forbidden for the chat non-member failure path (6j6v.z263)"
```

---

## Task 2: `ChatStore` — thread the real actor + deterministic ids + CLI read helpers

**Files:**
- Modify: `crates/chat/src/store.rs`
- Modify: `crates/chat/tests/coexistence.rs` (only the write-helper call sites — the T1 assertions are unchanged)
- Test: extend the inline `#[cfg(test)]` in `store.rs`

**Interfaces:**
- Consumes: the T1 store internals; `nxs_foundation::workspace::deterministic_ids_enabled`.
- Produces (write helpers, now actor-explicit): `post_message(&mut, &MessageEnvelope) -> String` (unchanged signature — author = `env.sender`); `set_channel_field(&mut, channel_id, field, value, author)`; `set_profile_field(&mut, handle, field, value, author)`; `add_member(&mut, channel_id, handle, author)`; `remove_member(&mut, channel_id, handle, author)`; `advance_read_cursor(&mut, consumer, channel_id, seen, author)`; `open_thread(&mut, thread_id, &ThreadRoot, author)`; `set_expects_reply_from(&mut, thread_id, value, author)`; plus `mint_id(prefix, count_table) -> String`.
- Produces (reads for the CLI): `channel_exists(&self, channel_id) -> bool`; `channel_kind(&self, channel_id) -> Option<String>`; `is_member(&self, channel_id, handle) -> bool`; `message_channel(&self, message_id) -> Option<(String, Option<String>)>` (channel_id, thread_id); `thread_channel(&self, thread_id) -> Option<String>`; `list_member_channels(&self, handle) -> Vec<ChannelRow>`; `profiles(&self) -> Vec<ProfileRow>`; `profile(&self, handle) -> Option<ProfileRow>`; `search_profiles(&self, q) -> Vec<ProfileRow>`; `search_messages(&self, handle, q) -> Vec<MessageHit>`. New public row structs `ChannelRow`, `ProfileRow`, `MessageHit`.

- [ ] **Step 1: Write the failing tests** — add to the `tests` module in `store.rs`:

```rust
    #[test]
    fn write_helpers_stamp_the_explicit_actor_not_system() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "name", "review", "local/alice");
        // The op's author is the caller, never the removed "system" placeholder (review T1 #179).
        let author: String = s.connection()
            .query_row("SELECT author FROM ops WHERE target_kind='channel' AND target_id='c-1' AND field='name'",
                       [], |r| r.get(0)).unwrap();
        assert_eq!(author, "local/alice");
    }

    #[test]
    fn deterministic_ids_are_sortable_and_stable_under_the_switch() {
        // Golden determinism (spec §2.2): under NXF_DETERMINISTIC_IDS, a minted message id is a
        // zero-padded, lexicographically-sortable m-<seq> so send/read/inbox goldens are byte-stable.
        std::env::set_var("NXF_DETERMINISTIC_IDS", "1");
        let mut s = ChatStore::open_in_memory(1);
        let m1 = s.post_message(&env("c-1", "one", Disposition::InTurn));
        let m2 = s.post_message(&env("c-1", "two", Disposition::InTurn));
        std::env::remove_var("NXF_DETERMINISTIC_IDS");
        assert_eq!(m1, "m-00000000000000000000000001");
        assert_eq!(m2, "m-00000000000000000000000002");
        assert!(m1 < m2, "insertion order == lexicographic order == time order");
    }

    #[test]
    fn channel_and_membership_reads_back_the_cli_predicates() {
        let mut s = ChatStore::open_in_memory(1);
        assert!(!s.channel_exists("c-1"), "unknown channel");
        s.set_channel_field("c-1", "kind", "group", "local/alice");
        assert!(s.channel_exists("c-1"), "created channel exists");
        assert!(!s.is_member("c-1", "local/bob"), "not joined yet");
        s.add_member("c-1", "local/bob", "local/alice");
        assert!(s.is_member("c-1", "local/bob"), "joined");
    }

    #[test]
    fn message_and_thread_channel_lookups_resolve_reply_targets() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_channel_field("c-1", "kind", "group", "local/a");
        let mid = s.post_message(&MessageEnvelope {
            origin: "local".into(), channel_id: "c-1".into(), sender: "local/a".into(),
            kind: MessageKind::Question, priority: Priority::Normal, disposition: Disposition::InTurn,
            thread_id: Some("m-thread".into()), refs: Refs::default(), body: "q?".into(),
        });
        assert_eq!(s.message_channel(&mid), Some(("c-1".into(), Some("m-thread".into()))));
        assert_eq!(s.message_channel("m-nope"), None);
        let root = ThreadRoot { origin: "local".into(), channel_id: "c-9".into(),
                                opener: "local/a".into(), created: String::new() };
        s.open_thread("m-thread", &root, "local/a");
        assert_eq!(s.thread_channel("m-thread"), Some("c-9".into()));
    }

    #[test]
    fn profile_and_message_search_use_substring_over_the_right_columns() {
        let mut s = ChatStore::open_in_memory(1);
        s.set_profile_field("local/qa", "job_title", "QA Reviewer", "local/qa");
        s.set_profile_field("local/qa", "job_description", "runs the race-flag suite", "local/qa");
        s.set_profile_field("local/dev", "job_title", "Backend Dev", "local/dev");
        // case-insensitive substring over job_title AND job_description
        let hits = s.search_profiles("reviewer");
        assert_eq!(hits.iter().map(|p| p.handle.as_str()).collect::<Vec<_>>(), vec!["local/qa"]);
        assert_eq!(s.search_profiles("RACE").len(), 1, "matches job_description too");

        s.set_channel_field("c-1", "kind", "group", "local/qa");
        s.add_member("c-1", "local/qa", "local/qa");
        s.post_message(&env("c-1", "ship the release now", Disposition::InTurn));
        let mhits = s.search_messages("local/qa", "RELEASE");
        assert_eq!(mhits.len(), 1, "substring over body, only the caller's channels");
        assert_eq!(s.search_messages("local/outsider", "release").len(), 0, "not a member → no hits");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --lib store`
Expected: FAIL — new signatures / helpers not defined (crate may not compile).

- [ ] **Step 3: Thread the actor + add the id helper.** In `crates/chat/src/store.rs`:

Delete the `fn author(&self) -> String { "system".into() }` placeholder entirely.

Change `post_message` to mint via `mint_id` (author stays `env.sender`):

```rust
    /// Append a message and return its minted global id (`m-`+ULID, spec §2.2; deterministic under
    /// the golden switch). The envelope rides in the op's `value`; the reducer folds it into
    /// `messages`. The op author IS the message sender (the acting agent's qualified handle).
    pub fn post_message(&mut self, env: &MessageEnvelope) -> String {
        let message_id = self.mint_id("m-", "messages");
        let value = serde_json::to_string(env).expect("envelope serializes");
        self.inner.emit(DOMAIN_MESSAGE, KIND_MESSAGE, &message_id, FIELD_ENVELOPE, OP_POST,
                        Some(value), &env.sender);
        message_id
    }

    /// Mint a chat id (spec §2.2). Production: `<prefix>` + a fresh ULID (global, collision-free).
    /// Under the golden-docs determinism switch: `<prefix>` + a 26-digit zero-padded, lexicographically
    /// sortable sequence (one past the current row count of `count_table`), so send/channel/thread
    /// goldens are byte-stable and still time-sortable. Test-only seam — production never sets the flag.
    fn mint_id(&self, prefix: &str, count_table: &str) -> String {
        if nxs_foundation::workspace::deterministic_ids_enabled() {
            let n: i64 = self.inner.connection()
                .query_row(&format!("SELECT COUNT(*) FROM {count_table}"), [], |r| r.get(0))
                .unwrap_or(0);
            return format!("{prefix}{:026}", n + 1);
        }
        format!("{prefix}{}", ulid::Ulid::new())
    }
```

> **Note for the CLI (Task 5):** group-channel and thread minting reuse `mint_id` — expose a thin `pub fn mint_channel_id(&self) -> String { self.mint_id("m-", "channels") }` and `pub fn mint_thread_id(&self) -> String { self.mint_id("m-", "threads") }` here so the CLI mints ids without re-implementing the scheme. (`count_table` differs per kind; the shared numeric space is harmless — ids are opaque, kind-scoped, never compared across kinds.)

Add those two public minters right after `mint_id`:

```rust
    /// Mint a group-channel id (`m-`+ULID; deterministic under the golden switch). DM ids are NOT
    /// minted here — they are derived deterministically from the two handles (spec §3.2), computed by
    /// the CLI.
    pub fn mint_channel_id(&self) -> String { self.mint_id("m-", "channels") }
    /// Mint a thread id (`m-`+ULID; deterministic under the golden switch).
    pub fn mint_thread_id(&self) -> String { self.mint_id("m-", "threads") }
```

Add `author: &str` to each mutating helper and forward it to `emit` (replace the `let author = self.author();` lines). The bodies are otherwise unchanged; the new signatures are:

```rust
    pub fn set_channel_field(&mut self, channel_id: &str, field: &str, value: &str, author: &str) {
        assert!(CHANNEL_FIELDS.contains(&field), "unknown channel field: {field}");
        self.inner.emit(DOMAIN_MESSAGE, KIND_CHANNEL, channel_id, field, OP_SET,
                        Some(value.to_string()), author);
    }

    pub fn set_profile_field(&mut self, handle: &str, field: &str, value: &str, author: &str) {
        assert!(PROFILE_FIELDS.contains(&field), "unknown profile field: {field}");
        self.inner.emit(DOMAIN_MESSAGE, KIND_PROFILE, handle, field, OP_SET,
                        Some(value.to_string()), author);
    }

    pub fn add_member(&mut self, channel_id: &str, handle: &str, author: &str) {
        assert!(!channel_id.contains(SEP) && !handle.contains(SEP),
                "membership id must not contain the U+001F separator: {channel_id:?} / {handle:?}");
        let tid = format!("{channel_id}{SEP}{handle}");
        self.inner.emit(DOMAIN_MESSAGE, KIND_MEMBERSHIP, &tid, "member", OP_ADD, None, author);
    }

    pub fn remove_member(&mut self, channel_id: &str, handle: &str, author: &str) {
        assert!(!channel_id.contains(SEP) && !handle.contains(SEP),
                "membership id must not contain the U+001F separator: {channel_id:?} / {handle:?}");
        let tags: Vec<String> = {
            let mut st = self.inner.connection().prepare(
                "SELECT tag FROM membership_adds WHERE channel_id=?1 AND handle=?2
                   AND tag NOT IN (SELECT tag FROM membership_removes)").unwrap();
            st.query_map(params![channel_id, handle], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        if tags.is_empty() { return; }
        let value = tags.join(&SEP.to_string());
        let tid = format!("{channel_id}{SEP}{handle}");
        self.inner.emit(DOMAIN_MESSAGE, KIND_MEMBERSHIP, &tid, "member", OP_REMOVE, Some(value), author);
    }

    pub fn advance_read_cursor(&mut self, consumer: &str, channel_id: &str, seen: &str, author: &str) {
        assert!(!consumer.contains(SEP) && !channel_id.contains(SEP),
                "read cursor id must not contain the U+001F separator: {consumer:?} / {channel_id:?}");
        let tid = format!("{consumer}{SEP}{channel_id}");
        self.inner.emit(DOMAIN_MESSAGE, KIND_READ_CURSOR, &tid, FIELD_SEEN, OP_SET,
                        Some(seen.to_string()), author);
    }

    pub fn open_thread(&mut self, thread_id: &str, root: &ThreadRoot, author: &str) {
        let value = serde_json::to_string(root).expect("thread root serializes");
        self.inner.emit(DOMAIN_MESSAGE, KIND_THREAD, thread_id, FIELD_ROOT, OP_OPEN, Some(value), author);
    }

    pub fn set_expects_reply_from(&mut self, thread_id: &str, value: &str, author: &str) {
        self.inner.emit(DOMAIN_MESSAGE, KIND_THREAD, thread_id, FIELD_EXPECTS_REPLY_FROM, OP_SET,
                        Some(value.to_string()), author);
    }
```

- [ ] **Step 4: Add the CLI read helpers + row structs.** Still in `store.rs`, add public structs near `InboxItem`:

```rust
/// A channel as `nxc channels list` renders it (spec §5.1). Declared field order = the JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ChannelRow {
    pub channel_id: String,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub members: usize,
    pub degraded: bool,
}

/// A profile as `nxc agents list`/`register`/`search` render it (spec §3.2). Declared field order =
/// the JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProfileRow {
    pub handle: String,
    pub job_title: Option<String>,
    pub job_description: Option<String>,
    pub capability_tags: Option<String>,
    pub runtime_binding: Option<String>,
    pub reports_to: Option<String>,
    pub origin: Option<String>,
}

/// One `nxc search` hit (spec §5.1). Declared field order = the JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MessageHit {
    pub message_id: String,
    pub channel_id: String,
    pub sender: String,
    pub body: String,
}
```

Add the read methods to `impl ChatStore` (after `inbox`):

```rust
    /// A channel is "known" once it has a `channels` row OR any resolved member — either a
    /// `channels create`/`dm` (which sets fields) or a bare `join` makes it addressable. Unknown →
    /// the CLI's `not_found` (spec §5.1).
    pub fn channel_exists(&self, channel_id: &str) -> bool {
        let has_row: bool = self.inner.connection().query_row(
            "SELECT EXISTS(SELECT 1 FROM channels WHERE channel_id=?1)", [channel_id], |r| r.get(0)).unwrap();
        has_row || !self.resolve_membership(channel_id).is_empty()
    }

    /// The channel's declared `kind` (`direct`|`group`), or `None` if it has no `channels` row.
    pub fn channel_kind(&self, channel_id: &str) -> Option<String> {
        self.inner.connection().query_row(
            "SELECT kind FROM channels WHERE channel_id=?1", [channel_id], |r| r.get(0)).ok()
    }

    /// Whether `handle` is a resolved member of `channel_id` (OR-set present). Non-member of an
    /// EXISTING channel is the CLI's `forbidden` (spec §5.1).
    pub fn is_member(&self, channel_id: &str, handle: &str) -> bool {
        self.resolve_membership(channel_id).iter().any(|h| h == handle)
    }

    /// (channel_id, thread_id) of a message, for `reply` target resolution. `None` if unknown.
    pub fn message_channel(&self, message_id: &str) -> Option<(String, Option<String>)> {
        self.inner.connection().query_row(
            "SELECT channel_id, thread_id FROM messages WHERE message_id=?1",
            [message_id], |r| Ok((r.get(0)?, r.get(1)?))).ok()
    }

    /// The channel a thread was opened in, for `reply` target resolution. `None` if unknown.
    pub fn thread_channel(&self, thread_id: &str) -> Option<String> {
        self.inner.connection().query_row(
            "SELECT channel_id FROM threads WHERE thread_id=?1", [thread_id], |r| r.get(0)).ok()
    }

    /// Channels `handle` is a member of, with name/kind/member-count/degraded flag, channel-id sorted.
    pub fn list_member_channels(&self, handle: &str) -> Vec<ChannelRow> {
        let ids: Vec<String> = {
            let mut st = self.inner.connection().prepare(
                "SELECT DISTINCT channel_id FROM membership_adds
                   WHERE handle=?1 AND tag NOT IN (SELECT tag FROM membership_removes)
                   ORDER BY channel_id").unwrap();
            st.query_map([handle], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        ids.into_iter().map(|id| {
            let members = self.resolve_membership(&id);
            let (name, kind): (Option<String>, Option<String>) = self.inner.connection().query_row(
                "SELECT name, kind FROM channels WHERE channel_id=?1", [&id],
                |r| Ok((r.get(0)?, r.get(1)?))).unwrap_or((None, None));
            let degraded = kind.as_deref() == Some("direct") && members.len() != 2;
            ChannelRow { channel_id: id, name, kind, members: members.len(), degraded }
        }).collect()
    }

    /// All profiles, handle-sorted (deterministic).
    pub fn profiles(&self) -> Vec<ProfileRow> { self.query_profiles(None) }

    /// One profile by handle, or `None`.
    pub fn profile(&self, handle: &str) -> Option<ProfileRow> {
        self.query_profiles(Some(ProfileFilter::Handle(handle))).into_iter().next()
    }

    /// Profiles whose `job_title` OR `job_description` contains `q` (case-insensitive, bound `LIKE ?`
    /// — no interpolation; spec §5.1 / review 3cz4), handle-sorted.
    pub fn search_profiles(&self, q: &str) -> Vec<ProfileRow> {
        self.query_profiles(Some(ProfileFilter::Search(q)))
    }

    fn query_profiles(&self, filter: Option<ProfileFilter>) -> Vec<ProfileRow> {
        let base = "SELECT handle, job_title, job_description, capability_tags, runtime_binding,
                           reports_to, origin FROM profiles";
        let map = |r: &rusqlite::Row| Ok(ProfileRow {
            handle: r.get(0)?, job_title: r.get(1)?, job_description: r.get(2)?,
            capability_tags: r.get(3)?, runtime_binding: r.get(4)?, reports_to: r.get(5)?, origin: r.get(6)?,
        });
        let conn = self.inner.connection();
        match filter {
            None => {
                let mut st = conn.prepare(&format!("{base} ORDER BY handle")).unwrap();
                st.query_map([], map).unwrap().map(Result::unwrap).collect()
            }
            Some(ProfileFilter::Handle(h)) => {
                let mut st = conn.prepare(&format!("{base} WHERE handle=?1")).unwrap();
                st.query_map([h], map).unwrap().map(Result::unwrap).collect()
            }
            Some(ProfileFilter::Search(q)) => {
                // Bound LIKE with the wildcards added to the PARAMETER, never the SQL.
                let like = format!("%{}%", q.to_lowercase());
                let mut st = conn.prepare(&format!(
                    "{base} WHERE lower(COALESCE(job_title,'')) LIKE ?1
                              OR lower(COALESCE(job_description,'')) LIKE ?1 ORDER BY handle")).unwrap();
                st.query_map([&like], map).unwrap().map(Result::unwrap).collect()
            }
        }
    }

    /// Messages whose `body` contains `q` (case-insensitive, bound `LIKE ?`), restricted to `handle`'s
    /// channels, ordered by (channel_id, message_id) — deterministic (spec §5.1).
    pub fn search_messages(&self, handle: &str, q: &str) -> Vec<MessageHit> {
        let like = format!("%{}%", q.to_lowercase());
        let mut st = self.inner.connection().prepare(
            "SELECT m.message_id, m.channel_id, m.sender, m.body
               FROM messages m
               JOIN (SELECT DISTINCT channel_id FROM membership_adds
                       WHERE handle=?1 AND tag NOT IN (SELECT tag FROM membership_removes)) mem
                 ON mem.channel_id = m.channel_id
              WHERE lower(m.body) LIKE ?2
              ORDER BY m.channel_id, m.message_id").unwrap();
        st.query_map(params![handle, like], |r| Ok(MessageHit {
            message_id: r.get(0)?, channel_id: r.get(1)?, sender: r.get(2)?, body: r.get(3)?,
        })).unwrap().map(Result::unwrap).collect()
    }
```

Add the private filter enum near the top of `store.rs` (below the `use`s):

```rust
/// Internal discriminator for the shared profile query (list / by-handle / substring-search).
enum ProfileFilter<'a> { Handle(&'a str), Search(&'a str) }
```

- [ ] **Step 5: Fix the T1 in-crate tests to the new signatures.** In `store.rs`'s existing `tests` module, update every `set_channel_field`/`add_member`/`advance_read_cursor`/`open_thread`/`set_expects_reply_from` call to pass an author (use a literal like `"local/alice"` / `"o/a"`). E.g.:

- `s.set_channel_field("c-1", "kind", "group")` → `s.set_channel_field("c-1", "kind", "group", "o/a")`
- `s.add_member("c-1", "o/reader")` → `s.add_member("c-1", "o/reader", "o/a")`
- `s.advance_read_cursor("o/reader", "c-1", &m1)` → `s.advance_read_cursor("o/reader", "c-1", &m1, "o/reader")`
- `s.open_thread("t-1", &root)` → `s.open_thread("t-1", &root, "o/a")`
- `s.set_expects_reply_from("t-1", &erf)` → `s.set_expects_reply_from("t-1", &erf, "o/a")`
- The `#[should_panic]` field/SEP guard tests keep their intent — just add the trailing `author` arg (e.g. `s.set_channel_field("c-1", "kynd", "group", "o/a")`).

- [ ] **Step 6: Fix the T1 coexistence test.** In `crates/chat/tests/coexistence.rs`, any write-helper call gains the author arg (the `post_message` calls are unchanged; if the test uses `add_member`/`set_channel_field` etc., add the author). The assertions are unchanged.

- [ ] **Step 7: Run to verify it passes**

Run: `cargo test -p nexus-chat`
Expected: PASS — all T1 tests (with updated signatures) + the 5 new tests green.

- [ ] **Step 8: Commit**

```bash
git add crates/chat/src/store.rs crates/chat/tests/coexistence.rs
git commit -m "feat(nexus-chat): T2 thread the actor through ChatStore writes + deterministic ids + CLI reads (6j6v.z263)"
```

---

## Task 3: CLI scaffolding — deps, `error.rs`, `workspace.rs`, `nxs` routing

**Files:**
- Modify: `crates/chat/Cargo.toml`, `crates/chat/src/lib.rs`
- Create: `crates/chat/src/error.rs`, `crates/chat/src/workspace.rs`
- Modify: `crates/nxs/Cargo.toml`, `crates/nxs/src/cli.rs`, `crates/nxs/build.rs`
- Test: inline `#[cfg(test)]` in `workspace.rs`

**Interfaces:**
- Produces: `nexus_chat::error::{NxfError, Result, ErrorKind, emit}`; `nexus_chat::workspace::{CHAT_MODULE, ChatWorkspaceExt, chat_config, Workspace, ...}`; the `nxc`/`chat` routing arms in `nxs`. `run`/`run_from` are added in Task 4.

- [ ] **Step 1: Add the CLI deps** to `crates/chat/Cargo.toml`. Under `[dependencies]` (after `serde_json`):

```toml
# The `nxc` CLI surface: parser tree, `--json` records.
clap = { version = "4", features = ["derive", "env"] }
# Deterministic DM channel id: dm: + hex(sha256(sorted handles joined by \x00))[..24] (spec §3.2).
sha2 = "0.10"
# The `created` display timestamp (RFC3339); pinned via NXC_NOW for deterministic goldens.
time = { version = "0.3", features = ["formatting", "parsing"] }
```

Under `[dev-dependencies]` (after `tempfile`):

```toml
assert_cmd = "2"
```

- [ ] **Step 2: Create `crates/chat/src/error.rs`** (mirror `memory/src/error.rs` verbatim, chat-worded):

```rust
//! The structured error envelope for `nxc`, re-exported from the foundation (the same closed `kind`
//! + human `msg` shape every nxs consumer shares) plus the process-rendering [`emit`]. Rendering an
//! error to a process is presentation, so it stays CLI-side (mirrors flow's/memory's `error`).

pub use nxs_foundation::error::*;

/// Render `err` and return the process exit code. In `--json` mode the structured envelope
/// `{"error":{"kind":..,"msg":..}}` goes to stdout; otherwise the human message goes to stderr.
pub fn emit(err: &NxfError, json: bool) -> i32 {
    if json {
        let env = serde_json::json!({ "error": { "kind": err.kind.as_str(), "msg": err.msg } });
        println!("{env}");
    } else {
        eprintln!("error: {}", err.msg);
    }
    1
}
```

- [ ] **Step 3: Create `crates/chat/src/workspace.rs`** (mirror `memory/src/workspace.rs`):

```rust
//! Workspace resolution is foundation-owned (the `.nxs/` directory belongs to the platform, not
//! chat); this module re-exports it so `nexus_chat::workspace::*` paths resolve, and adds the chat
//! half: opening the *chat* store (registers the message reducer + creates the six views) and
//! building chat's foundation config. Mirrors flow's/memory's `WorkspaceExt`.

pub use nxs_foundation::workspace::*;

use crate::error::{NxfError, Result};
use crate::store::ChatStore;

/// chat's module key in the foundation config's `active_modules` (spec §7). The `nxc init` that
/// *activates* it is T3; T2 only needs the constant + store-open + a fresh-workspace config.
pub const CHAT_MODULE: &str = "chat";

/// Opens chat's [`ChatStore`] over a resolved [`Workspace`]. WAL mode lets parallel `nxc` processes
/// serialize their short write transactions instead of failing with "database is locked". Bring this
/// trait into scope to call `ws.open_chat_store()`.
pub trait ChatWorkspaceExt {
    fn open_chat_store(&self) -> Result<ChatStore>;
}

impl ChatWorkspaceExt for Workspace {
    fn open_chat_store(&self) -> Result<ChatStore> {
        let path = self.db_path_str()?;
        let store = ChatStore::open(&path, self.replica.site_id)
            .map_err(|e| NxfError::io(format!("opening chat store at {path}: {e}")))?;
        store
            .connection()
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| NxfError::io(format!("failed to set sqlite pragmas: {e}")))?;
        Ok(store)
    }
}

/// Build the foundation config for a FRESH chat-only workspace: register `chat` as the sole active
/// module. When chat joins an EXISTING workspace, `foundation::setup` loads that config unchanged
/// (the `nxc init` that registers chat via `activate_module` is T3); this seeds only a brand-new
/// `.nxs/` (used by the T2 verb tests to stand up a workspace without the T3 `init` verb).
pub fn chat_config() -> WorkspaceConfig {
    WorkspaceConfig { active_modules: vec![CHAT_MODULE.to_string()], ..Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_config_registers_only_chat() {
        assert_eq!(chat_config().active_modules, vec!["chat".to_string()]);
    }

    #[test]
    fn open_chat_store_materializes_views_over_a_fresh_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = setup(tmp.path(), &chat_config()).unwrap();
        let store = ws.open_chat_store().unwrap();
        // The six views exist after open (the store applied the DDL + registered the reducer).
        let n: i64 = store.connection().query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='messages'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }
}
```

- [ ] **Step 4: Wire the modules** in `crates/chat/src/lib.rs`. Add below the existing `pub mod` lines (keep `cli` last; it is created in Task 4 — add its line THEN):

```rust
pub mod error;
pub mod message_reducer;
pub mod model;
pub mod schema;
pub mod store;
pub mod workspace;
```

*(Do not add `pub mod cli;` yet — its file is created in Task 4. Adding the module line before the file exists is an `rustc` E0583.)*

- [ ] **Step 5: Add `nexus-chat` as an `nxs` dependency.** In `crates/nxs/Cargo.toml`, under `[dependencies]` (after `nexus-memory`):

```toml
# The chat product (nexus-chat-M1 T2): `nxs` routes the `nxc`/`chat` persona to it. The ModuleInit
# self-registration + the `use nexus_chat as _;` link line are T3 (this dep only enables routing).
nexus-chat = { path = "../chat" }
```

- [ ] **Step 6: Add the `nxc` persona symlink.** In `crates/nxs/build.rs`, extend the persona loop:

```rust
    for persona in ["nxf", "nxm", "nxc"] {
```

- [ ] **Step 7: Add the routing arms.** In `crates/nxs/src/cli.rs`, in `run()`:

In the `argv[0]` persona match (after the `nxm` arm):

```rust
        Some("nxc") => return nexus_chat::run_from(args),
```

In the umbrella `args.get(1)` token match (after the `memory` arm):

```rust
        Some("chat") => nexus_chat::run_from(synth_argv("nxc", &args[2..])),
```

Update the `run()` doc-comment's persona list to mention `nxc`/`chat` (one line, matching the existing `nxf`/`nxm` phrasing).

- [ ] **Step 8: Verify it compiles** (the routing arms reference `nexus_chat::run_from`, added in Task 4 — so this step's build will FAIL until Task 4 adds `run_from`; that is expected and fine for a WIP commit boundary). To keep this task's commit self-consistent, TEMPORARILY stub `run_from` now by adding it in Task 4's order — i.e. **do Task 4 Step 1–6 before building `nxs`.** Practically: run the chat-only checks here, defer the `nxs` build to Task 4.

Run: `cargo test -p nexus-chat --lib workspace && cargo clippy -p nexus-chat --all-targets -- -D warnings && cargo fmt -p nexus-chat --check`
Expected: PASS (workspace tests) + clean lint/fmt. (`nxs` is built in Task 4 once `run_from` exists.)

- [ ] **Step 9: Commit**

```bash
git add crates/chat/Cargo.toml crates/chat/src/lib.rs crates/chat/src/error.rs crates/chat/src/workspace.rs \
        crates/nxs/Cargo.toml crates/nxs/src/cli.rs crates/nxs/build.rs
git commit -m "feat(nexus-chat): T2 CLI scaffolding (error/workspace) + nxs nxc routing arm (6j6v.z263)"
```

---

## Task 4: CLI skeleton + `send` (the first runnable verb)

**Files:**
- Create: `crates/chat/src/cli.rs`
- Modify: `crates/chat/src/lib.rs` — add `pub mod cli;` + `pub use cli::{run, run_from};`
- Create: `crates/chat/tests/verbs.rs`

**Interfaces:**
- Consumes: the store write/read helpers (Task 2), `error`/`workspace` (Task 3), the `nxc` routing (Task 3).
- Produces: `pub fn run() -> ExitCode`, `pub fn run_from(args) -> ExitCode`, `pub fn command() -> clap::Command`; the `send` verb `{message_id, persisted:true}`.

- [ ] **Step 1: Write the failing test** — `crates/chat/tests/verbs.rs`:

```rust
//! Black-box integration tests for `nxc`'s messaging verbs (spec §5.1): send / reply / inbox / read
//! / channels / agents / search over the built binary. Each test stands up its own `.nxs/` chat
//! workspace via the foundation setup (the `nxc init` that would do this is T3), so the verbs are
//! exercised exactly as an agent would call them.
//!
//! Determinism: `NXF_DETERMINISTIC_IDS=1` (sortable `m-<seq>` message/channel ids), `NXC_ACTOR=alice`,
//! `NXC_ORIGIN=local`, and a pinned `NXC_NOW`, so `--json` is byte-stable. The env is set on the CHILD
//! process only (`.env`), never process-global, so parallel tests do not race.

use assert_cmd::Command;
use nexus_chat::workspace::{chat_config, setup};
use serde_json::Value;
use tempfile::TempDir;

/// A fresh chat workspace. Returns the temp dir (kept alive by the caller).
fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    setup(tmp.path(), &chat_config()).expect("seed chat workspace");
    tmp
}

fn nxc(tmp: &TempDir) -> Command {
    let mut c = Command::cargo_bin("nxc").expect("nxc binary (built via the nxs multicall symlink)");
    c.current_dir(tmp.path())
        .env("NXF_DETERMINISTIC_IDS", "1")
        .env("NXC_ACTOR", "alice")
        .env("NXC_ORIGIN", "local")
        .env("NXC_NOW", "2026-07-08T00:00:00Z");
    c
}

fn json_of(out: &[u8]) -> Value {
    serde_json::from_str(String::from_utf8_lossy(out).trim()).expect("valid json")
}

#[test]
fn send_returns_a_synchronous_persist_ack() {
    let tmp = workspace();
    // A group channel to post into (create + join in Task 5; here we bootstrap membership so send's
    // channel-existence check passes — send only requires the channel to EXIST, spec §5.1).
    nxc(&tmp).args(["channels", "create", "review"]).assert().success();
    // capture the channel id
    let out = nxc(&tmp).args(["--json", "channels", "create", "review2"]).assert().success();
    let cid = json_of(&out.get_output().stdout)["channel_id"].as_str().unwrap().to_string();

    let out = nxc(&tmp).args(["--json", "send", &cid, "ship it"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["persisted"], true);
    assert!(v["message_id"].as_str().unwrap().starts_with("m-"), "global m- id, not shortened");
}

#[test]
fn send_to_an_unknown_channel_is_not_found() {
    let tmp = workspace();
    let out = nxc(&tmp).args(["--json", "send", "c-ghost", "hi"]).assert().failure();
    assert_eq!(json_of(&out.get_output().stdout)["error"]["kind"], "not_found");
}
```

*(These two tests also depend on `channels create` from Task 5. To keep Task 4 independently runnable, write ONLY the `send_to_an_unknown_channel_is_not_found` test now — it needs no channel — and add `send_returns_a_synchronous_persist_ack` in Task 5 once `channels create` exists. The plan lists both here for the reader; implement the unknown-channel one first.)*

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --test verbs`
Expected: FAIL — no `nxc` command / crate does not expose the CLI yet.

- [ ] **Step 3: Write the CLI skeleton + `send`** — `crates/chat/src/cli.rs`:

```rust
//! nxc — the nexus-chat agent CLI (spec §5.1). A thin, deterministic pull consumer of the T1 chat
//! views: every verb resolves a `.nxs/` workspace, opens the [`ChatStore`], and appends an op or
//! reads a view. `--json` everywhere is the agent contract; declared struct field order is the
//! byte-stable contract. Membership checks (the friendly not_found/forbidden message) live here; the
//! store stays pure. The `init`/`agent-manifest`/`prime` contract verbs + the `inventory`
//! self-registration are T3.

use crate::error::{self, NxfError, Result};
use crate::model::*;
use crate::store::ChatStore;
use crate::workspace::{ChatWorkspaceExt, Workspace};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// nexus-chat agent CLI.
#[derive(Parser, Debug)]
#[command(name = "nxc", version, about = "nexus-chat agent CLI", long_about = None)]
struct Cli {
    /// Emit machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    json: bool,
    /// Use this sqlite db file directly instead of discovering `.nxs/`.
    #[arg(long, global = true, env = "NXC_DB")]
    db: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Post a message to a channel; returns a synchronous persist-ack the instant it is durably
    /// logged (`{message_id, persisted:true}`) — no wait for delivery (spec §5.1).
    Send {
        /// The channel id to post to.
        channel: String,
        /// The message body (Markdown, the distilled work product).
        body: String,
        #[arg(long, default_value = "info", value_parser = ["task", "question", "report", "decision", "info"])]
        kind: String,
        #[arg(long, default_value = "normal", value_parser = ["urgent", "normal", "background"])]
        priority: String,
        #[arg(long, default_value = "in_turn", value_parser = ["in_turn", "next_session"])]
        disposition: String,
        /// The thread this message belongs to (null = not threaded).
        #[arg(long)]
        thread: Option<String>,
        /// A ref pointer `k=v` (repeatable): `session_id`, `branch`, `pr`, or `nxf_ids` (repeatable).
        #[arg(long = "ref")]
        refs: Vec<String>,
    },
}

/// The fully-built clap command tree for `nxc`.
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}

/// Entry point for the `nxc` binary. [`run_from`] is the multicall seam the `nxs` umbrella routes
/// `nxc`/`nxs chat` through.
pub fn run() -> ExitCode {
    run_from(std::env::args_os())
}

/// Run the `nxc` surface over an explicit argv INCLUDING the program name at index 0 (multicall seam,
/// mirrors `nxm`).
pub fn run_from<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    match dispatch(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => ExitCode::from(error::emit(&e, cli.json) as u8),
    }
}

fn dispatch(cli: &Cli) -> Result<()> {
    let db = cli.db.as_deref();
    match &cli.command {
        Command::Send { channel, body, kind, priority, disposition, thread, refs } =>
            send(cli.json, db, channel, body, kind, priority, disposition, thread.as_deref(), refs),
    }
}

// ---- shared helpers --------------------------------------------------------

/// The acting agent's bare name: `NXC_ACTOR`, else `USER`, else `nxc` (mirrors `nxm`).
fn actor() -> String {
    std::env::var("NXC_ACTOR")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "nxc".to_string())
}

/// The minting workspace identity (spec §2.2): `NXC_ORIGIN`, else the stable local default `local`.
/// The org/repo-qualified form is the federation bridge (`6j6v.kz8p`); M1 only needs a stable field.
fn origin() -> String {
    std::env::var("NXC_ORIGIN").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "local".to_string())
}

/// The caller's qualified handle `<origin>/<agent>` — the default `--consumer` and message `sender`.
fn caller_handle() -> String {
    format!("{}/{}", origin(), actor())
}

/// The `created` display timestamp: a pinned `NXC_NOW` (golden determinism), else the system clock.
fn resolve_now() -> Result<String> {
    match std::env::var("NXC_NOW").ok().filter(|s| !s.is_empty()) {
        Some(s) => Ok(s),
        None => OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|e| NxfError::io(format!("formatting current time: {e}"))),
    }
}

fn cwd() -> Result<PathBuf> {
    std::env::current_dir().map_err(|e| NxfError::io(format!("reading current dir: {e}")))
}

/// Resolve the workspace and open chat's store over it.
fn open(db: Option<&str>) -> Result<ChatStore> {
    let ws = Workspace::resolve(db, &cwd()?)?;
    ws.open_chat_store()
}

/// Parse `--kind`/`--priority`/`--disposition` (already constrained by clap's `value_parser`) into
/// the model enums. Infallible in practice; a parse error is an internal `validation`.
fn parse_kind(s: &str) -> Result<MessageKind> { parse_enum(s) }
fn parse_priority(s: &str) -> Result<Priority> { parse_enum(s) }
fn parse_disposition(s: &str) -> Result<Disposition> { parse_enum(s) }

fn parse_enum<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(&format!("\"{s}\""))
        .map_err(|_| NxfError::validation(format!("invalid enum value: {s}")))
}

/// Build [`Refs`] from repeated `--ref k=v` (spec §2.1: pointers, not content). Known keys:
/// `session_id`, `branch`, `pr` (scalar), `nxf_ids`/`nxf_id` (repeatable). An unknown key is a
/// loud `validation` error (no silent drop).
fn parse_refs(pairs: &[String]) -> Result<Refs> {
    let mut refs = Refs::default();
    for p in pairs {
        let (k, v) = p.split_once('=').ok_or_else(|| {
            NxfError::validation(format!("--ref expects k=v, got: {p}"))
        })?;
        match k {
            "session_id" => refs.session_id = Some(v.to_string()),
            "branch" => refs.branch = Some(v.to_string()),
            "pr" => refs.pr = Some(v.to_string()),
            "nxf_ids" | "nxf_id" => refs.nxf_ids.push(v.to_string()),
            other => return Err(NxfError::validation(format!("unknown --ref key: {other}"))),
        }
    }
    Ok(refs)
}

// ---- send ------------------------------------------------------------------

/// `nxc send <channel> "<body>" […]`: append a message op and return the persist-ack (spec §5.1). A
/// send to an unknown channel is `not_found` (the CLI checks; the store stays pure).
#[allow(clippy::too_many_arguments)]
fn send(json: bool, db: Option<&str>, channel: &str, body: &str, kind: &str, priority: &str,
        disposition: &str, thread: Option<&str>, refs: &[String]) -> Result<()> {
    let mut store = open(db)?;
    if !store.channel_exists(channel) {
        return Err(NxfError::not_found(format!("no such channel: {channel}")));
    }
    store.set_wall_clock(&resolve_now()?);
    let env = MessageEnvelope {
        origin: origin(),
        channel_id: channel.to_string(),
        sender: caller_handle(),
        kind: parse_kind(kind)?,
        priority: parse_priority(priority)?,
        disposition: parse_disposition(disposition)?,
        thread_id: thread.map(str::to_string),
        refs: parse_refs(refs)?,
        body: body.to_string(),
    };
    let message_id = store.post_message(&env);
    if json {
        println!("{}", serde_json::json!({ "message_id": message_id, "persisted": true }));
    } else {
        println!("sent {message_id}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_refs_builds_scalars_and_repeated_nxf_ids() {
        let r = parse_refs(&["session_id=s1".into(), "nxf_ids=6j6v.a".into(), "nxf_ids=6j6v.b".into()]).unwrap();
        assert_eq!(r.session_id.as_deref(), Some("s1"));
        assert_eq!(r.nxf_ids, vec!["6j6v.a", "6j6v.b"]);
    }

    #[test]
    fn parse_refs_rejects_an_unknown_key() {
        assert!(parse_refs(&["mystery=x".into()]).is_err());
    }

    #[test]
    fn caller_handle_qualifies_the_actor_with_the_origin() {
        std::env::set_var("NXC_ORIGIN", "acme");
        std::env::set_var("NXC_ACTOR", "bob");
        assert_eq!(caller_handle(), "acme/bob");
        std::env::remove_var("NXC_ORIGIN");
        std::env::remove_var("NXC_ACTOR");
    }
}
```

- [ ] **Step 4: Wire `cli` into the lib** — in `crates/chat/src/lib.rs` add `pub mod cli;` (alphabetical) and, at the end, `pub use cli::{run, run_from};`.

- [ ] **Step 5: Build `nxs` (now that `run_from` exists)** — the routing arm from Task 3 resolves:

Run: `cargo build -p nxs`
Expected: clean (the `nxc` symlink is produced by `nxs`'s build script).

- [ ] **Step 6: Run the verb test** (full `cargo test` so the `nxc` symlink exists):

Run: `cargo test -p nexus-chat --test verbs -- send_to_an_unknown_channel` (after a `cargo build -p nxs`)
Expected: PASS. (If `cargo_bin("nxc")` cannot find the symlink, run `cargo build -p nxs` first — it creates `target/debug/nxc -> nxs`.)

- [ ] **Step 7: Commit**

```bash
git add crates/chat/src/cli.rs crates/chat/src/lib.rs crates/chat/tests/verbs.rs
git commit -m "feat(nexus-chat): T2 nxc CLI skeleton + send persist-ack (6j6v.z263)"
```

---

## Task 5: `channels` — list / create / dm / join / leave

**Files:**
- Modify: `crates/chat/src/cli.rs`, `crates/chat/tests/verbs.rs`

**Interfaces:**
- Consumes: `mint_channel_id`, `set_channel_field`, `add_member`/`remove_member`, `list_member_channels`, `channel_exists`, `is_degraded_dm` (Task 2), `sha2`.
- Produces: the `Channels` subcommand with `List`/`Create`/`Dm`/`Join`/`Leave`; the deterministic `dm_channel_id` helper.

- [ ] **Step 1: Write the failing tests** — add to `crates/chat/tests/verbs.rs`:

```rust
#[test]
fn create_a_group_channel_then_list_it() {
    let tmp = workspace();
    let out = nxc(&tmp).args(["--json", "channels", "create", "review"]).assert().success();
    let cid = json_of(&out.get_output().stdout)["channel_id"].as_str().unwrap().to_string();
    assert!(cid.starts_with("m-"), "group channel is a minted m- id");
    // the creator is auto-joined, so it lists for them
    let out = nxc(&tmp).args(["--json", "channels"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["channel_id"], cid);
    assert_eq!(v[0]["name"], "review");
    assert_eq!(v[0]["kind"], "group");
}

#[test]
fn dm_is_deterministic_and_exactly_two_members() {
    let tmp = workspace();
    let out = nxc(&tmp).args(["--json", "channels", "dm", "local/alice", "local/bob"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    let id = v["channel_id"].as_str().unwrap().to_string();
    assert!(id.starts_with("dm:"), "deterministic dm id");
    assert_eq!(v["kind"], "direct");
    // Order-independent: the reverse pair derives the SAME id (spec §3.2).
    let out2 = nxc(&tmp).args(["--json", "channels", "dm", "local/bob", "local/alice"]).assert().success();
    assert_eq!(json_of(&out2.get_output().stdout)["channel_id"], id);
    // Exactly-2 enforced on create: a self-DM is rejected.
    nxc(&tmp).args(["--json", "channels", "dm", "local/alice", "local/alice"]).assert().failure();
}

#[test]
fn join_and_leave_toggle_membership() {
    let tmp = workspace();
    let out = nxc(&tmp).args(["--json", "channels", "create", "review"]).assert().success();
    let cid = json_of(&out.get_output().stdout)["channel_id"].as_str().unwrap().to_string();
    nxc(&tmp).args(["channels", "join", &cid, "local/bob"]).assert().success();
    // bob now a member → search/inbox as bob would see the channel (proved in later tasks); here just
    // assert leave removes him.
    nxc(&tmp).args(["channels", "leave", &cid, "local/bob"]).assert().success();
    // idempotent: leaving again is not an error (nothing to remove)
    nxc(&tmp).args(["channels", "leave", &cid, "local/bob"]).assert().success();
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p nexus-chat --test verbs -- channels` (build `nxs` first)
Expected: FAIL — `channels` subcommand unknown.

- [ ] **Step 3: Add the `Channels` subcommand + `dm` id helper.** In `crates/chat/src/cli.rs`:

Add to `enum Command`:

```rust
    /// Channels: list the caller's channels, create a group channel, open a DM (exactly 2), or add/
    /// remove membership (OR-set, spec §3.3).
    Channels {
        #[command(subcommand)]
        action: Option<ChannelsAction>,
    },
```

Add the action enum (near `Command`):

```rust
#[derive(Subcommand, Debug)]
enum ChannelsAction {
    /// List the channels the caller is a member of (default when no action is given).
    List,
    /// Create a named group channel (a minted `m-` id); the creator is auto-joined.
    Create { name: String },
    /// Open a DM channel between exactly two handles (a deterministic `dm:` id, spec §3.2).
    Dm { handle_a: String, handle_b: String },
    /// Add `handle` to `channel` (membership OR-set add).
    Join { channel: String, handle: String },
    /// Remove `handle` from `channel` (observed-remove).
    Leave { channel: String, handle: String },
}
```

Add the dispatch arm in `dispatch`:

```rust
        Command::Channels { action } => channels(cli.json, db, action.as_ref()),
```

Add the verb body + dm-id helper:

```rust
use sha2::{Digest, Sha256};

/// The deterministic DM channel id (spec §3.2): `dm:` + first 24 hex chars of
/// `sha256(sorted(a,b) joined by \x00)`. Both agents derive the SAME id without a rendezvous.
fn dm_channel_id(a: &str, b: &str) -> String {
    let mut pair = [a, b];
    pair.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(pair[0].as_bytes());
    hasher.update([0u8]); // the \x00 separator
    hasher.update(pair[1].as_bytes());
    let hex = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("dm:{}", &hex[..24])
}

/// `nxc channels [list|create|dm|join|leave]` (spec §5.1).
fn channels(json: bool, db: Option<&str>, action: Option<&ChannelsAction>) -> Result<()> {
    let mut store = open(db)?;
    match action.unwrap_or(&ChannelsAction::List) {
        ChannelsAction::List => {
            let rows = store.list_member_channels(&caller_handle());
            if json {
                println!("{}", serde_json::to_string(&rows).expect("channels serialize"));
            } else if rows.is_empty() {
                println!("no channels");
            } else {
                for c in &rows {
                    println!("{}  {}  ({}{})", c.channel_id,
                             c.name.as_deref().unwrap_or(""), c.kind.as_deref().unwrap_or("?"),
                             if c.degraded { ", degraded" } else { "" });
                }
            }
        }
        ChannelsAction::Create { name } => {
            let author = caller_handle();
            let cid = store.mint_channel_id();
            store.set_channel_field(&cid, "name", name, &author);
            store.set_channel_field(&cid, "kind", "group", &author);
            store.set_channel_field(&cid, "origin", &origin(), &author);
            store.add_member(&cid, &author, &author); // creator auto-joins
            if json {
                println!("{}", serde_json::json!({ "channel_id": cid, "name": name, "kind": "group" }));
            } else {
                println!("created group channel {cid} ({name})");
            }
        }
        ChannelsAction::Dm { handle_a, handle_b } => {
            if handle_a == handle_b {
                return Err(NxfError::validation("a DM needs two DISTINCT handles"));
            }
            let author = caller_handle();
            let cid = dm_channel_id(handle_a, handle_b);
            store.set_channel_field(&cid, "kind", "direct", &author);
            store.set_channel_field(&cid, "origin", &origin(), &author);
            store.add_member(&cid, handle_a, &author);
            store.add_member(&cid, handle_b, &author);
            if json {
                println!("{}", serde_json::json!({
                    "channel_id": cid, "kind": "direct", "members": [handle_a, handle_b]
                }));
            } else {
                println!("opened DM {cid} between {handle_a} and {handle_b}");
            }
        }
        ChannelsAction::Join { channel, handle } => {
            let author = caller_handle();
            store.add_member(channel, handle, &author);
            if json {
                println!("{}", serde_json::json!({ "channel_id": channel, "handle": handle, "joined": true }));
            } else {
                println!("{handle} joined {channel}");
            }
        }
        ChannelsAction::Leave { channel, handle } => {
            let author = caller_handle();
            store.remove_member(channel, handle, &author);
            if json {
                println!("{}", serde_json::json!({ "channel_id": channel, "handle": handle, "joined": false }));
            } else {
                println!("{handle} left {channel}");
            }
        }
    }
    Ok(())
}
```

Add a unit test for the dm-id helper in `cli.rs`'s `tests`:

```rust
    #[test]
    fn dm_channel_id_is_order_independent_and_prefixed() {
        let a = dm_channel_id("local/alice", "local/bob");
        assert_eq!(a, dm_channel_id("local/bob", "local/alice"));
        assert!(a.starts_with("dm:"));
        assert_eq!(a.len(), "dm:".len() + 24);
    }
```

- [ ] **Step 4: Add the deferred `send` happy-path test** from Task 4 Step 1 (`send_returns_a_synchronous_persist_ack`) to `verbs.rs` now that `channels create` exists.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs && cargo test -p nexus-chat --lib cli`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/chat/src/cli.rs crates/chat/tests/verbs.rs
git commit -m "feat(nexus-chat): T2 nxc channels list/create/dm/join/leave (6j6v.z263)"
```

---

## Task 6: `inbox` + `read` — disposition partition + synced cursor + forbidden

**Files:**
- Modify: `crates/chat/src/cli.rs`, `crates/chat/tests/verbs.rs`

**Interfaces:**
- Consumes: `inbox` (Task 1 store), `channel_exists`/`is_member` (Task 2), `advance_read_cursor` (Task 2).
- Produces: the `Inbox` + `Read` verbs; the partitioned inbox output `{consumer, in_turn:[...], next_session:[...]}`.

- [ ] **Step 1: Write the failing tests** — add to `verbs.rs`:

```rust
#[test]
fn inbox_partitions_by_disposition_and_read_advances_the_cursor() {
    let tmp = workspace();
    let out = nxc(&tmp).args(["--json", "channels", "create", "review"]).assert().success();
    let cid = json_of(&out.get_output().stdout)["channel_id"].as_str().unwrap().to_string();
    // alice is auto-joined as creator. Post one in_turn + one next_session.
    let out = nxc(&tmp).args(["--json", "send", &cid, "now please", "--disposition", "in_turn"]).assert().success();
    let m1 = json_of(&out.get_output().stdout)["message_id"].as_str().unwrap().to_string();
    nxc(&tmp).args(["--json", "send", &cid, "later", "--disposition", "next_session"]).assert().success();

    // Default inbox: only in_turn.
    let out = nxc(&tmp).args(["--json", "inbox"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["in_turn"].as_array().unwrap().len(), 1);
    assert_eq!(v["in_turn"][0]["body"], "now please");
    assert_eq!(v["next_session"].as_array().unwrap().len(), 0, "next_session hidden by default");
    // --all: both dispositions.
    let out = nxc(&tmp).args(["--json", "inbox", "--all"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["in_turn"].as_array().unwrap().len(), 1);
    assert_eq!(v["next_session"].as_array().unwrap().len(), 1);

    // Inbox is a PURE read — it did not advance the cursor, so in_turn is still there.
    let out = nxc(&tmp).args(["--json", "inbox"]).assert().success();
    assert_eq!(json_of(&out.get_output().stdout)["in_turn"].as_array().unwrap().len(), 1);
    // read --through m1 advances the synced cursor → in_turn now empty.
    nxc(&tmp).args(["read", &cid, "--through", &m1]).assert().success();
    let out = nxc(&tmp).args(["--json", "inbox"]).assert().success();
    assert_eq!(json_of(&out.get_output().stdout)["in_turn"].as_array().unwrap().len(), 0);
}

#[test]
fn read_and_inbox_forbid_a_non_member_but_not_found_an_unknown_channel() {
    let tmp = workspace();
    let out = nxc(&tmp).args(["--json", "channels", "create", "review"]).assert().success();
    let cid = json_of(&out.get_output().stdout)["channel_id"].as_str().unwrap().to_string();
    // caller is alice (creator/member); read as a DIFFERENT consumer who is not a member → forbidden.
    let out = nxc(&tmp).args(["--json", "read", &cid, "--through", "m-x", "--consumer", "local/outsider"])
        .assert().failure();
    assert_eq!(json_of(&out.get_output().stdout)["error"]["kind"], "forbidden");
    // unknown channel → not_found.
    let out = nxc(&tmp).args(["--json", "read", "c-ghost", "--through", "m-x"]).assert().failure();
    assert_eq!(json_of(&out.get_output().stdout)["error"]["kind"], "not_found");
    // inbox --channel for a non-member → forbidden.
    let out = nxc(&tmp).args(["--json", "inbox", "--channel", &cid, "--consumer", "local/outsider"])
        .assert().failure();
    assert_eq!(json_of(&out.get_output().stdout)["error"]["kind"], "forbidden");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs -- inbox read_and`
Expected: FAIL — `inbox`/`read` unknown.

- [ ] **Step 3: Add the `Inbox` + `Read` verbs.** In `cli.rs`:

Add to `enum Command`:

```rust
    /// The caller's unread inbox (spec §5.1), partitioned by disposition: default returns `in_turn`
    /// (act now); `--all` includes `next_session`. A PURE read — it does NOT advance the cursor.
    Inbox {
        /// Read as this handle (default: the caller's own qualified handle).
        #[arg(long)]
        consumer: Option<String>,
        /// Include `next_session` messages too (the catch-up set).
        #[arg(long)]
        all: bool,
        /// Restrict to one channel (membership-checked).
        #[arg(long)]
        channel: Option<String>,
    },
    /// Advance the SYNCED read cursor to `--through` (emit a `read_cursor` op) — the explicit ack that
    /// makes delivery at-least-once (spec §3.4/§5.1).
    Read {
        /// The channel whose cursor to advance.
        channel: String,
        /// The last-read message id (the cursor watermark).
        #[arg(long)]
        through: String,
        /// Ack as this handle (default: the caller's own qualified handle).
        #[arg(long)]
        consumer: Option<String>,
    },
```

Dispatch arms:

```rust
        Command::Inbox { consumer, all, channel } =>
            inbox(cli.json, db, consumer.as_deref(), *all, channel.as_deref()),
        Command::Read { channel, through, consumer } =>
            read(cli.json, db, channel, through, consumer.as_deref()),
```

Verb bodies:

```rust
/// One inbox row as `--json` renders it (declared field order = contract). Built from the store's
/// `InboxItem`.
#[derive(serde::Serialize)]
struct InboxOut {
    message_id: String,
    channel_id: String,
    sender: String,
    disposition: String,
    body: String,
}

fn inbox(json: bool, db: Option<&str>, consumer: Option<&str>, all: bool, channel: Option<&str>) -> Result<()> {
    let store = open(db)?;
    let consumer = consumer.map(str::to_string).unwrap_or_else(caller_handle);
    // A --channel filter is membership-checked (unknown → not_found, non-member → forbidden).
    if let Some(ch) = channel {
        require_member(&store, ch, &consumer)?;
    }
    let items = store.inbox(&consumer, true); // pull both dispositions; partition below
    let (mut in_turn, mut next_session): (Vec<InboxOut>, Vec<InboxOut>) = (Vec::new(), Vec::new());
    for i in items {
        if let Some(ch) = channel { if i.channel_id != ch { continue; } }
        let out = InboxOut {
            message_id: i.message_id, channel_id: i.channel_id, sender: i.sender,
            disposition: i.disposition.clone(), body: i.body,
        };
        if out.disposition == "in_turn" { in_turn.push(out); }
        else if all { next_session.push(out); }
    }
    if json {
        println!("{}", serde_json::json!({
            "consumer": consumer, "in_turn": in_turn, "next_session": next_session
        }));
    } else {
        println!("inbox for {consumer}:");
        for i in &in_turn { println!("  [in_turn] {} {}", i.channel_id, i.body); }
        for i in &next_session { println!("  [next_session] {} {}", i.channel_id, i.body); }
        if in_turn.is_empty() && next_session.is_empty() { println!("  (empty)"); }
    }
    Ok(())
}

fn read(json: bool, db: Option<&str>, channel: &str, through: &str, consumer: Option<&str>) -> Result<()> {
    let mut store = open(db)?;
    let consumer = consumer.map(str::to_string).unwrap_or_else(caller_handle);
    require_member(&store, channel, &consumer)?;
    // The ack must name a message that actually lives in this channel — a fat-fingered id must not
    // silently roll the max-register cursor past unread messages (spec §3.4).
    match store.message_channel(through) {
        Some((ch, _)) if ch == channel => {}
        _ => return Err(NxfError::not_found(format!("no message {through} in channel {channel}"))),
    }
    store.advance_read_cursor(&consumer, channel, through, &consumer);
    if json {
        println!("{}", serde_json::json!({ "consumer": consumer, "channel_id": channel, "seen": through }));
    } else {
        println!("{consumer} read {channel} through {through}");
    }
    Ok(())
}

/// The shared membership gate (spec §5.1): unknown channel → `not_found`; existing but caller is not
/// a member → `forbidden` (mirrors flow's `require_item` ergonomic).
fn require_member(store: &ChatStore, channel: &str, handle: &str) -> Result<()> {
    if !store.channel_exists(channel) {
        return Err(NxfError::not_found(format!("no such channel: {channel}")));
    }
    if !store.is_member(channel, handle) {
        return Err(NxfError::forbidden(format!("{handle} is not a member of {channel}")));
    }
    Ok(())
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/cli.rs crates/chat/tests/verbs.rs
git commit -m "feat(nexus-chat): T2 nxc inbox (disposition partition) + read (synced cursor) + not_found/forbidden (6j6v.z263)"
```

---

## Task 7: `reply` — post into the resolved thread/channel

**Files:**
- Modify: `crates/chat/src/cli.rs`, `crates/chat/tests/verbs.rs`

**Interfaces:**
- Consumes: `message_channel`/`thread_channel`/`channel_exists` (Task 2), the `send` envelope build (Task 4).
- Produces: the `Reply` verb — resolve a `thread_id|message_id` to its channel + thread, then post carrying `thread_id`.

- [ ] **Step 1: Write the failing test** — add to `verbs.rs`:

```rust
#[test]
fn reply_posts_into_the_targets_channel_carrying_the_thread() {
    let tmp = workspace();
    let out = nxc(&tmp).args(["--json", "channels", "create", "review"]).assert().success();
    let cid = json_of(&out.get_output().stdout)["channel_id"].as_str().unwrap().to_string();
    // Send a question with an explicit thread id so reply can target the message id.
    let out = nxc(&tmp).args(["--json", "send", &cid, "which db?", "--kind", "question", "--thread", "m-th1"])
        .assert().success();
    let qid = json_of(&out.get_output().stdout)["message_id"].as_str().unwrap().to_string();
    // Reply targeting the MESSAGE id → posts into cid carrying thread m-th1.
    let out = nxc(&tmp).args(["--json", "reply", &qid, "sqlite"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v["persisted"], true);
    assert_eq!(v["thread_id"], "m-th1");
    // The reply lands in the same channel and is found by search there.
    let out = nxc(&tmp).args(["--json", "search", "sqlite"]).assert().success();
    let hits = json_of(&out.get_output().stdout);
    assert_eq!(hits[0]["channel_id"], cid);

    // Reply to an unknown target → not_found.
    let out = nxc(&tmp).args(["--json", "reply", "m-nope", "x"]).assert().failure();
    assert_eq!(json_of(&out.get_output().stdout)["error"]["kind"], "not_found");
}
```

*(The `search` assertion depends on Task 9. Split: assert `persisted`/`thread_id` + the not_found path here; add the `search` cross-check when Task 9 lands, or reorder Task 9 before this. Simplest: implement Task 9 before Task 7, OR drop the search line from this test.)*

- [ ] **Step 2: Run to verify it fails**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs -- reply`
Expected: FAIL — `reply` unknown.

- [ ] **Step 3: Add the `Reply` verb.** In `cli.rs`:

Add to `enum Command`:

```rust
    /// Reply to a thread or message: a convenience over `send` that posts into the same channel,
    /// carrying the resolved `thread_id` (spec §5.1).
    Reply {
        /// The `thread_id` or `message_id` to reply to.
        target: String,
        /// The reply body.
        body: String,
        #[arg(long, default_value = "info", value_parser = ["task", "question", "report", "decision", "info"])]
        kind: String,
        #[arg(long, default_value = "normal", value_parser = ["urgent", "normal", "background"])]
        priority: String,
        #[arg(long, default_value = "in_turn", value_parser = ["in_turn", "next_session"])]
        disposition: String,
        #[arg(long = "ref")]
        refs: Vec<String>,
    },
```

Dispatch arm:

```rust
        Command::Reply { target, body, kind, priority, disposition, refs } =>
            reply(cli.json, db, target, body, kind, priority, disposition, refs),
```

Verb body:

```rust
/// `nxc reply <thread_id|message_id> "<body>" […]` (spec §5.1). Resolve the target to its channel +
/// thread, then post carrying `thread_id`. A message target inherits its own `thread_id` (or, if the
/// message is itself the thread root, its own id is not assumed — only an explicit thread carries).
#[allow(clippy::too_many_arguments)]
fn reply(json: bool, db: Option<&str>, target: &str, body: &str, kind: &str, priority: &str,
         disposition: &str, refs: &[String]) -> Result<()> {
    let mut store = open(db)?;
    // Resolve target → (channel_id, thread_id). A thread id resolves to its channel and IS the
    // thread; a message id resolves to its channel and carries the message's own thread (if any).
    let (channel, thread): (String, Option<String>) = if let Some(ch) = store.thread_channel(target) {
        (ch, Some(target.to_string()))
    } else if let Some((ch, th)) = store.message_channel(target) {
        (ch, th)
    } else {
        return Err(NxfError::not_found(format!("no such thread or message: {target}")));
    };
    store.set_wall_clock(&resolve_now()?);
    let env = MessageEnvelope {
        origin: origin(),
        channel_id: channel,
        sender: caller_handle(),
        kind: parse_kind(kind)?,
        priority: parse_priority(priority)?,
        disposition: parse_disposition(disposition)?,
        thread_id: thread.clone(),
        refs: parse_refs(refs)?,
        body: body.to_string(),
    };
    let message_id = store.post_message(&env);
    if json {
        println!("{}", serde_json::json!({
            "message_id": message_id, "persisted": true, "thread_id": thread
        }));
    } else {
        println!("replied {message_id}");
    }
    Ok(())
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs -- reply`
Expected: PASS. (If the `search` cross-check line is present, land Task 9 first.)

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/cli.rs crates/chat/tests/verbs.rs
git commit -m "feat(nexus-chat): T2 nxc reply (resolve thread/message → channel) (6j6v.z263)"
```

---

## Task 8: `agents` — list / register / search

**Files:**
- Modify: `crates/chat/src/cli.rs`, `crates/chat/tests/verbs.rs`

**Interfaces:**
- Consumes: `set_profile_field`, `profiles`/`profile`/`search_profiles` (Task 2), `PROFILE_FIELDS`.
- Produces: the `Agents` subcommand with `List`/`Register`/`Search`.

- [ ] **Step 1: Write the failing tests** — add to `verbs.rs`:

```rust
#[test]
fn register_then_list_and_search_agents() {
    let tmp = workspace();
    nxc(&tmp).args(["agents", "register", "local/qa", "--title", "QA Reviewer",
                    "--desc", "runs the race-flag suite", "--tags", "qa,rust", "--reports-to", "local/alice"])
        .assert().success();
    // list
    let out = nxc(&tmp).args(["--json", "agents"]).assert().success();
    let v = json_of(&out.get_output().stdout);
    assert_eq!(v[0]["handle"], "local/qa");
    assert_eq!(v[0]["job_title"], "QA Reviewer");
    assert_eq!(v[0]["capability_tags"], "[\"qa\",\"rust\"]");
    assert_eq!(v[0]["runtime_binding"], "local");
    assert_eq!(v[0]["reports_to"], "local/alice");
    // search over job_title AND job_description, case-insensitive, bound LIKE
    let out = nxc(&tmp).args(["--json", "agents", "search", "REVIEWER"]).assert().success();
    assert_eq!(json_of(&out.get_output().stdout).as_array().unwrap().len(), 1);
    let out = nxc(&tmp).args(["--json", "agents", "search", "race"]).assert().success();
    assert_eq!(json_of(&out.get_output().stdout).as_array().unwrap().len(), 1, "matches job_description");
    // a LIKE metacharacter in the query is treated literally (bound param, no injection)
    let out = nxc(&tmp).args(["--json", "agents", "search", "%"]).assert().success();
    assert_eq!(json_of(&out.get_output().stdout).as_array().unwrap().len(), 0, "literal %, not a wildcard");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs -- agents`
Expected: FAIL — `agents` unknown.

> **Note:** the `%`-literal assertion requires escaping `%`/`_` in the bound query (a bound param stops SQL *injection* but `%` is still a LIKE wildcard). Update `search_profiles`/`search_messages` (Task 2) to escape: build the pattern as `format!("%{}%", escape_like(&q.to_lowercase()))` where `escape_like` backslash-escapes `\`, `%`, `_`, and add `ESCAPE '\'` to the `LIKE ?n` clauses. Add `escape_like` to `store.rs`:
>
> ```rust
> /// Escape LIKE metacharacters so a user query matches them literally (bound params stop injection,
> /// not wildcard expansion). Pair with `LIKE ?n ESCAPE '\\'`.
> fn escape_like(s: &str) -> String {
>     let mut out = String::with_capacity(s.len());
>     for c in s.chars() {
>         if matches!(c, '\\' | '%' | '_') { out.push('\\'); }
>         out.push(c);
>     }
>     out
> }
> ```
> Apply the same to the message search. (If you prefer to keep Task 2 closed, fold this escape into Task 8 as an amendment to `store.rs` with its own test.)

- [ ] **Step 3: Add the `Agents` subcommand.** In `cli.rs`:

Add to `enum Command`:

```rust
    /// Agent/user profiles (spec §5.1): list, register/update, or search by job_title + job_description.
    Agents {
        #[command(subcommand)]
        action: Option<AgentsAction>,
    },
```

Action enum:

```rust
#[derive(Subcommand, Debug)]
enum AgentsAction {
    /// List all profiles (default when no action is given), handle-sorted.
    List,
    /// Register or update a profile by its qualified handle.
    Register {
        handle: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        desc: Option<String>,
        /// Comma-separated capability tags (stored as a JSON array).
        #[arg(long)]
        tags: Option<String>,
        #[arg(long = "reports-to")]
        reports_to: Option<String>,
    },
    /// Case-insensitive substring over job_title + job_description (bound LIKE).
    Search { query: String },
}
```

Dispatch arm:

```rust
        Command::Agents { action } => agents(cli.json, db, action.as_ref()),
```

Verb body:

```rust
fn agents(json: bool, db: Option<&str>, action: Option<&AgentsAction>) -> Result<()> {
    let mut store = open(db)?;
    match action.unwrap_or(&AgentsAction::List) {
        AgentsAction::List => print_profiles(json, &store.profiles()),
        AgentsAction::Search { query } => print_profiles(json, &store.search_profiles(query)),
        AgentsAction::Register { handle, title, desc, tags, reports_to } => {
            let author = caller_handle();
            if let Some(t) = title { store.set_profile_field(handle, "job_title", t, &author); }
            if let Some(d) = desc { store.set_profile_field(handle, "job_description", d, &author); }
            if let Some(t) = tags {
                let arr: Vec<&str> = t.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
                let json_tags = serde_json::to_string(&arr).expect("tags serialize");
                store.set_profile_field(handle, "capability_tags", &json_tags, &author);
            }
            if let Some(r) = reports_to { store.set_profile_field(handle, "reports_to", r, &author); }
            // M1 defaults: every handle is a local runtime binding; origin is folded (spec §2.2).
            store.set_profile_field(handle, "runtime_binding", "local", &author);
            store.set_profile_field(handle, "origin", &origin(), &author);
            match store.profile(handle) {
                Some(p) if json => println!("{}", serde_json::to_string(&p).expect("profile serializes")),
                Some(_) => println!("registered {handle}"),
                None => return Err(NxfError::io(format!("profile {handle} vanished after register"))),
            }
        }
    }
    Ok(())
}

fn print_profiles(json: bool, rows: &[crate::store::ProfileRow]) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(rows).expect("profiles serialize"));
    } else if rows.is_empty() {
        println!("no agents");
    } else {
        for p in rows {
            println!("{}  {}", p.handle, p.job_title.as_deref().unwrap_or(""));
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs -- agents && cargo test -p nexus-chat --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/cli.rs crates/chat/src/store.rs crates/chat/tests/verbs.rs
git commit -m "feat(nexus-chat): T2 nxc agents list/register/search (bound LIKE, escaped) (6j6v.z263)"
```

---

## Task 9: `search` — message body substring in the caller's channels

**Files:**
- Modify: `crates/chat/src/cli.rs`, `crates/chat/tests/verbs.rs`

**Interfaces:**
- Consumes: `search_messages` (Task 2, with the `escape_like` amendment from Task 8).
- Produces: the `Search` verb.

- [ ] **Step 1: Write the failing test** — add to `verbs.rs`:

```rust
#[test]
fn search_finds_body_substrings_only_in_the_callers_channels() {
    let tmp = workspace();
    let out = nxc(&tmp).args(["--json", "channels", "create", "review"]).assert().success();
    let cid = json_of(&out.get_output().stdout)["channel_id"].as_str().unwrap().to_string();
    nxc(&tmp).args(["send", &cid, "ship the RELEASE now"]).assert().success();
    nxc(&tmp).args(["send", &cid, "unrelated chatter"]).assert().success();

    let out = nxc(&tmp).args(["--json", "search", "release"]).assert().success();
    let hits = json_of(&out.get_output().stdout);
    assert_eq!(hits.as_array().unwrap().len(), 1, "case-insensitive substring over body");
    assert_eq!(hits[0]["channel_id"], cid);
    assert_eq!(hits[0]["body"], "ship the RELEASE now");

    // A non-member sees nothing (channel scoping).
    let out = nxc(&tmp).args(["--json", "search", "release", "--consumer", "local/outsider"]).assert().success();
    assert_eq!(json_of(&out.get_output().stdout).as_array().unwrap().len(), 0);
}
```

*(`search` takes an optional `--consumer` to scope to another handle's channels — add it for the test's non-member assertion; default is the caller.)*

- [ ] **Step 2: Run to verify it fails**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs -- search_finds`
Expected: FAIL — `search` unknown.

- [ ] **Step 3: Add the `Search` verb.** In `cli.rs`:

Add to `enum Command`:

```rust
    /// Case-insensitive substring over message bodies in the caller's channels (bound LIKE),
    /// deterministically ordered (spec §5.1).
    Search {
        /// The substring to search for.
        query: String,
        /// Search as this handle's channels (default: the caller's own qualified handle).
        #[arg(long)]
        consumer: Option<String>,
    },
```

Dispatch arm:

```rust
        Command::Search { query, consumer } => search(cli.json, db, query, consumer.as_deref()),
```

Verb body:

```rust
fn search(json: bool, db: Option<&str>, query: &str, consumer: Option<&str>) -> Result<()> {
    let store = open(db)?;
    let consumer = consumer.map(str::to_string).unwrap_or_else(caller_handle);
    let hits = store.search_messages(&consumer, query);
    if json {
        println!("{}", serde_json::to_string(&hits).expect("hits serialize"));
    } else if hits.is_empty() {
        println!("no matches");
    } else {
        for h in &hits {
            println!("{}  {}  {}", h.channel_id, h.sender, h.body);
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo build -p nxs && cargo test -p nexus-chat --test verbs`
Expected: PASS (all verb tests). If Task 7's `reply` test carried the `search` cross-check, it now passes too.

- [ ] **Step 5: Commit**

```bash
git add crates/chat/src/cli.rs crates/chat/tests/verbs.rs
git commit -m "feat(nexus-chat): T2 nxc search (body substring in caller's channels, bound LIKE) (6j6v.z263)"
```

---

## Task 10: Full gates + byte-stable golden assertions + PR

**Files:**
- Modify: `crates/chat/tests/verbs.rs` (add the explicit byte-exact `--json` assertions — the "byte-stable Goldens" deliverable)

**Interfaces:** none new — this task hardens and gates.

- [ ] **Step 1: Add the byte-exact golden assertions.** Add to `verbs.rs` a test that pins the EXACT stdout bytes for the core `--json` verbs (the byte-stable contract; deterministic ids + pinned NXC_NOW/actor/origin make this stable):

```rust
#[test]
fn json_output_is_byte_stable_under_the_determinism_switch() {
    let tmp = workspace();
    // First minted channel → m-...0001; first message → m-...0001 (kind-scoped counters).
    nxc(&tmp).args(["--json", "channels", "create", "review"]).assert().success()
        .stdout("{\"channel_id\":\"m-00000000000000000000000001\",\"name\":\"review\",\"kind\":\"group\"}\n");
    nxc(&tmp).args(["--json", "send", "m-00000000000000000000000001", "ship it"]).assert().success()
        .stdout("{\"message_id\":\"m-00000000000000000000000001\",\"persisted\":true}\n");
    // A DM id is deterministic by construction (sha256 of the sorted handles).
    let dm = nexus_chat_dm_id("local/alice", "local/bob");
    nxc(&tmp).args(["--json", "channels", "dm", "local/alice", "local/bob"]).assert().success()
        .stdout(format!("{{\"channel_id\":\"{dm}\",\"kind\":\"direct\",\"members\":[\"local/alice\",\"local/bob\"]}}\n"));
}

/// Mirror of the CLI's `dm_channel_id` so the test can assert the exact id (kept in-test to avoid
/// exporting the helper).
fn nexus_chat_dm_id(a: &str, b: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut pair = [a, b]; pair.sort_unstable();
    let mut h = Sha256::new(); h.update(pair[0].as_bytes()); h.update([0u8]); h.update(pair[1].as_bytes());
    let hex = h.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
    format!("dm:{}", &hex[..24])
}
```

Add `sha2` to `crates/chat/Cargo.toml` `[dev-dependencies]` (the test uses it):

```toml
sha2 = "0.10"
```

- [ ] **Step 2: Run the chat crate gates**

Run: `cargo build -p nxs && cargo test -p nexus-chat && cargo clippy -p nexus-chat --all-targets -- -D warnings && cargo fmt -p nexus-chat --check`
Expected: all PASS. Fix any clippy/fmt nits (e.g. `#[allow(clippy::too_many_arguments)]` already on `send`/`reply`).

- [ ] **Step 3: Run the FULL workspace gates** (the M1 invariant: flow + memory unchanged; the `nxc` symlink built; release catches nothing new):

```bash
cargo test
cargo test --release
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: all green. The flow differential-oracle + memory/flow goldens are unchanged (chat only ADDED a routing arm + a dep to `nxs`; no flow/memory code path changed). The added `ErrorKind::Forbidden` is additive.

- [ ] **Step 4: Confirm the `nxs` multicall routes `nxc` end-to-end** (manual smoke, mirrors how a host would reach it):

```bash
cargo build -p nxs
DIR=$(mktemp -d); ( cd "$DIR" && NXF_DETERMINISTIC_IDS=1 NXC_ACTOR=alice \
  "$PWD"/../target/debug/nxc --json channels create review ) # via the nxc symlink
# and via the umbrella token:
( cd "$DIR" && target/debug/nxs chat --json channels ) 2>/dev/null || true
```

Expected: `nxc` (symlink) and `nxs chat` (umbrella token) both reach the chat CLI. (Adjust paths; the point is both personas route.)

- [ ] **Step 5: Push + open the PR.**

```bash
git push -u origin feat/nexus-chat-t2-cli
gh pr create --title "feat(nexus-chat): M1 T2 — nxc messaging verbs (send/reply/inbox/read/channels/agents/search)" \
  --body "<see body below>"
```

PR body must cover:
- **Scope:** T2 (spec §10) — the `nxc` messaging verbs over the merged T1 substrate; a pure pull consumer of the views.
- **In-scope deviations from §10's T2/T3 line (user-approved 2026-07-09):**
  1. The **minimal `nxs` routing** (persona symlink + `Some("nxc")`/`Some("chat")` arms + `nexus-chat` dep) lands here because byte-stable `--json` goldens need a runnable binary. The **`ModuleInit` self-registration + the `use nexus_chat as _;` link line + `init`/`agent-manifest`/`prime`** stay **T3** (`6j6v.rtgh`).
  2. **`ErrorKind::Forbidden`** added to the shared foundation error set (additive; only `as_str` gains an arm) to honor the §5.1 `not_found`/`forbidden` contract literally (unknown channel → not_found; non-member → forbidden).
- **T1 review follow-ups closed:** the `author="system"` placeholder is removed (the real actor `NXC_ACTOR→USER`, qualified `<origin>/<agent>`, is threaded through every write helper — review T1 #179); `search`/`agents search` use bound, LIKE-escaped params (review 3cz4); inbox/read are read-defensive on non-member and unknown channels.
- **Tests:** `crates/chat/tests/verbs.rs` — black-box `assert_cmd` over the `nxc` binary, byte-exact `--json` assertions under the determinism switch + JSON-field behavioral assertions + the `not_found`/`forbidden` failure paths.
- **Not in T2:** the `.trycmd` doc-example suite (co-located with the T3 `init`/`prime`, mirroring memory's `memory.trycmd`), the `ModuleInit` registration, sync round-trip + the flow/memory byte-identity gate (T4 `6j6v.j25s`), the changelog fragment.
- **Changelog:** deferred to T3 per spec decision → apply the **`skip-changelog`** label.

- [ ] **Step 6: Apply the `skip-changelog` label** (T2 carries no user-facing changelog; it lands with T3):

```bash
gh pr edit --add-label skip-changelog
```

- [ ] **Step 7: Verify CI is green** (all six gates: `cargo test`, `cargo test --release`, clippy, fmt, changelog-check [satisfied by the label], facade-semver [untouched]).

- [ ] **Step 8: Close the ticket** once the PR is up and green:

```bash
nxf close z263 --reason "T2 nxc messaging verbs (send/reply/inbox/read/channels/agents/search) over the merged T1 substrate: byte-stable --json, disposition-partitioned inbox, synced read cursor, deterministic DM (exactly-2 on create), bound-LIKE search, not_found/forbidden non-member paths. Minimal nxs routing + ErrorKind::Forbidden added per 2026-07-09 decision; ModuleInit/init/prime + changelog stay T3 (rtgh). PR: <url>"
```

---

## Self-Review

**1. Spec coverage (`docs/specs/nexus-chat-M1.md` §2.1/§5.1/§6):**
- §5.1 `send` (persist-ack `{message_id,persisted:true}`, defaults kind=info/priority=normal/disposition=in_turn, `--thread`, `--ref`) → Task 4. ✓
- §5.1 `reply` (convenience over send, carries thread_id, resolve thread|message → channel) → Task 7. ✓
- §5.1 `inbox` (disposition interpretation: default in_turn, `--all` includes next_session; PURE read, no cursor advance; `--channel`/`--consumer`) → Task 6. ✓
- §5.1 `read --through` (advance the SYNCED read_cursor = the at-least-once ack) → Task 6. ✓
- §5.1 `channels` list/create/dm/join/leave (DM exactly-2 on create; degraded-DM read-defensive) → Task 5 (create/dm/join/leave) + Task 2 (`list_member_channels` surfaces `degraded`). ✓
- §5.1 `agents` list/register/search (job_title + job_description, bound LIKE) → Task 8. ✓
- §5.1 `search` (body substring in caller's channels, bound LIKE, deterministic order) → Task 9. ✓
- §5.1 not_found/forbidden (unknown channel → not_found; non-member → forbidden) → Task 1 (kind) + Task 6 (`require_member`) + Task 4/6/9 (channel-existence). ✓
- §5.1 `--consumer` defaults to caller's handle; §2.2 qualified handle `<origin>/<agent>` → Task 4 (`caller_handle`). ✓
- §2.1 envelope (all fields, enum ranges via clap value_parser + serde) → Task 4. ✓
- §6 pull-only (verb = one-shot pull consumer, no watcher/daemon) → the CLI holds no `Engine::subscribe`; every verb opens, reads/writes, exits. ✓
- §3.2 deterministic DM id (`dm:`+sha256 of sorted handles) → Task 5. ✓
- §3.4 synced cursor is a max-register (never regresses) → Task 2/6 drive `advance_read_cursor` (fold is T1). ✓
- Review 3cz4 (bound LIKE, no interpolation; inbox read-defensive on degraded DM) → Task 2 (`escape_like` + bound params) + `list_member_channels` degraded flag. ✓
- Review T1 #179 (real actor, not "system") → Task 2 (author threaded through all writes). ✓

**2. Placeholder scan:** No "TBD"/"handle edge cases". Three explicit cross-task couplings are flagged, not hidden: (a) Task 4's `send` happy-path test needs `channels create` → deferred to Task 5 Step 4; (b) Task 7's `reply` test's `search` cross-check needs Task 9 → noted to reorder or drop the line; (c) Task 8's `%`-literal assertion needs the `escape_like` amendment → spelled out with full code. Each is a "land in this order / this exact code" instruction, not a vague placeholder.

**3. Type consistency:** `caller_handle()`/`origin()`/`actor()`/`resolve_now()`, `parse_kind/priority/disposition`, `parse_refs`, `dm_channel_id`, `require_member`, `open()`, the `ChannelRow`/`ProfileRow`/`MessageHit`/`InboxOut` structs, and the store signatures (`author: &str` on every write; `mint_channel_id`/`mint_thread_id`; `channel_exists`/`is_member`/`message_channel`/`thread_channel`/`list_member_channels`/`profiles`/`profile`/`search_profiles`/`search_messages`) are used consistently across tasks. `post_message` keeps its 1-arg signature (author = `env.sender`). `ErrorKind::Forbidden`/`NxfError::forbidden` (Task 1) are consumed by `require_member` (Task 6). The `nxc` routing (Task 3) calls `nexus_chat::run_from` (Task 4).

**Deferred to later slices (not T2):** the `.trycmd` doc-example goldens, `ModuleInit` self-registration + `init`/`agent-manifest`/`prime` + the `use nexus_chat as _;` link line + the changelog fragment (all **T3** `6j6v.rtgh`); the live `nxs-sync` message round-trip + the flow/memory byte-identity acceptance gate (**T4** `6j6v.j25s`); the 2-agent dogfood handoff (`6j6v.cdy7`, now gated on T4).
