//! The relay's HTTP surface (Axum). Three paths per stream: `ops` (append a batch, or read a page
//! since a cursor), `register` (claim a replica prefix) and `machines` (a machine's service says it
//! is here, or anyone asks which machines have — 6j6v.f0b5). The handlers are a thin shell over
//! the stores — they fold nothing, derive nothing, and reject no well-formed op (spec §2/§3).

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

use crate::presence;
use crate::registry::PrefixRegistry;
use crate::store::OpStore;
use nxs_sync::presence::{check_hello, MAX_LISTED_MACHINES, RETENTION_SECS};
use nxs_sync::protocol::{
    Cursor, MachineHello, MachinesResponse, PullResponse, PushRequest, PushResponse,
    RegisterOutcome, RegisterRequest, StreamId,
};

/// The relay's op log, shared across handlers. A trait object so any backend (§10) drops
/// in without touching the HTTP layer.
pub type SharedStore = Arc<dyn OpStore + Send + Sync>;

/// The relay's prefix coordinator (bab/§6), shared across handlers. It may also carry the presence
/// board (6j6v.f0b5, [`PrefixRegistry::presence`]); one that does not — a registry an embedder
/// wrote before presence existed — still fits here unchanged.
pub type SharedRegistry = Arc<dyn PrefixRegistry + Send + Sync>;

/// The relay's wall clock, in Unix seconds. Injected rather than read inside the handler so the
/// age of a sighting — the one thing presence is judged by — is provable without waiting.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The real clock: `SystemTime::now()` in Unix seconds. A clock set before 1970 reads as 0 rather
/// than panicking the relay.
pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    })
}

/// What every handler shares: the durable op log, the registry (prefix claims + presence) and the
/// clock presence is stamped with. None of it folds or derives anything.
#[derive(Clone)]
pub struct AppState {
    pub ops: SharedStore,
    pub registry: SharedRegistry,
    pub clock: Clock,
}

/// Default page size when a client omits `limit` — bounds the response without forcing
/// the client to know a number (it follows `next` until exhausted).
const DEFAULT_LIMIT: usize = 500;

/// Explicit, intentional ceiling on a single push request body, in bytes. Makes the
/// request size a deliberate contract rather than relying on axum's implicit default; an
/// over-limit body is rejected by the [`DefaultBodyLimit`] layer with 413 before a handler
/// runs (k64).
const MAX_PUSH_BYTES: usize = 8 * 1024 * 1024;

/// The body ceiling of the presence route. A valid announcement is under 300 bytes; without its own
/// limit the route inherited [`MAX_PUSH_BYTES`] and an anonymous caller could make the relay buffer
/// and parse 8 MiB before the bounds check refused it (review of PR #485, Integrity #6).
const MAX_PRESENCE_BYTES: usize = 4 * 1024;

/// Explicit, intentional ceiling on the op COUNT of a single push request — the second
/// axis of the deliberate request bound (k64), orthogonal to [`MAX_PUSH_BYTES`]: many tiny
/// ops can be well under the byte ceiling yet still amount to an abusive op count. It sits
/// ABOVE the client's push page size (`SYNC_PAGE_LIMIT`, 500) so a client that paginates
/// push into bounded batches never trips the OP-COUNT cap; a giant batch is rejected with
/// 413 instead of forcing unbounded per-request work. (Note: client pagination bounds the
/// op COUNT only — a page of unusually large-bodied ops could still hit the byte ceiling;
/// byte-aware sub-pagination is tracked as nexus-flow-7qe.)
pub const MAX_PUSH_OPS: usize = 1000;

/// Build the relay router over the op log + registry, on the system clock. Loopback, no auth (a
/// later slice). The body limit guards the push route; register/read/presence bodies are tiny.
pub fn app(ops: SharedStore, registry: SharedRegistry) -> Router {
    app_with_clock(ops, registry, system_clock())
}

/// [`app`] with the clock presence is stamped with named explicitly — what a test turns by hand.
pub fn app_with_clock(ops: SharedStore, registry: SharedRegistry, clock: Clock) -> Router {
    Router::new()
        .route("/streams/:id/ops", post(append).get(read_since))
        .route("/streams/:id/register", post(register))
        .route(
            "/streams/:id/machines",
            post(announce)
                .get(machines)
                .layer(DefaultBodyLimit::max(MAX_PRESENCE_BYTES)),
        )
        .layer(DefaultBodyLimit::max(MAX_PUSH_BYTES))
        .with_state(AppState {
            ops,
            registry,
            clock,
        })
}

async fn append(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, StatusCode> {
    // Reject an over-cap batch BEFORE touching the store, so it costs nothing and nothing is
    // partially appended (rationale on the constant). The byte axis is the DefaultBodyLimit.
    if req.ops.len() > MAX_PUSH_OPS {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let stream = StreamId(id);
    for op in &req.ops {
        state
            .ops
            .append(&stream, op)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(PushResponse {
        appended: req.ops.len(),
    }))
}

/// Register a replica's prefix for the stream (bab/§6). The relay coordinates distinctness
/// here — the one place it is more than a dumb op-carrier — and stays opaque to op shape.
async fn register(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<RegisterOutcome>, StatusCode> {
    let stream = StreamId(id);
    let outcome = state
        .registry
        .register(&stream, &req.prefix, &req.replica_uuid)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(outcome))
}

/// A machine's background service says it is here (6j6v.f0b5). Stamped with the RELAY's clock,
/// never the machine's, so every sighting in a stream is on one clock, and the stream's machines
/// not heard from within the retention window are forgotten in the same call. A hello that would
/// make the relay store something unbounded is refused with 400 and the reason — the relay
/// authenticates nobody, so that bound is the only one there is. A registry without a presence
/// board answers 404, like a relay that predates the route. Nothing reaches the op log.
async fn announce(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(hello): Json<MachineHello>,
) -> Result<StatusCode, (StatusCode, String)> {
    let Some(board) = state.registry.presence() else {
        return Err((StatusCode::NOT_FOUND, String::new()));
    };
    check_hello(&hello).map_err(|reason| (StatusCode::BAD_REQUEST, reason))?;
    let now = (state.clock)();
    board
        .announce(&StreamId(id.clone()), &hello, now, now - RETENTION_SECS)
        .map_err(|e| {
            // The client gets a bare 500 like every other store failure here; the operator gets the
            // reason, which is otherwise nowhere (review of PR #485, Integrity #7).
            eprintln!("nxf-relay: recording machine presence for stream {id:?}: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, String::new())
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// Which machines have announced themselves for the stream within the retention window, with their
/// age on the relay's clock, most recent first — and whether there were more than one answer
/// carries. Unjudged: whether one is online is `nxs_sync::presence::judge`'s to say, in the reader.
async fn machines(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<MachinesResponse>, StatusCode> {
    let Some(board) = state.registry.presence() else {
        return Err(StatusCode::NOT_FOUND);
    };
    let now = (state.clock)();
    let page = board
        .machines(
            &StreamId(id.clone()),
            now - RETENTION_SECS,
            MAX_LISTED_MACHINES,
        )
        .map_err(|e| {
            eprintln!("nxf-relay: reading machine presence for stream {id:?}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(Json(MachinesResponse {
        machines: page
            .records
            .iter()
            .map(|r| presence::sighting(r, now))
            .collect(),
        truncated: page.truncated,
    }))
}

#[derive(Debug, Deserialize)]
struct ReadParams {
    since: Option<i64>,
    limit: Option<usize>,
}

async fn read_since(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<ReadParams>,
) -> Result<Json<PullResponse>, StatusCode> {
    let stream = StreamId(id);
    let cursor = Cursor(params.since.unwrap_or(0));
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    let (ops, next) = state
        .ops
        .read_since(&stream, cursor, limit)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(PullResponse { ops, next }))
}
