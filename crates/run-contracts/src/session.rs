//! Contract (c): the session start spec in, session facts out. The host builds the spec, and the adapter
//! turns it into a running session. Nothing in the spec is a decision and nothing in a fact is one.
//!
//! The start spec carries the commission body, so it is NEVER persisted into the run log (M8). Facts
//! reach the store through `enqueue` from the session ledger, never as a direct append.

use serde::{Deserialize, Serialize};

use crate::events::UnknownBody;
use crate::ids::{MessageId, SessionKey, Timestamp};

/// The only variables a session's environment may carry.
pub const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "NXC_DB",
    "NXC_ORIGIN",
    "NXC_HOP",
    "NXC_SIDECAR",
    "NXC_WORKER",
    "NXC_SESSION",
    "NXC_ACTOR",
];

/// What a session is started from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartSpec {
    pub v: u32,
    pub session_key: SessionKey,
    /// The directory the session runs in; the adapter sets it as the child's working directory.
    pub cwd: String,
    pub env: Vec<EnvVar>,
    pub prompt: Prompt,
    pub turn: Turn,
    /// The backend session id to resume, per run.
    pub resume: Option<String>,
    pub model: Option<String>,
    pub tools: Vec<String>,
    /// Where the session's facts, offset and child record live.
    pub ledger_dir: String,
}

/// One variable of the session's environment. A list and not a map, so the wire order is the writer's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// The standing context of a session, as the declaration port composed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prompt {
    pub system_prompt: String,
    pub use_claude_code_preset: bool,
    pub declaration_hash: String,
}

/// The turn a session is handed, and the thread it answers in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub thread: MessageId,
    pub text: String,
}

/// What a session reported. Every variant is a fact, not a conclusion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionFact {
    /// `pid` is absent where the host has no process.
    Started {
        backend_session_id: String,
        #[serde(default)]
        pid: Option<u32>,
    },
    Heartbeat {
        beat: HeartbeatKind,
    },
    Usage {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        turns: u32,
    },
    /// `program` is the fail-closed basename of the first shell word, `<unparsed>`, or the tool's name.
    /// No arguments and no hash, so a secret in an argument never reaches the log.
    PermissionRefused {
        rule: String,
        program: String,
    },
    Ended {
        cause: EndCause,
    },
    /// A fact kind this version does not know.
    #[serde(untagged)]
    Unknown(UnknownBody),
}

/// Signs of life. A `model_output` re-arms `quiet`; a busy period opens and closes around overlapping
/// calls; `subprocess_moved` says a sample of an open call's process tree changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatKind {
    ModelOutput,
    CallsOpen,
    CallsClosed,
    SubprocessMoved,
}

/// Why a session ended, as the adapter mapped the raw SDK outcome by its fixture table. The raw message
/// never reaches the log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum EndCause {
    TurnComplete,
    Exit { status: Option<i32> },
    UsageLimit { resets_at: Option<Timestamp> },
    ProviderUnavailable,
    PermissionRefused { rule: String, program: String },
    StoppedByHost,
    Lost,
}

/// What a backend can do. The engine uses only what is declared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub backend: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Resume,
    Nudge,
    Stop,
    Usage,
    PermissionRules,
    SubprocessSample,
}

/// What a backend says about a session. An end is confirmed only with the leader dead and reaped AND
/// the group empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Liveness {
    Dead,
    Alive {
        open_call: Option<Timestamp>,
        moved_at: Option<Timestamp>,
    },
}

/// The answer of `start`, which is idempotent by session key and answers from its ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartOutcome {
    /// A new process was spawned.
    Spawned,
    /// The recorded process is alive under its identity, and its facts are read on from the offset.
    Reattached,
    /// Spawned earlier with no child record yet; `start_deadline` decides.
    Waiting,
    /// The recorded process is gone, or is another process now.
    Lost,
}

/// One fact and the byte offset it starts at in the session's facts file, because the inbox key of a
/// fact is `fact/<session_key>/<byte offset>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrainedFact {
    pub offset: u64,
    pub fact: SessionFact,
}

/// A backend could not do what was asked. The text is for the host's own status and never enters the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendError {
    pub what: String,
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session backend: {}", self.what)
    }
}

impl std::error::Error for BackendError {}

/// The Rust face of contract (c). A backend holds no in-memory queue a crash could lose, and it never
/// reads the run store or the chat model.
///
/// The identity guard every signal goes through (pid and start time re-verified before each kill) is
/// not part of this trait. Its home in this slice is `run-session-claude`'s `procs` module, and the
/// host uses it through its existing dependency; the first non-Claude backend moves it to a crate of
/// its own.
pub trait SessionBackend: Send + Sync {
    fn capabilities(&self) -> Capabilities;
    fn start(&self, spec: &StartSpec) -> Result<StartOutcome, BackendError>;
    fn nudge(&self, key: &str, text: &str) -> Result<(), BackendError>;
    /// SIGTERM, or SIGKILL of the group with `force`, which only a withdrawal sets.
    fn stop(&self, key: &str, force: bool) -> Result<(), BackendError>;
    fn alive(&self, key: &str) -> Result<Liveness, BackendError>;
    /// The complete facts from `from_offset` on, and the offset to read from next.
    fn drain_facts(
        &self,
        key: &str,
        from_offset: u64,
    ) -> Result<(Vec<DrainedFact>, u64), BackendError>;
}
