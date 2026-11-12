//! The ports (§5): traits defined here and implemented on the old side, in `crates/chat`, and injected by
//! `nxs`, which is how the new crates stay free of old imports while the chat model, the declarations and
//! the sidecar bundle are still owned by old code.
//!
//! Every port read returns `Result`, and the host never turns an error into an absence: a failed `find`
//! posts nothing and retries next tick, a failed `read` leaves the effect owed.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nexus_run_contracts::{Handle, MessageId, Timestamp};
use nexus_run_engine::{ChannelDecl, TurnPart};
use serde::{Deserialize, Serialize};

pub type PortResult<T> = Result<T, PortError>;

/// A port could not answer. The text may quote what the old side holds, so the host keeps it out of its
/// log lines and off the log (PV1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortError {
    pub what: String,
}

impl std::fmt::Display for PortError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "port: {}", self.what)
    }
}

impl std::error::Error for PortError {}

/// What a message points at, beside the key the port stores in `refs.run_key`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    #[serde(default)]
    pub nxf_ids: Vec<String>,
}

/// A message the state names by id: the host composes from it, because run state carries ids only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub sender: Handle,
    pub body: String,
    pub refs: Refs,
}

/// The chat store, through its `try_` forms only.
pub trait MessagePort: Send + Sync {
    /// Posts into the thread's own channel from `sender`. `refs` carry `run_key = key` beside `refs`.
    fn post(
        &self,
        thread: &str,
        sender: &str,
        body: &str,
        refs: &Refs,
        key: &str,
    ) -> PortResult<MessageId>;
    /// The message of this thread whose refs carry `key`.
    fn find(&self, thread: &str, key: &str) -> PortResult<Option<MessageId>>;
    fn read(&self, message: &str) -> PortResult<Option<Message>>;
    /// Opens a thread under an id the HOST minted. A second open of the same id is the same op.
    fn open_thread(
        &self,
        thread_id: &str,
        parent: Option<&str>,
        channel: &str,
        opener: &str,
    ) -> PortResult<()>;
}

/// A persona as declared, in this crate's own plain shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PersonaDecl {
    pub handle: Handle,
    pub model: Option<String>,
    pub permissions: Option<String>,
    /// The `tools:` the persona declares. `None` is no declaration, which leaves the base toolset whole; `Some([])`
    /// declares nothing, which narrows it to what the grant needs (the old sidecar's `resolveToolsOption`).
    pub tools: Option<Vec<String>>,
    pub resume_session: bool,
}

/// Who a prompt is composed for. A v2 prime leaves out `reply_obligation` and the commissioning sentences
/// of the old prime block: the host writes its own obligation (§5, §6.4 item 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptFor {
    V2,
}

/// The standing context of a session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptBundle {
    pub system_prompt: String,
    pub use_claude_code_preset: bool,
    pub granted_tools: Vec<String>,
    pub declaration_hash: String,
}

/// The declarations, the prompt and the framing of a turn: all of it keeps its one home on the v1 side.
pub trait DeclarationPort: Send + Sync {
    fn channel(&self, name: &str) -> PortResult<ChannelDecl>;
    fn persona(&self, handle: &str) -> PortResult<PersonaDecl>;
    fn prompt(&self, handle: &str, purpose: PromptFor) -> PortResult<PromptBundle>;
    /// Composes the parts that carry a message, framing another agent's reply as untrusted text. The host
    /// hands ids; the port reads and frames.
    fn turn(&self, handle: &str, parts: &[TurnPart]) -> PortResult<String>;
}

/// Where the sidecar bundle is unpacked.
pub trait SidecarPort: Send + Sync {
    fn bundle_path(&self) -> PortResult<PathBuf>;
}

/// The host's only clock. Tests inject a simulated one.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The machine's clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis());
        Timestamp::from_unix_millis(i64::try_from(millis).unwrap_or(i64::MAX))
    }
}
