//! Typed parameter structs for the flow MCP tools (#y6f / #wpw).
//!
//! These are the single schema/validation source for the tool inputs: each derives
//! `serde::Deserialize` (request decoding) and `schemars::JsonSchema` (the tool's published input
//! schema), so the schema a host sees and the type a tool receives can never drift. The flow CLI's
//! clap surface is unchanged and stays green.
//!
//! `schemars` is reached through rmcp's re-export (`rmcp::schemars`) so the derive always matches the
//! version rmcp generates tool schemas with. The field semantics mirror the `nxf` CLI flags 1:1, so
//! a tool call and the matching `nxf <cmd>` resolve the same record (the cross-seam invariant).
//!
//! Every MUTATION struct (#wpw/#yie) carries `now` and `actor` as EXPLICIT optional parameters (the
//! epic invariant): `now` is the wall-clock stamped on the emitted ops (defaults like the read
//! tools' `now`), `actor` overrides the server's launch `--actor` for that one call so a long-lived
//! host can attribute each write rather than author every op under one identity (#zxj).
//!
//! Every struct — read AND write — also carries `workspace`, the per-call workspace override
//! (S7/ppa, the second branch of the hybrid discovery). Omitted (or empty) it resolves against the
//! server's launch-pinned workspace (the App-Data-Home default or `--workspace`, #76u.7); given a
//! non-empty path it resolves THIS call against the `.nxs/` there instead, so one launched server can
//! serve many workspaces without a registry. The store is opened per call, so the override is just a
//! different resolution root — resolution WALKS UP from the given path (CLI discovery), so a path with
//! no `.nxs/` in itself OR any ancestor is a `no_workspace` DOMAIN error (the tool result), distinct
//! from a missing launch default (a `serve()` START error). Like `actor`, an empty string is treated
//! as unset and falls back to the launch default.

use rmcp::schemars;
use serde::Deserialize;

/// `flow_create` input — a new item (the mutation half of the seam, #wpw). Fields mirror the
/// `nxf create` flags 1:1, so the tool and `nxf create` resolve the same record.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowCreateParams {
    /// Item type — a value the active plugin declares (e.g. `feature` | `bug` | `epic`).
    #[serde(rename = "type")]
    pub item_type: String,
    /// Short human title.
    pub title: String,
    /// The mandatory free-text description (why + goal).
    pub description: String,
    /// Priority label (the active plugin's vocabulary, e.g. `P1`).
    pub priority: String,
    /// Optional design / path-to-goal long text.
    #[serde(default)]
    pub design: Option<String>,
    /// Optional definition-of-done / completion criterion.
    #[serde(default)]
    pub dod: Option<String>,
    /// Optional due date (ISO-8601 / RFC3339).
    #[serde(default)]
    pub due: Option<String>,
    /// Optional defer-until date (ISO-8601 / RFC3339).
    #[serde(default)]
    pub defer: Option<String>,
    /// Optional parent id (a bare local suffix or a full `<prefix>.<suffix>`).
    #[serde(default)]
    pub parent: Option<String>,
    /// Ids this item depends on (each a bare suffix or a full id).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Plugin custom-field assignments as `name=value` strings (plugin-custom-fields §5), mirroring
    /// `nxf create --set name=value`. Canonical fields use the dedicated params above; `set` carries
    /// the active plugin's declared custom fields, validated against the resolved type at write time.
    #[serde(default)]
    pub set: Vec<String>,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_update` input — set fields on an existing item (#wpw). `set` mirrors the repeatable
/// `nxf update --set field=value`: each entry is a `field=value` string the shared write layer
/// validates (the same whitelist, aliases, and per-field checks as the CLI).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowUpdateParams {
    /// Item id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub id: String,
    /// Field assignments as `field=value` strings (e.g. `status=in_progress`, `priority=P0`).
    pub set: Vec<String>,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_claim` input — mark an item in progress (and optionally assign it) (#yie).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowClaimParams {
    /// Item id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub id: String,
    /// Optional assignee recorded on the item.
    #[serde(default)]
    pub assignee: Option<String>,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_close` input — close an item with a mandatory closing comment (#yie). Closing is the only
/// way an item leaves the board (no hard delete), so `reason` is required.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowCloseParams {
    /// Item id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub id: String,
    /// The mandatory closing comment (the "so we closed it this way" outcome half).
    pub reason: String,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_dep_add`/`flow_dep_remove` + `flow_mention_add`/`flow_mention_remove` input — a directed
/// `from -> to` edge (#yie). For a dependency, `from` depends on `to`; for a mention, `from`'s text
/// cites `to`. Both ids may be a bare suffix or a full id.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowEdgeParams {
    /// The edge source id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub from: String,
    /// The edge target id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub to: String,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_label_add`/`flow_label_remove` input (h89s.3) — one item id plus the label text. The
/// label is opaque user vocabulary; the server trims + validates it (blank/separator rejected).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowLabelParams {
    /// Item id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub id: String,
    /// The label text to attach/detach.
    pub label: String,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_archive`/`flow_unarchive` input — a batch of cascade roots (#76u.12). `archive` cascades
/// DOWN each fully-closed subtree; `unarchive` cascades UP each item's ancestor chain. Each id may
/// be a bare suffix or a full id; at least one is required (an empty batch is a no-op receipt).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowArchiveParams {
    /// Item ids to (un)archive — each a cascade root (a bare local suffix or a full `<prefix>.<suffix>`).
    pub ids: Vec<String>,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_note_add` input — append an immutable worklog note to an item (#yie).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowNoteAddParams {
    /// Item id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub id: String,
    /// The note body (free text).
    pub text: String,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_next` input — actionable work (ready plus already-claimed) ranked by the active plugin's
/// `next` policy, tiered to finish before starting.
// The removed `include_in_progress` field (claimed work is always included now) is documented HERE,
// in a plain comment, deliberately: a doc comment lands in the published tool schema's
// `description`, and naming a dead parameter there would invite a model to send it again. Like
// every params struct in this module, this one carries no `deny_unknown_fields`, so an older client
// that still sends the field is tolerated — serde ignores it rather than hard-erroring the call.
// Both halves are pinned by the tests at the foot of this file.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct FlowNextParams {
    /// Reference time (ISO-8601 / RFC3339); defaults like `flow_prime`'s `now`.
    #[serde(default)]
    pub now: Option<String>,
    /// Sort order: `rank` | `id` (default `rank`). An unknown key is a loud error.
    #[serde(default)]
    pub sort: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_show` input — one item with its dependencies, contributes-to edges, and notes.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowShowParams {
    /// Item id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub id: String,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_list` input — items optionally filtered by status and/or type.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct FlowListParams {
    /// Filter by status.
    #[serde(default)]
    pub status: Option<String>,
    /// Filter by type (the active plugin's vocabulary, e.g. `feature` | `bug`).
    #[serde(default, rename = "type")]
    pub item_type: Option<String>,
    /// Sort order: `rank` | `id` (default `id`). An unknown key is a loud error.
    #[serde(default)]
    pub sort: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_blocked` input — open items held up by an open blocker (or in a cycle), each with its
/// open blockers named. Mirrors `nxf blocked`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct FlowBlockedParams {
    /// Sort order: `rank` | `id` (default `rank`). An unknown key is a loud error.
    #[serde(default)]
    pub sort: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_search` input — a lane-ranked substring search over title/description/design/DoD/notes,
/// refined by the optional filters. Field semantics mirror `nxf search` 1:1.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowSearchParams {
    /// Case-insensitive substring matched over title, description, design, DoD, and notes.
    pub query: String,
    /// Filter by status (applied before grouping).
    #[serde(default)]
    pub status: Option<String>,
    /// Filter by type (the active plugin's vocabulary, e.g. `feature` | `bug`).
    #[serde(default, rename = "type")]
    pub item_type: Option<String>,
    /// Reference time (ISO-8601 / RFC3339); sets the defer boundary used to group deferred matches.
    #[serde(default)]
    pub now: Option<String>,
    /// Append the archived group (lowest priority) instead of excluding it.
    #[serde(default)]
    pub include_archived: bool,
    /// Search ONLY the archive; the live lanes are skipped.
    #[serde(default)]
    pub archived_only: bool,
    /// Flatten the lane grouping into one order by this key (`rank` | `id` | …). Unknown = loud error.
    #[serde(default)]
    pub sort: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_note_list` / `flow_mention_list` input — one item addressed by id. Both read tools take
/// just the item id (a bare local suffix or a full `<prefix>.<suffix>`).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FlowItemRefParams {
    /// Item id (a bare local suffix or a full `<prefix>.<suffix>`).
    pub id: String,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `flow_schema` input — the active plugin's field + type-system model. Takes no filter beyond the
/// optional per-call workspace override. Mirrors `nxf schema`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct FlowSchemaParams {
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

// ---- umbrella tool params (0jq8) ----------------------------------------------------------------

/// `list_workspaces` input — no arguments. The umbrella tool lists the GLOBAL
/// `~/.nexusflow/workspaces.toml` registry, independent of any launch workspace or per-call override,
/// so unlike every module tool it carries no `workspace` field. Kept as an (empty) struct so the tool
/// still publishes a JSON-Schema input object and can grow options later without a wire break.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ListWorkspacesParams {}

// ---- memory tool params (#76u.2 read / #76u.3 write) --------------------------------------------
//
// The memory line's typed inputs — the same single schema/validation source as the flow params, the
// mirror over the memory facade. Field semantics track the `nxm` CLI 1:1, so a tool call and the
// matching `nxm <cmd>` resolve the same record (the cross-seam invariant). The write structs carry
// `now`/`actor` as EXPLICIT optional parameters (the epic invariant), exactly like the flow
// mutations; the reads take neither — the `memories` view is independent of the clock. Every struct
// carries `workspace` (S7/ppa), same as the flow tools.

/// `memory_list` input — all active memories, key-sorted (no filter; use `memory_search` to filter).
/// Mirrors `nxm memories`.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct MemoryListParams {
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `memory_search` input — active memories matching a case-insensitive substring over key + body.
/// Mirrors `nxm memories <query>`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemorySearchParams {
    /// Case-insensitive substring matched against each memory's key and body.
    pub query: String,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `memory_show` input — one memory's full record by key. Mirrors `nxm recall <key>`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryShowParams {
    /// The memory key.
    pub key: String,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `memory_add` input — capture a NEW fact under the content-hash auto-key (#76u.3). The key is
/// derived from the text (so re-adding identical text is idempotent); use `memory_update` to evolve a
/// fact under a stable key. Mirrors `nxm remember <text>` (no `--key`).
///
/// The classification trio (6j6v.9a1r) files the memory in the SAME write, mirroring
/// `nxm remember --category/--scope/--refs`. Each is optional and an omitted one leaves that
/// register at its default (`unsorted`, reach `project`, no references).
///
/// `introduction` is NOT optional (6j6v.xbnh) — it is the one line the session start replays for
/// this memory, and a memory written without one reaches every future session as a placeholder.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryAddParams {
    /// The fact text to store.
    pub text: String,
    /// REQUIRED. The one line every session start is handed for this memory — at most 200
    /// characters, no line breaks. The body is read on demand with `memory_show`; this line is what
    /// decides whether anyone does, so it says what the memory SAYS rather than what it is about.
    /// For `category: "rules"` it SPEAKS the rule ("Never X, always Y"), because nobody looks a
    /// prohibition up before breaking it.
    pub introduction: String,
    /// Section to file this memory under — a lower-case slug (e.g. `introduction`, `architecture`,
    /// `rules`). Omitted ⇒ `unsorted`.
    #[serde(default)]
    pub category: Option<String>,
    /// How far this memory reaches: `item` (only the board items in `refs`) | `project` (this
    /// workspace) | `global` (beyond it). Omitted ⇒ `project`. A `project`/`global` memory is
    /// replayed at session start; an `item` one surfaces on the board items it names instead.
    #[serde(default)]
    pub scope: Option<String>,
    /// Board item ids this memory is about (e.g. `["6j6v.e0z6"]`). Required for `scope: "item"` to
    /// reach anything. An empty array is an explicit "no references".
    #[serde(default)]
    pub refs: Option<Vec<String>>,
    /// Reference time (ISO-8601 / RFC3339) stamped as the memory's `updated`.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded as the memory's author; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `memory_update` input — upsert a fact in place under an explicit, stable key (#76u.3); overwrites
/// the body stored under `key`. Mirrors `nxm remember <text> --key <key>`. Carries the same optional
/// classification trio as [`MemoryAddParams`] (6j6v.9a1r).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryUpdateParams {
    /// The stable key to upsert in place (e.g. `auth-jwt`).
    pub key: String,
    /// The new fact text for this key.
    pub text: String,
    /// REQUIRED. The one line every session start is handed for this memory — at most 200
    /// characters, no line breaks. The body is read on demand with `memory_show`; this line is what
    /// decides whether anyone does, so it says what the memory SAYS rather than what it is about.
    /// For `category: "rules"` it SPEAKS the rule ("Never X, always Y"), because nobody looks a
    /// prohibition up before breaking it.
    pub introduction: String,
    /// Section to file this memory under — a lower-case slug (e.g. `introduction`, `architecture`,
    /// `rules`). Omitted leaves the memory's current section untouched.
    #[serde(default)]
    pub category: Option<String>,
    /// How far this memory reaches: `item` | `project` | `global`. Omitted leaves the current reach
    /// untouched.
    #[serde(default)]
    pub scope: Option<String>,
    /// Board item ids this memory is about. Replaces the whole set; an empty array clears it;
    /// omitted leaves it untouched.
    #[serde(default)]
    pub refs: Option<Vec<String>>,
    /// Reference time (ISO-8601 / RFC3339) stamped as the memory's `updated`.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded as the memory's author; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `memory_classify` input — file an EXISTING memory without touching its text (6j6v.9a1r,
/// 6j6v.xbnh). Mirrors `nxm classify <key> [--category …] [--scope …] [--refs …] [--introduction …]`:
/// at least one of the four is required (silently doing nothing would be worse than saying so), and
/// an omitted one leaves that register exactly as it was.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryClassifyParams {
    /// The key of the memory to file.
    pub key: String,
    /// Rewrite the one line every session start is handed for this memory — at most 200 characters,
    /// no line breaks. This is also how a memory written before introductions existed gets one.
    #[serde(default)]
    pub introduction: Option<String>,
    /// Section to file it under — a lower-case slug (e.g. `introduction`, `architecture`, `rules`).
    #[serde(default)]
    pub category: Option<String>,
    /// How far it reaches: `item` | `project` | `global`.
    #[serde(default)]
    pub scope: Option<String>,
    /// Board item ids it is about. Replaces the whole set; an empty array clears it.
    #[serde(default)]
    pub refs: Option<Vec<String>>,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops. Filing does NOT move the
    /// memory's own `updated` stamp — that dates the fact, not where it is filed.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `memory_reorder` input — store an explicit reading order (6j6v.9a1r). Mirrors
/// `nxm reorder <key>…`: the first key becomes position 1, the second 2, and so on. Deliberate and
/// rare — everyday writes never touch the order, and a memory that was never reordered keeps
/// sorting by insertion order, after every placed one.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryReorderParams {
    /// The memory keys, in the order they should read. Every key must exist — a rejected sequence
    /// leaves the stored order exactly as it was.
    pub keys: Vec<String>,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted ops.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted ops; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

/// `memory_close` input — forget a memory by key (a reversible tombstone) (#76u.3). Mirrors
/// `nxm forget <key>`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MemoryCloseParams {
    /// The memory key to forget.
    pub key: String,
    /// Reference time (ISO-8601 / RFC3339) stamped on the emitted tombstone op.
    #[serde(default)]
    pub now: Option<String>,
    /// Actor recorded on the emitted tombstone op; overrides the server's launch `--actor`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Per-call workspace override (S7/ppa); empty/omitted ⇒ the launch default. See the module docs.
    #[serde(default)]
    pub workspace: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wire-compatibility guard for the removed `flow_next` parameter
    /// (`docs/specs/next-finish-first-tiers.md` §5): an older client that still sends
    /// `include_in_progress` must be TOLERATED, not hard-errored. This holds because no params
    /// struct here sets `deny_unknown_fields` — this test is what stops someone adding it (or a
    /// `#[serde(deny_unknown_fields)]` sweep) and silently breaking every stale MCP client.
    #[test]
    fn flow_next_tolerates_a_stale_include_in_progress_field() {
        let p: FlowNextParams =
            serde_json::from_str(r#"{"sort":"rank","include_in_progress":true}"#)
                .expect("a stale client's removed field is ignored, not rejected");
        assert_eq!(
            p.sort.as_deref(),
            Some("rank"),
            "the live fields still bind"
        );
    }

    /// The flag is gone from the published tool SCHEMA too — neither as an input property nor
    /// anywhere in the prose a host shows the model — so a current client is never invited to send
    /// it. (Doc comments on these structs become the schema's `description`, which is exactly how a
    /// dead parameter name leaks back into an agent's context.)
    #[test]
    fn flow_next_schema_no_longer_advertises_include_in_progress() {
        let schema = serde_json::to_value(schemars::schema_for!(FlowNextParams)).unwrap();
        let props = schema["properties"].as_object().expect("input properties");
        assert!(
            !props.contains_key("include_in_progress"),
            "the removed param must not be an input property: {props:?}"
        );
        assert!(
            props.contains_key("sort") && props.contains_key("now"),
            "sanity: the live params are still advertised: {props:?}"
        );
        assert!(
            !serde_json::to_string(&schema)
                .unwrap()
                .contains("include_in_progress"),
            "the removed param must not surface in the schema prose either: {schema}"
        );
    }
}
