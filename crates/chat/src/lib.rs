//! nexus-chat — the message-domain kind-product over the shared nxs substrate (spec
//! docs/specs/nexus-chat-M1.md). Sibling of nexus-flow/nexus-memory: depends only on the
//! foundation. T1 is the substrate — reducer + views + store; T2 is the `nxc` messaging CLI
//! (send/reply/status/threads/search — `inbox`, `read`, `channels` and `agents` stood here until
//! 6j6v.dvyq §3, 6j6v.1gm9 and 6j6v.4d2z removed them); T3 (this) is the setup/onboarding contract:
//! the `init`/`agent-manifest`/`prime` verbs + the [`inventory`]-based `ModuleInit`
//! self-registration that makes `nxc` appear in the `nxs init` chooser and the `nxs prime` fan-out.

// The CLI/onboarding surface (a6ah): behind the default-on `cli` feature so an embedding consumer
// that links only the `engine` handle can drop the whole clap/nxs-ui/nxs-init/inventory/sha2
// closure with `default-features = false`. Everything below `cli`/`onboarding` — the substrate the
// engine reads/writes — is always compiled.
#[cfg(feature = "cli")]
pub mod cli;
/// chat's half of the ONE embedded-guide mechanism (6j6v.9e3r): the `include_dir!` embed + the
/// ordered topic list `nxc guide` serves and `nxs guide` fans out over. CLI-surface only — an
/// embedding consumer has no `nxc` to explain.
#[cfg(feature = "cli")]
pub mod guide;
/// **A session that stopped because the model went away, and the way back** (nxf 6j6v.npy3).
pub mod interruption;
#[cfg(feature = "cli")]
pub mod onboarding;

/// **Whom a caller may answer right now** (nxf 6j6v.dq59) — the derivation an `nxc reply` without a
/// thread id resolves through, and the lines the coordinator puts in front of a session it wakes.
pub mod addressees;
/// **Where the working copy stood when a turn changed hands** (nxf 6j6v.2af2) — the commit anchor
/// every `send`/`reply` writes onto its message, plus the fingerprint that makes it comparable when
/// an interrupted operation is taken up again. `pub` because the fact itself is: it rides
/// [`model::Refs`] and comes back on [`facade::StatusThread`], both of which an embedding host
/// reads.
pub mod anchor;
/// **How a requester learns the answer** (nxf 6j6v.7qfm) — the `await` block `send` and `reply`
/// hand back, and the human lines that say the same thing. Not CLI-gated: the block is a receipt
/// field an embedding app reads exactly as `nxc --json` does.
pub mod awaiting;
pub mod channel;
/// **The persona coordinator's pending queue** (nxf 6j6v.gn8b): the durable hold that turns "your
/// caller is busy, so nobody was woken" into "your caller is resumed with this, and with everything
/// else that arrived meanwhile, once it settles".
pub mod collecting;
pub mod consolidation_claim;
pub mod declaration_freeze;
/// **Is this declaration WRITTEN so that it works** (nxf 6j6v.9w08) — the decidable sliver of
/// `nxc guide writing-declarations`, beside `channel::validate_channels`'s "does it RESOLVE".
pub mod declaration_quality;
pub mod declaration_version;
pub mod definitions;
pub mod engine;
pub mod error;
pub mod facade;
// Not `pub`: every item in it is engine-internal, it has no caller outside this crate, and
// `nexus-chat` is a consumed SemVer surface — new public API that nothing consumes is a contract to
// keep for no one. The value it carries reaches a reader through `ChatStore::thread_quorum`'s
// `deadline`/`stale`, which is public and always was.
pub mod machine;
pub(crate) mod member_deadline;
pub mod message_reducer;
pub mod model;
/// **What a conversation is CALLED** (nxf 6j6v.e76c) — the seam a thread's short display name is
/// derived through, and the rules that keep it a display name. `pub` for [`timer`]'s reason: an
/// embedding host chooses its backend as a value, so it needs the type to name.
pub mod naming;
pub mod orchestration;
pub mod park;
pub mod persona;
pub mod precondition;
pub mod role;
pub mod schema;
pub mod session_map;
pub mod store;
pub mod stream;
pub mod surface;
pub mod timer;
pub mod transcript;
pub mod watch;
pub mod worker;
pub mod working_tree;
pub mod workspace;

#[cfg(feature = "cli")]
pub use cli::{run, run_from, run_from_with};
