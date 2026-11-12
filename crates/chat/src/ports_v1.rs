//! **The new engine's ports, implemented on the old side** (6j6v.zjyv §5; ticket 6j6v.b8tj): the chat model,
//! the declarations and the sidecar bundle keep their one home in this crate, and the host reaches them only
//! through these three traits, which is how the new crates stay free of old imports.
//!
//! [`V1Ports`] is one value behind all three, because all three read the same workspace: an
//! `Arc<V1Ports>` coerces to each `Arc<dyn …Port>` the host's `Ports` holds.
//!
//! **Declarations are composed from the REGISTERED root** ([`V1Ports::for_root`]): a session of a run works
//! in `<root>-v2`, whose `.nxs` link resolves to the same workspace, but the standing text of its prompt
//! comes from the root the service registered — the clone's own copies matter only to what the session reads
//! as files (§6.6, "What the directory proves").
//!
//! Every line here goes when the old engine does.
// v2-bridge: deleted with the old engine (6j6v.2vtw)

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use nexus_run_contracts::MessageId;
use nexus_run_host::ports::{
    self, DeclarationPort, Message, MessagePort, PersonaDecl, PortError, PortResult, PromptBundle,
    PromptFor, SidecarPort, TurnPart,
};

use crate::definitions::Definitions;
use crate::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use crate::store::ChatStore;
use crate::workspace::{ChatWorkspaceExt, Workspace};

fn refused(error: impl std::fmt::Display) -> PortError {
    PortError {
        what: error.to_string(),
    }
}

/// The host's ports over this workspace's chat store and declarations.
pub struct V1Ports {
    db_path: String,
    origin: String,
    defs: Definitions,
    /// Opened on first use: a commission reads no message, and a store it never touches is not opened.
    store: OnceLock<Mutex<ChatStore>>,
}

impl std::fmt::Debug for V1Ports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("V1Ports")
            .field("db_path", &self.db_path)
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

impl V1Ports {
    /// Over declarations a caller already resolved — the CLI's and an `Engine`'s, for a bridge verb run in
    /// the caller's own process.
    pub fn with_definitions(db_path: &str, origin: &str, defs: Definitions) -> V1Ports {
        V1Ports {
            db_path: db_path.to_owned(),
            origin: origin.to_owned(),
            defs,
            store: OnceLock::new(),
        }
    }

    fn store(&self) -> PortResult<MutexGuard<'_, ChatStore>> {
        if self.store.get().is_none() {
            let store = Workspace::resolve(Some(&self.db_path), Path::new("."))
                .and_then(|ws| ws.open_chat_store())
                .map_err(refused)?;
            // A racing first use opened its own: either one is the same file.
            let _ = self.store.set(Mutex::new(store));
        }
        let cell = self.store.get().expect("set above");
        cell.lock()
            .map_err(|_| refused("the chat store's lock is poisoned"))
    }

    fn now() -> String {
        time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default()
    }
}

impl MessagePort for V1Ports {
    fn post(
        &self,
        thread: &str,
        sender: &str,
        body: &str,
        refs: &ports::Refs,
        key: &str,
    ) -> PortResult<MessageId> {
        let mut store = self.store()?;
        let channel_id = store
            .thread_channel(thread)
            .ok_or_else(|| refused(format!("thread {thread} is not open")))?;
        let envelope = MessageEnvelope {
            origin: self.origin.clone(),
            channel_id,
            sender: sender.to_owned(),
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some(thread.to_owned()),
            refs: Refs {
                nxf_ids: refs.nxf_ids.clone(),
                run_key: Some(key.to_owned()),
                ..Refs::default()
            },
            body: body.to_owned(),
        };
        store.try_post_message(&envelope).map_err(refused)
    }

    fn find(&self, thread: &str, key: &str) -> PortResult<Option<MessageId>> {
        let rows = self.store()?.messages_in_thread(thread).map_err(refused)?;
        Ok(rows.into_iter().find_map(|row| {
            let refs: Refs = serde_json::from_str(row.refs.as_deref()?).ok()?;
            (refs.run_key.as_deref() == Some(key)).then_some(row.message_id)
        }))
    }

    fn read(&self, message: &str) -> PortResult<Option<Message>> {
        let store = self.store()?;
        let Some((_, Some(thread))) = store.message_channel(message) else {
            return Ok(None);
        };
        let rows = store.messages_in_thread(&thread).map_err(refused)?;
        Ok(rows.into_iter().find_map(|row| {
            (row.message_id == message).then(|| {
                let refs: Refs = row
                    .refs
                    .as_deref()
                    .and_then(|refs| serde_json::from_str(refs).ok())
                    .unwrap_or_default();
                Message {
                    sender: row.sender,
                    body: row.body,
                    refs: ports::Refs {
                        nxf_ids: refs.nxf_ids,
                    },
                }
            })
        }))
    }

    fn open_thread(
        &self,
        thread_id: &str,
        parent: Option<&str>,
        channel: &str,
        opener: &str,
    ) -> PortResult<()> {
        let root = ThreadRoot {
            origin: self.origin.clone(),
            channel_id: channel.to_owned(),
            opener: opener.to_owned(),
            created: V1Ports::now(),
            parent: parent.map(str::to_owned),
        };
        self.store()?
            .try_open_thread(thread_id, &root, opener)
            .map_err(refused)
    }
}

impl DeclarationPort for V1Ports {
    /// The channel as the old side declares it, in the engine's own plain shape: the two read the same
    /// declaration language, so the chat declaration's wire form is the engine's input.
    fn channel(&self, name: &str) -> PortResult<ports::ChannelDecl> {
        let decl = self
            .defs
            .channel(name)
            .ok_or_else(|| refused(format!("no channel `{name}` is declared")))?;
        let wire = serde_json::to_value(decl).map_err(refused)?;
        serde_json::from_value(wire).map_err(refused)
    }

    fn persona(&self, _: &str) -> PortResult<PersonaDecl> {
        Err(refused("not built yet"))
    }
    fn prompt(&self, _: &str, _: PromptFor) -> PortResult<PromptBundle> {
        Err(refused("not built yet"))
    }
    fn turn(&self, _: &str, _: &[TurnPart]) -> PortResult<String> {
        Err(refused("not built yet"))
    }
}

impl SidecarPort for V1Ports {
    fn bundle_path(&self) -> PortResult<PathBuf> {
        Err(refused("not built yet"))
    }
}
