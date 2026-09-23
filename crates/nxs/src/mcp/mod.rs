//! The MCP seam (E9 #76u): `nxs mcp serve` exposes the active modules' read ops as MCP tools over
//! stdio, against the SAME shared `.nxs/` store as the CLI and embedding apps.
//!
//! This is the agent-protocol seam beside the CLI (agents) and the embed API (apps): MCP-native
//! hosts (Claude Desktop, Amazon Quick) cannot drive the CLI, so the server wraps each active
//! module's Record-Facade as tool results. A tool's `structuredContent` is the canonical per-op
//! record — byte-identical to `nxf <cmd> --json` for the same state (proven by the cross-seam
//! parity tests). The rmcp SDK is protocol glue (handshake, stdio framing); the correctness path is
//! the synchronous facade calls in [`flow`].
//!
//! This module owns the rmcp wiring (the handler, the tool routers, `get_info`, `serve`); [`flow`]
//! and [`memory`] own the open-per-call facade reads/writes + render for their module; [`params`] the
//! typed tool inputs.
//!
//! The surface is a fan-out over the launch workspace's ACTIVE modules (epic invariant), like
//! `nxs prime`: the flow tools are the always-on base, and the memory tools (#76u.2/#76u.3) are
//! merged into the router ONLY when the launch workspace has the memory module active — so a
//! flow-only workspace never shows them. Remote transports are a later slice.

mod flow;
mod install;
mod memory;
mod params;

pub use install::install;

/// The workspace registry moved out of the MCP feature (the sync daemon reads it too and
/// must not depend on `mcp`). Re-exported under the historical path so the MCP seam and
/// its tests read unchanged.
pub use crate::workspaces as registry;
pub use registry::workspaces;

use crate::error::NxfError;
use crate::prime::PrimeFanOut;
use crate::workspaces::write_new_exclusive;
use nxs_foundation::workspace::Workspace;
use params::{
    FlowArchiveParams, FlowBlockedParams, FlowClaimParams, FlowCloseParams, FlowCreateParams,
    FlowEdgeParams, FlowItemRefParams, FlowLabelParams, FlowListParams, FlowNextParams,
    FlowNoteAddParams, FlowSchemaParams, FlowSearchParams, FlowShowParams, FlowUpdateParams,
    ListWorkspacesParams, MemoryAddParams, MemoryClassifyParams, MemoryCloseParams,
    MemoryListParams, MemoryReorderParams, MemorySearchParams, MemoryShowParams,
    MemoryUpdateParams,
};
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, Implementation, ServerCapabilities, ServerInfo, Tool};
use rmcp::{tool, ServerHandler, ServiceExt};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The nxs MCP server handler. Holds the launch-pinned workspace (MCP hosts have no useful cwd, so
/// the server fixes it at start) and the generated tool router — the flow base surface, plus the
/// memory tools when the launch workspace has the memory module active (#76u.2). The store is opened
/// per call (CLI behaviour), so this handle owns no live db.
#[derive(Clone)]
pub struct NxsMcp {
    /// Lower-level `--db`/`NXS_DB` store override, threaded to every per-call open (usually `None`).
    db: Option<String>,
    /// The resolved launch workspace root (#76u.7): `--workspace`/`NXS_WORKSPACE` if given, else the
    /// auto-initialized App-Data-Home default. `None` ONLY on the `--db` path, where the store
    /// override resolves the workspace directly and no root is needed. Never a cwd walk-up — MCP
    /// hosts have no useful cwd, so that tier is dropped for this persona.
    workspace: Option<PathBuf>,
    /// The server-instance actor default (#zxj): `--actor`/`NXS_ACTOR` if given. A per-tool `actor`
    /// argument overrides it per call; absent both, the `NXF_ACTOR`/`USER` env fallback applies. So
    /// a long-lived host attributes each write rather than authoring every op under one identity.
    launch_actor: Option<String>,
    tool_router: ToolRouter<Self>,
}

#[rmcp::tool_router(router = flow_router)]
impl NxsMcp {
    /// Build a handler pinned to a launch workspace (and optional db override + actor default). The
    /// tool router is the flow base surface; when `memory_active` (the launch workspace has the
    /// memory module on, #76u.2) the memory tools are merged in, so the registered surface is the
    /// fan-out over the active modules — a flow-only workspace exposes no `memory_*` tools.
    pub fn new(
        db: Option<String>,
        workspace: Option<PathBuf>,
        launch_actor: Option<String>,
        memory_active: bool,
    ) -> Self {
        let mut tool_router = Self::flow_router();
        if memory_active {
            tool_router.merge(Self::memory_router());
        }
        Self {
            db,
            workspace,
            launch_actor,
            tool_router,
        }
    }

    /// Ready, actionable work, ranked by the active plugin's `next` policy.
    #[tool(
        name = "flow_next",
        description = "List ready, actionable flow work — open, unblocked, not deferred — ranked by \
                       the active plugin's policy. Optionally fold in claimed in-progress work.",
        annotations(
            title = "List ready work",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_next(&self, Parameters(p): Parameters<FlowNextParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::next(db, &start, &p))
    }

    /// One item with its dependencies, contributes-to edges, and notes.
    #[tool(
        name = "flow_show",
        description = "Show one flow item with its dependencies, contributes-to edges, and notes.",
        annotations(
            title = "Show one item",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_show(&self, Parameters(p): Parameters<FlowShowParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::show(db, &start, &p))
    }

    /// Items, optionally filtered by status and/or type; id-ordered by default.
    #[tool(
        name = "flow_list",
        description = "List flow items, optionally filtered by status and/or type. Id-ordered by \
                       default; pass sort=rank for the ranked order.",
        annotations(title = "List items", read_only_hint = true, open_world_hint = false)
    )]
    fn flow_list(&self, Parameters(p): Parameters<FlowListParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::list(db, &start, &p))
    }

    /// The active plugin's type system + field model.
    #[tool(
        name = "flow_schema",
        description = "Show the active plugin's schema: the valid item types and their containment \
                       roles (which types are containers, each type's allowed parents, parent \
                       cardinality, and the hierarchy depth limit), plus the create/update field \
                       model (labels, required/settable, flags). Call this to learn which types \
                       exist here before creating items.",
        annotations(
            title = "Show plugin schema",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_schema(&self, Parameters(p): Parameters<FlowSchemaParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::schema(db, &start, &p))
    }

    // ---- remaining read tools (#nes) ------------------------------------------------------------
    //
    // The rest of the read surface — blocked/search/note_list/mention_list — each a thin shell over
    // the SAME `read::*` compute + render the `nxf` CLI serializes under `--json`, so the cross-seam
    // parity tests compare like for like. Read annotations throughout: `read_only_hint = true`,
    // `open_world_hint = false` (the closed local board). flow_next already covers the ready set (its
    // ready verb was folded into next by #4ti), so it is not part of this slice.

    /// Blocked items with their open blockers.
    #[tool(
        name = "flow_blocked",
        description = "List blocked flow items — open, held up by an open blocker or in a cycle — \
                       each with the open blockers holding it up. Ranked by default; pass sort=id \
                       for id order.",
        annotations(
            title = "List blocked work",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_blocked(&self, Parameters(p): Parameters<FlowBlockedParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::blocked(db, &start, &p))
    }

    /// Substring search over title/description/design/DoD/notes, lane-ranked.
    #[tool(
        name = "flow_search",
        description = "Search flow items by a case-insensitive substring over title, description, \
                       design, DoD, and notes. Results are grouped by lane priority \
                       (ready/in-progress → blocked → deferred → closed); archived excluded by \
                       default. Optional status/type filters and a sort override.",
        annotations(title = "Search items", read_only_hint = true, open_world_hint = false)
    )]
    fn flow_search(&self, Parameters(p): Parameters<FlowSearchParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::search(db, &start, &p))
    }

    /// One item's worklog notes, oldest-first.
    #[tool(
        name = "flow_note_list",
        description = "List a flow item's worklog notes (the change/worklog stream), oldest-first, \
                       as {id, body} records.",
        annotations(title = "List notes", read_only_hint = true, open_world_hint = false)
    )]
    fn flow_note_list(&self, Parameters(p): Parameters<FlowItemRefParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::note_list(db, &start, &p))
    }

    /// The short-ids one item cites in its free text.
    #[tool(
        name = "flow_mention_list",
        description = "List the short-ids a flow item mentions (cites in its free text), sorted. A \
                       plain array of the cited ids.",
        annotations(
            title = "List mentions",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_mention_list(&self, Parameters(p): Parameters<FlowItemRefParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::mention_list(db, &start, &p))
    }

    // ---- write tools (#wpw / #yie) --------------------------------------------------------------
    //
    // Each is a thin shell over a `flow::<op>` facade write, returning the canonical receipt as
    // `structuredContent` (byte-identical to `nxf <cmd> --json`). The annotations are per-op (epic
    // invariant): `read_only_hint = false` on every mutation; `destructive_hint` is `false` for
    // purely additive ops (create a new item, add an edge, append a note) and `true` for ops that
    // overwrite an existing field value or remove an edge; `idempotent_hint` is `true` where calling
    // again with the same args lands the same state, `false` for the minting ops (create, note_add).
    // `open_world_hint = false` throughout — the board is a closed local domain. `actor` is resolved
    // through the hybrid (per-tool override > launch default > env fallback) before the facade call.

    /// Create a new flow item.
    #[tool(
        name = "flow_create",
        description = "Create a new flow item (type, title, description, priority required; \
                       design/dod/due/defer/parent/depends_on optional). Returns the new item's \
                       canonical record.",
        annotations(
            title = "Create item",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    fn flow_create(&self, Parameters(p): Parameters<FlowCreateParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::create(db, &start, &actor, &p))
    }

    /// Set fields on an existing item.
    #[tool(
        name = "flow_update",
        description = "Update fields on an existing flow item. `set` is a list of `field=value` \
                       assignments (e.g. status=in_progress, priority=P0). Returns the item's \
                       canonical record.",
        annotations(
            title = "Update item",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_update(&self, Parameters(p): Parameters<FlowUpdateParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::update(db, &start, &actor, &p))
    }

    /// Mark an item in progress (and optionally assign it).
    #[tool(
        name = "flow_claim",
        description = "Claim a flow item — mark it in progress, optionally assigning it. Returns \
                       the item's canonical record.",
        annotations(
            title = "Claim item",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_claim(&self, Parameters(p): Parameters<FlowClaimParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::claim(db, &start, &actor, &p))
    }

    /// Close an item with a mandatory closing comment.
    #[tool(
        name = "flow_close",
        description = "Close a flow item with a mandatory closing comment (`reason`). Closing is \
                       how an item leaves the board. Returns the item's canonical record.",
        annotations(
            title = "Close item",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_close(&self, Parameters(p): Parameters<FlowCloseParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::close(db, &start, &actor, &p))
    }

    /// Add a `from -> to` dependency (from depends on to).
    #[tool(
        name = "flow_dep_add",
        description = "Add a dependency: `from` depends on `to`. Rejected if it would create a \
                       cycle. Returns a receipt.",
        annotations(
            title = "Add dependency",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_dep_add(&self, Parameters(p): Parameters<FlowEdgeParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::dep_add(db, &start, &actor, &p))
    }

    /// Remove a `from -> to` dependency.
    #[tool(
        name = "flow_dep_remove",
        description = "Remove the `from -> to` dependency. Returns a receipt.",
        annotations(
            title = "Remove dependency",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_dep_remove(&self, Parameters(p): Parameters<FlowEdgeParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::dep_remove(db, &start, &actor, &p))
    }

    /// Record a `from -> to` reference edge (from's text cites to).
    #[tool(
        name = "flow_mention_add",
        description = "Record a reference: `from`'s text cites `to`. A pure citation (never blocks). \
                       Returns a receipt.",
        annotations(
            title = "Add mention",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_mention_add(&self, Parameters(p): Parameters<FlowEdgeParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::mention_add(db, &start, &actor, &p))
    }

    /// Remove a `from -> to` reference edge.
    #[tool(
        name = "flow_mention_remove",
        description = "Remove the `from -> to` reference edge. Returns a receipt.",
        annotations(
            title = "Remove mention",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_mention_remove(&self, Parameters(p): Parameters<FlowEdgeParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::mention_remove(db, &start, &actor, &p))
    }

    /// Append an immutable worklog note to an item.
    #[tool(
        name = "flow_note_add",
        description = "Append an immutable worklog note to a flow item. Returns the new note's id \
                       and body.",
        annotations(
            title = "Add note",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    fn flow_note_add(&self, Parameters(p): Parameters<FlowNoteAddParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::note_add(db, &start, &actor, &p))
    }

    // ---- remaining write tools (#76u.12) --------------------------------------------------------
    //
    // The rest of the shared facade write surface (contributes-to add/remove + the archive/unarchive
    // batch), each over the existing `write::<op>` fn with the same receipt as the CLI. Annotations
    // follow the per-op rule (#76u.11): `read_only_hint = false` throughout; `destructive_hint` is
    // `false` for the additive contributes-add edge and `true` for the contributes-remove edge and
    // both archive ops (they overwrite the `archived` field / hide items); `idempotent_hint = true`
    // for all four (re-applying lands the same state — re-archiving keeps the original instant).
    // `open_world_hint = false` — the closed local board.

    /// Record a `from -> to` contributes-to edge (from contributes to to; n:m, never blocks).
    #[tool(
        name = "flow_contributes_add",
        description = "Record a contributes-to edge: `from` contributes to `to` (n:m; never blocks, \
                       so a self-edge is allowed). Returns a receipt.",
        annotations(
            title = "Add contributes-to",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_contributes_add(&self, Parameters(p): Parameters<FlowEdgeParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::contributes_add(db, &start, &actor, &p))
    }

    /// Remove a `from -> to` contributes-to edge.
    #[tool(
        name = "flow_contributes_remove",
        description = "Remove the `from -> to` contributes-to edge. Returns a receipt.",
        annotations(
            title = "Remove contributes-to",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_contributes_remove(&self, Parameters(p): Parameters<FlowEdgeParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::contributes_remove(db, &start, &actor, &p))
    }

    // ---- labels (h89s.3) ------------------------------------------------------------------------
    //
    // The user-vocabulary OR-set (h89s) exposed as MCP tools, each byte-identical to its
    // `nxf label <cmd> --json`. `label_add` is additive/idempotent (re-adding lands the same state),
    // `label_remove` is destructive (observed-remove); both `idempotent_hint = true`, `list` is
    // read-only. `open_world_hint = false` — the closed local board.

    /// Attach a user label to an item.
    #[tool(
        name = "flow_label_add",
        description = "Attach a user label (free-text tag) to a flow item. The label is trimmed; a \
                       blank label is rejected. Idempotent. Returns a receipt.",
        annotations(
            title = "Add label",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_label_add(&self, Parameters(p): Parameters<FlowLabelParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::label_add(db, &start, &actor, &p))
    }

    /// Detach a user label from an item (observed-remove).
    #[tool(
        name = "flow_label_remove",
        description = "Detach a user label from a flow item (observed-remove). Returns a receipt.",
        annotations(
            title = "Remove label",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_label_remove(&self, Parameters(p): Parameters<FlowLabelParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::label_remove(db, &start, &actor, &p))
    }

    /// List an item's user labels (sorted).
    #[tool(
        name = "flow_label_list",
        description = "List a flow item's user labels (sorted). Returns a JSON array of the labels.",
        annotations(title = "List labels", read_only_hint = true, open_world_hint = false)
    )]
    fn flow_label_list(&self, Parameters(p): Parameters<FlowItemRefParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::label_list(db, &start, &p))
    }

    /// Archive a batch of roots, cascading down each fully-closed subtree.
    #[tool(
        name = "flow_archive",
        description = "Archive one or more items — cascade DOWN each fully-closed subtree (a root \
                       must be closed with every descendant closed). Reversible via flow_unarchive. \
                       A partial batch: each root's outcome is in the receipt, never an error.",
        annotations(
            title = "Archive items",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_archive(&self, Parameters(p): Parameters<FlowArchiveParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::archive(db, &start, &actor, &p))
    }

    /// Unarchive a batch of roots, cascading up each item's ancestor chain.
    #[tool(
        name = "flow_unarchive",
        description = "Unarchive one or more items, making them visible again — cascade UP only \
                       (each item and its ancestor chain resurface; children/siblings stay archived). \
                       A partial batch: each root's outcome is in the receipt, never an error.",
        annotations(
            title = "Unarchive items",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn flow_unarchive(&self, Parameters(p): Parameters<FlowArchiveParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(flow::unarchive(db, &start, &actor, &p))
    }

    /// The registered workspaces (0jq8) — an UMBRELLA tool, not a flow op: it rides the always-on
    /// base router but reads the GLOBAL `~/.nexusflow/workspaces.toml` registry, independent of the
    /// launch workspace and its active modules. A host calls it to learn the `{name, path}` records,
    /// then feeds a `path` back through any tool's per-call `workspace` override — selecting a board
    /// without knowing paths. Its `structuredContent.items` is byte-identical to
    /// `nxs mcp workspaces --json` (the shared [`registry::list_value`] builder).
    ///
    /// Trust model (0jq8 Integrity #2): under the local-stdio trusted-host model this deliberately
    /// discloses EVERY registered path to any connected client, regardless of the launch workspace's
    /// scope — the registry is a convenience index a local host reads, not an access-control boundary.
    #[tool(
        name = "list_workspaces",
        description = "List the nexus-flow workspaces registered with this service instance \
                       (~/.nexusflow/workspaces.toml, or ~/.nexusflow-<name>/ when this is a \
                       development BUILD that NXS_SERVICE_INSTANCE names) as {name, path} \
                       records. Use a returned \
                       path as the per-call `workspace` argument on any tool to target that board \
                       without knowing its path. Entries are added by \
                       `nxs mcp install --workspace <path>`.",
        annotations(
            title = "List registered workspaces",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn list_workspaces(&self, Parameters(_p): Parameters<ListWorkspacesParams>) -> CallToolResult {
        self.tool_result(registry::list_value())
    }
}

// ---- memory tools (#76u.2 read / #76u.3 write) --------------------------------------------------
//
// Merged into the router only when the launch workspace has the memory module active (see `new`).
// Each is a thin shell over a `memory::<op>` facade call returning the canonical record as
// `structuredContent` — byte-identical to `nxm <cmd> --json` (proven by the cross-seam parity tests
// #76u.4). The reads carry `readOnlyHint = true`; the writes `read_only_hint = false` with per-op
// destructive/idempotent hints: `memory_add` is content-addressed capture (additive, idempotent —
// the auto-key is a pure hash of the text); `memory_update` overwrites the body under a key
// (destructive, idempotent upsert); `memory_close` forgets, removing it from the active set
// (destructive, idempotent); `memory_classify`/`memory_reorder` (6j6v.9a1r) leave the memory's text
// alone but OVERWRITE the classification/order registers in place, which is destructive by the same
// per-op rule the flow tools follow. `open_world_hint = false` throughout — memory is a closed local
// store.
// `actor` resolves through the shared hybrid before the write; `now` defaults like the flow writes'.
#[rmcp::tool_router(router = memory_router)]
impl NxsMcp {
    /// All active memories, key-sorted — the flat session-start dump.
    #[tool(
        name = "memory_list",
        description = "List all active durable memories (facts the agent recalls across sessions), \
                       key-sorted. The flat session-start dump; use memory_search to filter.",
        annotations(
            title = "List memories",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn memory_list(&self, Parameters(p): Parameters<MemoryListParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(memory::list(db, &start, &p))
    }

    /// Active memories matching a case-insensitive substring over key + body.
    #[tool(
        name = "memory_search",
        description = "Search active memories by a case-insensitive substring over key and body. \
                       Returns the matching memory records.",
        annotations(
            title = "Search memories",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn memory_search(&self, Parameters(p): Parameters<MemorySearchParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(memory::search(db, &start, &p))
    }

    /// One memory's full record by key.
    #[tool(
        name = "memory_show",
        description = "Show one memory's full record (key, body, author, updated) by its key.",
        annotations(
            title = "Show one memory",
            read_only_hint = true,
            open_world_hint = false
        )
    )]
    fn memory_show(&self, Parameters(p): Parameters<MemoryShowParams>) -> CallToolResult {
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(memory::show(db, &start, &p))
    }

    /// Capture a new fact under a content-hash auto-key.
    #[tool(
        name = "memory_add",
        description = "Capture a new durable fact. The key is a content hash of the text (re-adding \
                       identical text is a no-op); use memory_update to evolve a fact under a stable \
                       key. Returns the memory's record.",
        annotations(
            title = "Add memory",
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn memory_add(&self, Parameters(p): Parameters<MemoryAddParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(memory::add(db, &start, &actor, &p))
    }

    /// Upsert a fact in place under an explicit, stable key.
    #[tool(
        name = "memory_update",
        description = "Update (upsert) the fact stored under a stable key — overwrites its body in \
                       place. Returns the memory's record.",
        annotations(
            title = "Update memory",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn memory_update(&self, Parameters(p): Parameters<MemoryUpdateParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(memory::update(db, &start, &actor, &p))
    }

    /// File an existing memory — category, reach and/or references — without touching its text.
    #[tool(
        name = "memory_classify",
        description = "File an existing memory without changing its text: set its category \
                       (section slug), its scope/reach (item | project | global) and/or the board \
                       item ids it is about. At least one is required; an omitted one is left as \
                       it was. The reach decides where the memory surfaces — project/global read \
                       at session start, item reads on the board items in refs. Returns the \
                       memory's record.",
        annotations(
            title = "Classify memory",
            read_only_hint = false,
            // PR #282 review, Code Quality #4: `true` by this module's OWN per-op rule — an op that
            // OVERWRITES an existing field value is destructive, and each of the three registers
            // this writes replaces a curatorial judgement in place (the same reason `memory_update`
            // carries it for the body). The memory's text is untouched, which is a different claim.
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn memory_classify(&self, Parameters(p): Parameters<MemoryClassifyParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(memory::classify(db, &start, &actor, &p))
    }

    /// Store an explicit reading order over the given memory keys.
    #[tool(
        name = "memory_reorder",
        description = "Store an explicit reading order: the first key becomes position 1, the \
                       second 2, and so on. Deliberate and rare — everyday writes never touch the \
                       order, and a memory that was never reordered sorts by insertion order after \
                       every placed one. Every key must exist; a rejected sequence leaves the order \
                       unchanged. Returns the records in the order just written.",
        annotations(
            title = "Reorder memories",
            read_only_hint = false,
            // Same rule, same answer: reordering overwrites the `ordinal` register of every key in
            // the sequence, and the positions they held are not recoverable from the result.
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn memory_reorder(&self, Parameters(p): Parameters<MemoryReorderParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(memory::reorder(db, &start, &actor, &p))
    }

    /// Forget a memory by key (a reversible tombstone).
    #[tool(
        name = "memory_close",
        description = "Forget a memory by key — a reversible tombstone (a later memory_add/update \
                       under the same key revives it). Returns a receipt.",
        annotations(
            title = "Forget memory",
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    fn memory_close(&self, Parameters(p): Parameters<MemoryCloseParams>) -> CallToolResult {
        let actor = self.resolve_actor(p.actor.as_deref());
        let (db, start) = self.effective(p.workspace.as_deref());
        self.tool_result(memory::close(db, &start, &actor, &p))
    }
}

impl NxsMcp {
    /// Resolve the actor for a mutation through the hybrid (#zxj): the per-tool `actor` argument
    /// wins, else this server instance's launch default (`--actor`/`NXS_ACTOR`), else the
    /// `NXF_ACTOR`/`USER` env fallback the CLI uses, else `"nxs"`. See [`pick_actor`].
    fn resolve_actor(&self, per_tool: Option<&str>) -> String {
        pick_actor(per_tool, self.launch_actor.as_deref(), env_actor())
    }

    /// The directory to resolve `.nxs/` from: the resolved launch workspace (#76u.7). The `.`
    /// fallback is reached ONLY on the `--db` path, where [`Workspace::resolve`] ignores `start`
    /// (the override names the db directly) — so it never triggers a cwd walk-up for this persona.
    fn start_dir(&self) -> PathBuf {
        self.workspace
            .clone()
            .unwrap_or_else(|| Path::new(".").to_path_buf())
    }

    /// The `(db, start-dir)` THIS call resolves against — the S7/ppa per-tool `workspace` override on
    /// top of the launch default (the second branch of the hybrid discovery). A non-empty per-tool
    /// `workspace` names a DIFFERENT board, so resolve it directly and DROP the launch `--db` (which
    /// pinned the launch default's store): one launched server can then serve many workspaces without
    /// a registry. Empty/omitted ⇒ the launch default (its `--db` + pinned root), exactly as before.
    ///
    /// The override resolves the named root like any workspace (open-per-call [`Workspace::resolve`]),
    /// with NO auto-init. Resolution WALKS UP from the named root (CLI discovery semantics), so a path
    /// whose own dir has no `.nxs/` but an ANCESTOR does binds that ancestor board — only a path with
    /// no `.nxs/` in itself OR any ancestor surfaces as a `no_workspace` DOMAIN error on the tool
    /// result (via [`tool_result`](Self::tool_result)), the same "run `nxs init`" contract as an
    /// explicit `--workspace`. That is distinct from a missing launch DEFAULT, which fails earlier at
    /// [`serve`] start — the two `no_workspace` paths the ticket asks to keep separate. An empty
    /// string is treated as unset (like the actor hybrid), so `workspace: ""` falls to the default.
    fn effective(&self, per_tool: Option<&str>) -> (Option<&str>, PathBuf) {
        match per_tool.filter(|s| !s.is_empty()) {
            Some(ws) => (None, PathBuf::from(ws)),
            None => (self.db.as_deref(), self.start_dir()),
        }
    }

    /// Compute the connect-time instructions (#76u.1): the umbrella `nxs prime` fan-out over the
    /// launch workspace's ACTIVE modules — the SAME source as `nxs prime` — rendered for the MCP seam
    /// (command references mapped to tool names where a tool exists). `None` when there is no launch
    /// workspace (a clean start, no crash) or the fan-out fails. `NXS_NOW` pins it so the instructions
    /// are byte-stable.
    fn compute_instructions(&self) -> Option<String> {
        // No launch workspace is the normal "nothing to prime" case — return silently.
        let ws = Workspace::resolve(self.db.as_deref(), &self.start_dir()).ok()?;
        // A workspace that resolves but won't prime is UNEXPECTED (a module's prime failed, a corrupt
        // store, …) — surface it on STDERR (stdout is the MCP protocol channel) so a
        // connects-but-never-primes server is diagnosable, then start without instructions rather
        // than failing the handshake. Read tools still report their own per-call errors.
        match crate::prime::fan_out(&ws, true) {
            Ok(fan) => Some(render_instructions(&fan, &self.tool_router.list_all())),
            Err(e) => {
                eprintln!(
                    "nxs mcp: prime fan-out failed; serving without instructions: {}",
                    e.msg
                );
                None
            }
        }
    }

    /// Map a facade read OR write outcome to an MCP tool result — the shared mapper for every flow
    /// AND memory tool: success carries the canonical record (or write receipt) as `structuredContent`
    /// plus a compact text projection; a domain failure becomes `isError = true` with the
    /// `{ error: { kind, msg } }` envelope, never a protocol error. `kind` is the foundation's closed
    /// [`ErrorKind`](nxs_foundation::error::ErrorKind) verbatim; the full set can surface here —
    /// `no_workspace`/`not_found`/`validation`/`io` on any tool, plus the write-path `cycle`/`conflict`
    /// on a mutation (e.g. a `flow_dep_add` cycle). Only `verification` never reaches a tool.
    ///
    /// The success value is run through [`as_structured_object`] first: MCP requires
    /// `structuredContent` to be a JSON OBJECT, but the list-shaped tools (`flow_list`/`flow_next`,
    /// `memory_list`/`memory_search`) return canonical ARRAYS, so those are wrapped under `items`
    /// (#76u.9). The text projection mirrors the same shape, so a host reading either channel sees one
    /// consistent record.
    fn tool_result(&self, outcome: Result<Value, NxfError>) -> CallToolResult {
        match outcome {
            Ok(value) => {
                let value = as_structured_object(value);
                let mut result = CallToolResult::success(vec![Content::text(value.to_string())]);
                result.structured_content = Some(value);
                result
            }
            Err(e) => {
                let envelope =
                    json!({ "error": { "kind": e.kind.as_str(), "msg": e.msg.clone() } });
                let mut result = CallToolResult::error(vec![Content::text(e.msg)]);
                result.structured_content = Some(envelope);
                result
            }
        }
    }
}

/// Coerce a canonical record into the JSON OBJECT shape MCP requires for
/// `CallToolResult.structuredContent` (#76u.9). An OBJECT (`flow_show`/`memory_show`, a write receipt,
/// the error envelope) passes through untouched; ANYTHING ELSE is wrapped under a single `items` key,
/// since strict hosts (Claude Cowork/Desktop) reject a non-object `structuredContent` as missing. In
/// practice the only non-objects the tools yield are the top-level ARRAYS of the list-shaped reads —
/// `flow_list`/`flow_next` (≙ `nxf list/next --json`) and `memory_list`/`memory_search` (≙ `nxm
/// memories --json`); matching on "not an object" rather than "is an array" keeps the helper TOTAL, so
/// a stray scalar/`null` can never slip through and silently violate the very object contract this
/// helper exists to enforce.
///
/// This consciously REFINES the epic parity invariant (#76u, `structuredContent` ≙ `<module> <cmd>
/// --json`) for list-shaped ops: the canonical array is preserved byte-for-byte UNDER `items`, so the
/// cross-seam parity tests compare `structuredContent.items` to the CLI array for those tools, and
/// `structuredContent` directly for the object-shaped reads/receipts. Wrapping at the seam (not in
/// [`flow`]/[`memory`]) keeps the facade results literally CLI-identical and isolates the MCP-spec
/// shim to one place — any future array-returning tool complies automatically.
fn as_structured_object(value: Value) -> Value {
    match value {
        Value::Object(_) => value,
        other => json!({ "items": other }),
    }
}

#[rmcp::tool_handler(router = self.tool_router)]
impl ServerHandler for NxsMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::new(ServerCapabilities::builder().enable_tools().build());
        let mut imp = Implementation::from_build_env();
        imp.name = "nxs".to_string();
        imp.version = env!("CARGO_PKG_VERSION").to_string();
        imp.title = Some("nexus-flow".to_string());
        info.server_info = imp;
        // The SessionStart-hook equivalent for MCP hosts (#76u.1, #76u.5): STATIC usage guidance —
        // the nudge, the tool reference, and each active module's workflow rules — pushed at connect.
        // No live snapshot: `instructions` are delivered once at initialize (= host start for stdio)
        // and would go stale, so the live ready set comes from `flow_next`/`flow_list` instead.
        // Absent a launch workspace, start cleanly with no instructions (never crash).
        info.instructions = self.compute_instructions();
        info
    }
}

/// Render the static MCP `instructions` (#76u.5): a session-start nudge first, then the tool
/// reference (named, drift-free from the router), then each active module's workflow RULES from its
/// prime record — the static "how to use nxs" part of `nxs prime`. A rule's command references are
/// rendered as tool names where a matching tool exists, else left as CLI (ctm — see
/// [`seamify_commands`]). No live ready/blocked/next data: that goes stale between chats, and the
/// live set is always one `flow_next` call away.
fn render_instructions(fan: &PrimeFanOut, tools: &[Tool]) -> String {
    // The registered tool names, the source of truth for command→tool rendering (ctm): a rule's
    // command reference becomes a tool name ONLY when that tool actually exists here.
    let tool_names: HashSet<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let mut out = String::new();
    out.push_str("# nexus-flow (MCP)\n\n");
    // Nudge FIRST and prominent: instructions are static, so each chat must pull the live set via the
    // tool(s) before answering. Name `memory_list` too once the memory surface is active (#76u.2) —
    // it is the flat session-start dump of durable facts (`memory_show`/`nxm recall` is key-based and
    // unfit for that). The reference stays honest (ctm): `memory_list` is named only when registered.
    if tool_names.contains("memory_list") {
        out.push_str(
            "**Before you answer, call `flow_next` and `memory_list` first** to load the current \
             ready work and your durable project memories — these instructions are static usage \
             guidance, not a live snapshot.\n\n",
        );
    } else {
        out.push_str(
            "**Before you answer, call `flow_next` first** to load the current ready work — these \
             instructions are static usage guidance, not a live snapshot.\n\n",
        );
    }

    out.push_str("## Tools\n");
    for t in tools {
        let desc = t.description.as_deref().unwrap_or("");
        let _ = writeln!(out, "- `{}` — {desc}", t.name);
    }

    // Each active module's workflow rules, from its prime record (the static rules part of
    // `nxs prime`) — command references translated to the MCP tool-name convention.
    for module in &fan.modules {
        let Ok(report) = serde_json::from_str::<Value>(module.raw.trim()) else {
            // Deliberate skip: the fan-out runs `<module> prime --json`, so each `raw` is always a
            // JSON record — this is unreachable today. If a module ever emits non-JSON, omit that
            // module's section rather than fail the whole initialize handshake.
            continue;
        };
        let Some(rules) = report["rules"].as_array() else {
            continue;
        };
        let _ = write!(out, "\n## {} workflow\n", module.module);
        for rule in rules {
            if let Some(text) = rule.as_str() {
                let _ = writeln!(out, "- {}", seamify_commands(text, &tool_names));
            }
        }
    }
    out
}

/// Render the CLI command references in a rule as MCP TOOL names — but ONLY where the active surface
/// actually exposes that tool (ctm). A `nxf …` / `nxm …` head maps to a `flow_*` / `memory_*` tool
/// exactly when that name is a REGISTERED tool (`tools`); every other reference is left in its honest
/// CLI form. The earlier blind prefix-swap rewrote EVERY command into `flow_*` names, inventing tools
/// that did not exist; mapping against the live tool set keeps the seam invariant honest: every
/// `flow_*`/`memory_*` token this emits is a real tool, and a command with no tool reads as valid CLI.
///
/// Multi-word commands fold into the tool's underscored name: a CLI head can be several
/// space-separated verb tokens (`nxf note add`, `nxf dep add`, `nxf mention remove`), whose tool — if
/// any — is `<tool-prefix><tok1>_<tok2>…` ([`tool_candidate`]). The LONGEST registered candidate wins
/// (so `nxf dep add` → `flow_dep_add` even though `flow_dep` is also a syntactic candidate), and only
/// that token run is consumed — trailing args stay verbatim. With no candidate registered the head
/// stays CLI, so a host without that tool still reads a valid command (and the ctm honesty holds).
fn seamify_commands(s: &str, tools: &HashSet<&str>) -> String {
    // CLI binary prefix → tool namespace. A command head is `<cli-prefix><verb-run>`.
    const PAIRS: &[(&str, &str)] = &[("nxf ", "flow_"), ("nxm ", "memory_")];
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    'scan: while i < s.len() {
        let rest = &s[i..];
        for (cli, tool_ns) in PAIRS {
            if let Some(after) = rest.strip_prefix(cli) {
                match tool_candidate(after, tool_ns, tools) {
                    // A real tool: collapse `<cli> <verb-run>` → `<tool>` (trailing args stay).
                    Some((tool, verb_bytes)) => {
                        out.push_str(&tool);
                        i += cli.len() + verb_bytes;
                    }
                    // No tool for this command: keep the CLI prefix verbatim (the verb + rest are
                    // copied by the normal scan), so the reference stays honest CLI — never a fake
                    // `flow_*`/`memory_*` tool name.
                    None => {
                        out.push_str(cli);
                        i += cli.len();
                    }
                }
                continue 'scan;
            }
        }
        let ch = rest.chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// The registered tool a CLI command head maps to, if any. `after` is the text right after the CLI
/// prefix (so it starts at the first verb token). The head is the maximal run of alphanumeric verb
/// tokens joined by single spaces (`note add`, `dep remove`); its candidate tools are
/// `<tool_ns><tok1>_…_<tokk>` for k = n..1. Returns the LONGEST candidate that is a registered tool
/// plus the number of bytes of `after` it consumes (the verb run, excluding trailing args), or `None`
/// when none is registered. Longest-first so `dep add` prefers `flow_dep_add` over `flow_dep`.
fn tool_candidate(after: &str, tool_ns: &str, tools: &HashSet<&str>) -> Option<(String, usize)> {
    // Byte-end of each alphanumeric verb token, while tokens are separated by exactly one space.
    let bytes = after.as_bytes();
    let mut ends = Vec::new();
    let mut pos = 0;
    loop {
        let start = pos;
        while pos < bytes.len() && bytes[pos].is_ascii_alphanumeric() {
            pos += 1;
        }
        if pos == start {
            break; // no token here
        }
        ends.push(pos);
        // Continue the run only across a single space that is itself followed by another token.
        if pos + 1 < bytes.len() && bytes[pos] == b' ' && bytes[pos + 1].is_ascii_alphanumeric() {
            pos += 1;
        } else {
            break;
        }
    }
    // Longest candidate first: k tokens span `after[..ends[k-1]]`, spaces → `_`.
    for &end in ends.iter().rev() {
        let candidate = format!("{tool_ns}{}", after[..end].replace(' ', "_"));
        if tools.contains(candidate.as_str()) {
            return Some((candidate, end));
        }
    }
    None
}

/// Resolve the `actor` recorded on a mutation's ops (E9 #zxj) from the actor HYBRID, never ambient
/// process state: a per-tool `actor` argument wins, else the server-instance launch default
/// (`--actor`/`NXS_ACTOR`), else the `NXF_ACTOR`/`USER` env fallback the CLI uses, else `"nxs"`. A
/// long-lived server started by a host must be able to attribute each write to its real author — a
/// per-call override above one instance default — rather than write every op under one identity.
///
/// A BLANK value at any tier is treated as unset and falls through to the next: a host tool call may
/// pass `actor: ""` or `actor: " "` (the field is `#[serde(default)]` and carries no schema
/// constraint, so it is arbitrary host-supplied text), and a blank `--actor " "`/`NXS_ACTOR=` is the
/// same. The rule is the substrate's [`resolve_author`](nxs_foundation::model::resolve_author) —
/// applied at EVERY tier, not just the env one. It has to be: the per-tool tier is the only one an
/// MCP client controls per call, and a whitespace-only value that slipped through would reach
/// `Store::emit`'s non-blank assert and panic the request rather than author a bad op.
///
/// Pure over all three inputs (the env fallback is resolved by [`env_actor`] and passed in) so the
/// precedence is unit-testable without touching process env.
fn pick_actor(
    per_tool: Option<&str>,
    launch: Option<&str>,
    env_fallback: Option<String>,
) -> String {
    nxs_foundation::model::resolve_author(
        // `resolve_author` takes two tiers; this hybrid has three, so fold the first two through it
        // first. Passing the empty sentinel keeps "still absent" representable for the second fold.
        Some(nxs_foundation::model::resolve_author(
            per_tool.map(str::to_string),
            launch.map(str::to_string),
            "",
        )),
        env_fallback,
        "nxs",
    )
}

/// The ambient actor fallback at the bottom of the [`pick_actor`] hybrid: the CLI's `NXF_ACTOR` →
/// `$USER` chain, so a write with no explicit actor lands the SAME identity an `nxf <cmd>` would
/// (cross-seam parity). Empty values are treated as unset so a blank env var never authors an op. The
/// ONE deliberate divergence from the CLI's `actor()` is the final default: when neither var is set
/// [`pick_actor`] substitutes `"nxs"` (this server's identity), not the CLI's `"nxf"`.
fn env_actor() -> Option<String> {
    // The blank-is-absent rule is the substrate's, shared with every product CLI's `actor()`
    // (invariant 1, 6j6v.xsf3). Sentinel-default rather than a second copy of the rule: this seam
    // must report ABSENCE so `pick_actor` can apply its own `"nxs"` default, which the CLIs' `"nxf"`
    // deliberately differs from.
    const ABSENT: &str = "";
    let resolved = nxs_foundation::model::resolve_author(
        std::env::var("NXF_ACTOR").ok(),
        std::env::var("USER").ok(),
        ABSENT,
    );
    Some(resolved).filter(|s| !s.is_empty())
}

/// The reverse-DNS identifier of the standalone MCP server's default workspace (#76u.7). This is a
/// NEUTRAL, nxs-owned identifier — the OSS server's default board deliberately does NOT live under
/// any consumer's app-data home (4cmg). Earlier this was the nexflow.it app's `it.nexflow.app` so
/// the two shared one board; that coupled the OSS default to a specific proprietary consumer (its
/// plugin would have to be known there), so the default is now consumer-agnostic. A consumer that
/// wants its own board installs its OWN MCP entry (its `--workspace`/`--db`) which overrides this
/// default and imports this board's data — see `crates/nxs/src/mcp/install.rs` and the nexflow.it
/// app's onboarding. Nothing in another repo is pinned to this literal anymore; the test below is a
/// tripwire so a rename is a deliberate edit (it changes the on-disk path of every existing board).
const APP_DATA_IDENTIFIER: &str = "com.nxsflow.nxs";

/// The MCP persona's default workspace root (#76u.7): a stable per-user directory under the platform
/// data dir, joined with the neutral [`APP_DATA_IDENTIFIER`]. Not tied to any consumer app (4cmg).
///
/// We deliberately use [`BaseDirs::data_dir`](directories::BaseDirs::data_dir), NOT `ProjectDirs`:
/// `ProjectDirs::from("com","nxsflow","nxs")` applies PLATFORM-SPECIFIC naming — full reverse-DNS
/// only on macOS, `nxsflow\nxs` on Windows, bare `nxs` on Linux — so it would resolve to a DIFFERENT
/// (and on Linux, generic/collision-prone) directory per platform. `BaseDirs::data_dir()` is the
/// same `dirs::data_dir()` every platform's data-dir convention uses, so joining the literal
/// identifier yields a single, predictable path on every target:
/// - macOS:   `~/Library/Application Support/com.nxsflow.nxs/`
/// - Linux:   `$XDG_DATA_HOME/com.nxsflow.nxs/` (else `~/.local/share/com.nxsflow.nxs/`)
/// - Windows: `%APPDATA%\com.nxsflow.nxs\` (Roaming)
///
/// The result is the workspace root; the server's `.nxs/` lives directly under it.
fn app_data_home() -> crate::error::Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_dir().join(APP_DATA_IDENTIFIER))
        .ok_or_else(|| {
            NxfError::io("could not resolve a home directory for the default MCP workspace")
        })
}

/// Resolve the launch workspace ROOT for `nxs mcp serve`, per the MCP persona's resolution tiers,
/// auto-initializing the default on first start (#76u.7). cwd discovery is intentionally NOT a tier
/// here — MCP hosts have no useful cwd, so the CLI's "the project I'm standing in" walk-up is wrong
/// for this persona and is dropped:
///
/// 1. `--db`/`NXS_DB` — the store override resolves the workspace directly (its parent dir), so no
///    root is needed: returns `None` and the per-call [`Workspace::resolve`] does the rest.
/// 2. `--workspace`/`NXS_WORKSPACE` — the explicit launch root. Honored verbatim; a MISSING `.nxs/`
///    there still errors per call ("run `nxs init`"). The auto-init contract below changes only for
///    the server's OWN App-Data-Home, never a user-named path.
/// 3. App-Data-Home (the default) — a stable per-user dir; `.nxs/` is AUTO-INITIALIZED there on
///    first start so the server "just works" under a host that passes no flags.
fn launch_workspace(
    db: Option<&str>,
    workspace: Option<&str>,
) -> crate::error::Result<Option<PathBuf>> {
    launch_workspace_in(db, workspace, app_data_home)
}

/// [`launch_workspace`] with the App-Data-Home resolver injected, so the tiering + auto-init is
/// unit-testable against a tempdir without touching the real user home. The resolver is consulted
/// (and auto-init runs) ONLY on the default tier — `--db`/`--workspace` never reach it.
fn launch_workspace_in(
    db: Option<&str>,
    workspace: Option<&str>,
    app_data_home: impl FnOnce() -> crate::error::Result<PathBuf>,
) -> crate::error::Result<Option<PathBuf>> {
    if db.is_some() {
        return Ok(None);
    }
    if let Some(ws) = workspace {
        return Ok(Some(PathBuf::from(ws)));
    }
    let root = app_data_home()?;
    // Auto-init: the App-Data-Home is the server's OWN managed workspace, so a first start
    // materializes `.nxs/` there (idempotent `setup`, never clobbers an existing board) instead of
    // erroring. Seeded with flow active + the default plugin — the same shape `nxs init` lands on
    // non-interactively — so the read tools + prime instructions work out of the box.
    use nexus_flow_facade::workspace as fws;
    nxs_foundation::workspace::setup(&root, &fws::flow_config(fws::DEFAULT_PLUGIN)).map_err(
        |e| {
            NxfError::io(format!(
                "initializing the default MCP workspace at {}: {}",
                root.display(),
                e.msg
            ))
        },
    )?;
    Ok(Some(root))
}

/// Whether the memory module is active in the launch workspace (#76u.2) — the fan-out gate deciding
/// if the `memory_*` tools are registered. Resolves the launch workspace's config and checks
/// `active_modules`; ANY resolution failure (no workspace yet, a clean start, the `--db` path naming
/// a fresh store) means "not active", so the memory surface is simply absent rather than a crash.
/// Read ONCE at start, like the prime fan-out reads the active set — a module activated mid-session
/// shows up on the next connect (the same staleness contract as the instructions).
fn memory_module_active(db: Option<&str>, workspace: Option<&Path>) -> bool {
    let start = workspace
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(".").to_path_buf());
    Workspace::resolve(db, &start)
        .map(|ws| {
            ws.config
                .active_modules
                .iter()
                .any(|m| m == nexus_memory::workspace::MEMORY_MODULE)
        })
        .unwrap_or(false)
}

/// Run an MCP server over stdio against the launch workspace (the `nxs mcp serve` body). Blocks
/// until the client disconnects. `db` is the `--db`/`NXS_DB` override; `workspace` the launch root;
/// `actor` the server-instance write identity default (#zxj), overridable per tool call.
pub fn serve(
    db: Option<&str>,
    workspace: Option<&str>,
    actor: Option<&str>,
) -> crate::error::Result<()> {
    // Scope the `main` SIG_DFL reset (4yt4) away from this long-running persona: restore SIG_IGN so
    // a host closing the stdio pipe surfaces as an EPIPE the rmcp transport turns into the
    // structured `NxfError::io` error + clean exit below, instead of killing the server by signal
    // before that path runs. Done before any protocol byte is written.
    crate::sigpipe::restore_ignore();
    let workspace = launch_workspace(db, workspace)?;
    // Fan-out gate (#76u.2): read the launch workspace's active modules ONCE at start (like the prime
    // fan-out) to decide whether the memory tools join the surface.
    let memory_active = memory_module_active(db, workspace.as_deref());
    let handler = NxsMcp::new(
        db.map(str::to_string),
        workspace,
        actor.map(str::to_string),
        memory_active,
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| NxfError::io(format!("starting the MCP runtime: {e}")))?;
    runtime.block_on(async move {
        let service = handler
            .serve(rmcp::transport::io::stdio())
            .await
            .map_err(|e| NxfError::io(format!("starting the MCP server: {e}")))?;
        service
            .waiting()
            .await
            .map_err(|e| NxfError::io(format!("serving MCP over stdio: {e}")))?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ---- #76u.9: the structuredContent object-shaping shim is TOTAL -------------------------------

    #[test]
    fn as_structured_object_passes_objects_through_and_wraps_everything_else() {
        // The MCP-spec shim must be TOTAL: an object passes through untouched; ANY non-object is
        // wrapped under `items`, so structuredContent can never end up a non-object (#76u.9 review —
        // closing the totality gap the array-only match left open).
        let obj = json!({ "item": { "id": "x" }, "deps": [] });
        assert_eq!(
            as_structured_object(obj.clone()),
            obj,
            "an object (flow_show / error envelope) passes through untouched"
        );
        assert_eq!(
            as_structured_object(json!([1, 2, 3])),
            json!({ "items": [1, 2, 3] }),
            "the list-shaped array (flow_list / flow_next) is wrapped under items"
        );
        // Defensive totality: a stray scalar/null cannot arise from the facade reads today, but the
        // helper must still never leak one as a non-object structuredContent.
        assert_eq!(as_structured_object(json!(42)), json!({ "items": 42 }));
        assert_eq!(as_structured_object(Value::Null), json!({ "items": null }));
    }

    // ---- #zxj: the actor hybrid (per-tool > launch default > env fallback > "nxs") ----------------

    #[test]
    fn pick_actor_prefers_the_per_tool_override() {
        // A per-tool `actor` argument wins over the launch default AND the env fallback — so a
        // long-lived server can attribute an individual write to its real author.
        assert_eq!(
            pick_actor(Some("alice"), Some("bob"), Some("carol".to_string())),
            "alice"
        );
    }

    #[test]
    fn pick_actor_falls_back_to_the_launch_default() {
        // No per-tool actor: the server-instance launch default (`--actor`/`NXS_ACTOR`) is used.
        assert_eq!(
            pick_actor(None, Some("bob"), Some("carol".to_string())),
            "bob"
        );
    }

    #[test]
    fn pick_actor_falls_back_to_the_env_actor_then_a_stable_default() {
        // Neither per-tool nor launch: the NXF_ACTOR/USER env fallback (the CLI's identity), else a
        // stable `"nxs"` so an op is never authored by the empty string.
        assert_eq!(pick_actor(None, None, Some("carol".to_string())), "carol");
        assert_eq!(pick_actor(None, None, None), "nxs");
    }

    #[test]
    fn pick_actor_treats_an_empty_override_as_unset() {
        // An untrusted host tool call may pass `actor: ""` (the field is `#[serde(default)]`), and a
        // blank launch `--actor ""`/`NXS_ACTOR=` is the same. Neither may author an op as the empty
        // string — the "never authored by the empty string" invariant — so an empty value falls
        // THROUGH to the next tier, exactly as `env_actor` already drops a blank env var.
        assert_eq!(
            pick_actor(Some(""), Some("bob"), Some("carol".to_string())),
            "bob",
            "an empty per-tool actor falls through to the launch default"
        );
        assert_eq!(
            pick_actor(Some(""), Some(""), Some("carol".to_string())),
            "carol",
            "an empty per-tool AND launch actor fall through to the env fallback"
        );
        assert_eq!(
            pick_actor(Some(""), Some(""), None),
            "nxs",
            "all tiers empty/absent ⇒ the stable default, never the empty string"
        );
    }

    #[test]
    fn pick_actor_treats_a_whitespace_only_override_as_unset() {
        // Review finding (Integrity #1, PR #311): the per-tool tier is arbitrary host-supplied text
        // with no schema constraint, and it is the ONLY tier an MCP client controls per call. A
        // whitespace-only value is as much a non-identity as an empty one — and since `Store::emit`
        // now asserts a non-blank author, letting one through would panic the request instead of
        // writing a bad op. Every tier applies the substrate's blank-is-absent rule.
        assert_eq!(
            pick_actor(Some("   "), Some("bob"), Some("carol".to_string())),
            "bob",
            "a whitespace-only per-tool actor falls through to the launch default"
        );
        assert_eq!(
            pick_actor(Some("\t"), Some(" \n "), Some("carol".to_string())),
            "carol",
            "whitespace-only per-tool AND launch actors fall through to the env fallback"
        );
        assert_eq!(
            pick_actor(Some(" "), Some(" "), None),
            "nxs",
            "all tiers blank ⇒ the stable default, never a blank author"
        );
    }

    // ---- #ppa: the per-tool workspace override (hybrid discovery) ---------------------------------

    #[test]
    fn effective_prefers_the_per_tool_workspace_and_drops_the_launch_db() {
        // A non-empty per-tool `workspace` names a DIFFERENT board (S7/ppa): resolve it directly and
        // DROP the launch --db (which pinned the launch default's store), so one launched server can
        // serve many workspaces without a registry.
        let handler = NxsMcp::new(
            Some("/launch/db.sqlite".to_string()),
            Some(PathBuf::from("/launch/ws")),
            None,
            false,
        );
        let (db, start) = handler.effective(Some("/other/project"));
        assert_eq!(db, None, "the override drops the launch --db");
        assert_eq!(start, PathBuf::from("/other/project"));
    }

    #[test]
    fn effective_without_an_override_uses_the_launch_default() {
        // Default-only path stays exactly as before: the launch --db + pinned workspace root.
        let handler = NxsMcp::new(None, Some(PathBuf::from("/launch/ws")), None, false);
        let (db, start) = handler.effective(None);
        assert_eq!(db, None);
        assert_eq!(start, PathBuf::from("/launch/ws"));
    }

    #[test]
    fn effective_treats_an_empty_override_as_unset() {
        // Like the actor hybrid, an empty per-tool `workspace: ""` is unset ⇒ the launch default,
        // never a resolve against "" (which would be a spurious cwd walk-up).
        let handler = NxsMcp::new(None, Some(PathBuf::from("/launch/ws")), None, false);
        let (_db, start) = handler.effective(Some(""));
        assert_eq!(start, PathBuf::from("/launch/ws"));
    }

    #[test]
    fn effective_on_the_launch_db_path_needs_no_root() {
        // On the launch --db path the workspace root is None; with no per-tool override the start dir
        // is the `.` sentinel (Workspace::resolve ignores it when a db override is set — never a real
        // cwd walk-up for this persona).
        let handler = NxsMcp::new(Some("/launch/db.sqlite".to_string()), None, None, false);
        let (db, start) = handler.effective(None);
        assert_eq!(db, Some("/launch/db.sqlite"));
        assert_eq!(start, PathBuf::from("."));
    }

    // ---- #76u.7: App-Data-Home as the default workspace tier + auto-init --------------------------

    #[test]
    fn app_data_home_is_the_neutral_nxs_identifier() {
        // 4cmg (#76u.7): the OSS default board lives under a NEUTRAL, nxs-owned identifier — never a
        // consumer's app-data home. (Earlier `it.nexflow.app`, shared with the nexflow.it app; that
        // coupled the OSS default to a specific proprietary consumer, so it was neutralized.)
        //
        // Two assertions, two distinct regressions:
        //  1. Pin the identifier against a HARD-CODED literal (not the const against itself): a
        //     co-located, fast tripwire so a rename is a DELIBERATE edit — it changes the on-disk
        //     path of every existing default board. The e2e test
        //     `default_workspace_auto_inits_app_data_home_not_cwd` (tests/mcp.rs) pins the same
        //     literal via the RESOLVED path, so a const edit does not go unnoticed — but this unit
        //     assert states the contract next to the const and needs no server spawn.
        assert_eq!(
            APP_DATA_IDENTIFIER, "com.nxsflow.nxs",
            "the default MCP workspace identifier is neutral + nxs-owned (4cmg); renaming it moves \
             every existing default board's on-disk path, so it must be a deliberate change"
        );
        //  2. The resolved root actually ends with that identifier (the join wiring is intact). Pure
        //     path resolution — touches no filesystem.
        let home = app_data_home().expect("resolves a data home under test");
        assert!(
            home.ends_with(APP_DATA_IDENTIFIER),
            "the default workspace root ends with the identifier `{APP_DATA_IDENTIFIER}`: {}",
            home.display()
        );
    }

    #[test]
    fn launch_workspace_defaults_to_app_data_home_and_auto_inits() {
        // With NEITHER --db NOR --workspace, the launch root is the App-Data-Home and `.nxs/` is
        // auto-initialized there (flow active, "as after nxs init") — the server "just works".
        let home = TempDir::new().unwrap();
        let root = home.path().to_path_buf();
        let resolved = launch_workspace_in(None, None, || Ok(root.clone())).unwrap();
        assert_eq!(resolved.as_deref(), Some(home.path()));
        assert!(
            home.path().join(".nxs").join("replica.toml").is_file(),
            "auto-init materialized a `.nxs/` workspace at the App-Data-Home"
        );
        let ws = Workspace::resolve(None, home.path()).unwrap();
        assert!(
            ws.config.active_modules.iter().any(|m| m == "flow"),
            "the auto-init'd workspace has flow active, like `nxs init`"
        );
    }

    #[test]
    fn launch_workspace_honors_explicit_workspace_without_auto_init() {
        // An explicit --workspace is honored verbatim and is NOT auto-initialized: the
        // "no workspace → error, run nxs init" contract is unchanged for a user-named path, and the
        // App-Data-Home resolver is never even consulted.
        let home = TempDir::new().unwrap();
        let ws_dir = TempDir::new().unwrap();
        let home_root = home.path().to_path_buf();
        let resolved = launch_workspace_in(None, Some(ws_dir.path().to_str().unwrap()), || {
            Ok(home_root.clone())
        })
        .unwrap();
        assert_eq!(resolved.as_deref(), Some(ws_dir.path()));
        assert!(
            !ws_dir.path().join(".nxs").exists(),
            "an explicit --workspace is not auto-init'd"
        );
        assert!(
            !home.path().join(".nxs").exists(),
            "the App-Data-Home is untouched when --workspace is given"
        );
    }

    #[test]
    fn launch_workspace_with_db_override_needs_no_root_and_skips_auto_init() {
        // A --db override resolves the store directly (its parent dir is the workspace), so there is
        // no launch root (None) and the App-Data-Home is never auto-init'd.
        let home = TempDir::new().unwrap();
        let home_root = home.path().to_path_buf();
        let resolved =
            launch_workspace_in(Some("/some/db.sqlite"), None, || Ok(home_root.clone())).unwrap();
        assert_eq!(resolved, None);
        assert!(
            !home.path().join(".nxs").exists(),
            "the App-Data-Home is not auto-init'd on the --db path"
        );
    }

    #[test]
    fn launch_workspace_auto_init_is_idempotent_and_never_clobbers() {
        // A second start over an existing App-Data-Home loads it unchanged (same replica identity),
        // so the server never clobbers the durable board.
        let home = TempDir::new().unwrap();
        let home_root = home.path().to_path_buf();
        let first = launch_workspace_in(None, None, || Ok(home_root.clone())).unwrap();
        let uuid1 = Workspace::resolve(None, first.as_deref().unwrap())
            .unwrap()
            .replica
            .replica_uuid;
        launch_workspace_in(None, None, || Ok(home_root.clone())).unwrap();
        let uuid2 = Workspace::resolve(None, home.path())
            .unwrap()
            .replica
            .replica_uuid;
        assert_eq!(
            uuid1, uuid2,
            "second start loads the same workspace; identity preserved"
        );
    }

    // ---- ctm: seamify maps a command to its TOOL only when one exists ----------------------------

    fn tool_set<'a>(names: &[&'a str]) -> std::collections::HashSet<&'a str> {
        names.iter().copied().collect()
    }

    /// Every `flow_*`/`memory_*` identifier token in `s` — the unit mirror of the e2e cross-check.
    fn flow_memory_tokens(s: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < s.len() {
            let rest = &s[i..];
            let prefix = ["flow_", "memory_"]
                .into_iter()
                .find(|p| rest.starts_with(p));
            if let Some(p) = prefix {
                let after = &rest[p.len()..];
                let len = after
                    .bytes()
                    .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                    .count();
                out.push(format!("{p}{}", &after[..len]));
                i += p.len() + len;
            } else {
                i += rest.chars().next().map(char::len_utf8).unwrap_or(1);
            }
        }
        out
    }

    #[test]
    fn seamify_maps_a_command_to_its_tool_when_one_exists() {
        let tools = tool_set(&["flow_next", "flow_show", "flow_list"]);
        assert_eq!(
            seamify_commands("Find work: `nxf next`.", &tools),
            "Find work: `flow_next`."
        );
        // Args after the verb are preserved; only the `nxf <verb>` head maps.
        assert_eq!(
            seamify_commands("`nxf show <id>`", &tools),
            "`flow_show <id>`"
        );
        // Memory verbs map to the memory_ namespace when that tool is registered.
        let mem = tool_set(&["memory_recall"]);
        assert_eq!(
            seamify_commands("`nxm recall <key>`", &mem),
            "`memory_recall <key>`"
        );
    }

    #[test]
    fn seamify_leaves_a_command_with_no_tool_as_cli_never_a_fake_tool() {
        // The ctm bug: write-verb + multi-word CORE rules have NO tool on the read-only surface, so
        // they must stay honest CLI references, never become non-existent flow_* tool names.
        let tools = tool_set(&["flow_next", "flow_show", "flow_list"]);
        assert_eq!(
            seamify_commands("Claim: `nxf claim <id>`.", &tools),
            "Claim: `nxf claim <id>`."
        );
        // The exact ctm symptom: `nxf note add` must never collapse to a fake `flow_note`.
        let out = seamify_commands("Append (`nxf note add <id> <text>`).", &tools);
        assert!(!out.contains("flow_note"), "no fake flow_note tool: {out}");
        assert_eq!(out, "Append (`nxf note add <id> <text>`).");
    }

    #[test]
    fn seamify_maps_a_multi_word_command_to_its_underscored_tool_when_one_exists() {
        // Once the write tools exist (#wpw/#yie), the multi-word CORE rules (`nxf note add`,
        // `nxf mention add`, `nxf dep add`) must render as their real underscored tools — not stay
        // CLI an MCP-only host can't run. The LONGEST registered candidate wins; trailing args stay.
        let tools = tool_set(&[
            "flow_note_add",
            "flow_dep_add",
            "flow_dep_remove",
            "flow_mention_add",
        ]);
        assert_eq!(
            seamify_commands("Append (`nxf note add <id> <text>`).", &tools),
            "Append (`flow_note_add <id> <text>`)."
        );
        assert_eq!(
            seamify_commands("record: `nxf mention add <this> <cited>`", &tools),
            "record: `flow_mention_add <this> <cited>`"
        );
        assert_eq!(
            seamify_commands("`nxf dep remove <from> <to>`", &tools),
            "`flow_dep_remove <from> <to>`"
        );
        // A single-verb command still maps (the existing path is unchanged).
        let mixed = tool_set(&["flow_next", "flow_note_add"]);
        assert_eq!(
            seamify_commands("`nxf next` then `nxf note add x y`", &mixed),
            "`flow_next` then `flow_note_add x y`"
        );
        // A multi-word command with NO matching tool stays honest CLI (the ctm guarantee holds).
        let none = tool_set(&["flow_next"]);
        assert_eq!(
            seamify_commands("`nxf note add x y`", &none),
            "`nxf note add x y`"
        );
    }

    #[test]
    fn seamify_emits_no_flow_or_memory_token_that_is_not_a_registered_tool() {
        // The invariant the e2e cross-check enforces, pinned at the unit over the real CORE-rule
        // shapes: no produced flow_*/memory_* token escapes the registered tool set.
        let tools = tool_set(&["flow_next", "flow_show", "flow_list"]);
        let rules = [
            "Find what to work on next: `nxf next`.",
            "Claim work before starting it: `nxf claim <id>`.",
            "Close with a reason: `nxf close <id> --reason <text>`.",
            "stabilize (`nxf note add <id> <text>`)",
            "record the reference: `nxf mention add <this-item> <cited-id>`",
            "create item as JSON (`nxf create --json -`)",
        ];
        for rule in rules {
            let out = seamify_commands(rule, &tools);
            for tok in flow_memory_tokens(&out) {
                assert!(
                    tools.contains(tok.as_str()),
                    "seamify emitted a non-tool token {tok:?} from rule {rule:?}"
                );
            }
        }
    }
}
