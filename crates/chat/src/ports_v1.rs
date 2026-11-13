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
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use nexus_run_contracts::MessageId;
use nexus_run_host::ports::{
    self, DeclarationPort, Message, MessagePort, PersonaDecl, PortError, PortResult, PromptBundle,
    PromptFor, SidecarPort, TurnKind, TurnPart,
};

use crate::channel::CollectedReply;
use crate::definitions::Definitions;
use crate::facade::{ModulePrimes, PrimeFor};
use crate::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use crate::role::{ClaudeMd, SessionPolicy};
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
    project_claude_md: Option<String>,
    module_primes: Option<Arc<dyn ModulePrimes>>,
    sidecar: Option<PathBuf>,
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
            project_claude_md: None,
            module_primes: None,
            sidecar: None,
            store: OnceLock::new(),
        }
    }

    /// Composed from the REGISTERED root: its `.nxs-personas` and the user-level folder, and its own
    /// `CLAUDE.md` — what the service's host passes, whichever directory a session runs in.
    pub fn for_root(root: &Path, db_path: &str, origin: &str) -> crate::error::Result<V1Ports> {
        let mut ports = V1Ports::with_definitions(db_path, origin, Definitions::resolve(root)?);
        ports.project_claude_md = std::fs::read_to_string(root.join("CLAUDE.md")).ok();
        Ok(ports)
    }

    /// The sibling modules' blocks (`nxf prime`, `nxm prime`), which only the umbrella can compose.
    pub fn with_module_primes(mut self, primes: Arc<dyn ModulePrimes>) -> V1Ports {
        self.module_primes = Some(primes);
        self
    }

    /// The sidecar bundle a session starts from, in place of the one this binary carries.
    pub fn with_sidecar(mut self, bundle: PathBuf) -> V1Ports {
        self.sidecar = Some(bundle);
        self
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

/// A message's refs: none stored is none; stored and not decoding is an error naming the message.
fn decode_refs(message: &str, refs: Option<&str>) -> PortResult<Refs> {
    match refs {
        None => Ok(Refs::default()),
        Some(refs) => serde_json::from_str(refs).map_err(|error| {
            refused(format!(
                "the refs of message {message} do not decode: {error}"
            ))
        }),
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
        // A row whose refs do not decode is an error, never "not posted": the host's find-or-post would post
        // a second copy on that answer.
        for row in rows {
            if decode_refs(&row.message_id, row.refs.as_deref())?
                .run_key
                .as_deref()
                == Some(key)
            {
                return Ok(Some(row.message_id));
            }
        }
        Ok(None)
    }

    fn read(&self, message: &str) -> PortResult<Option<Message>> {
        let store = self.store()?;
        let Some((_, Some(thread))) = store.message_channel(message) else {
            return Ok(None);
        };
        let rows = store.messages_in_thread(&thread).map_err(refused)?;
        let Some(row) = rows.into_iter().find(|row| row.message_id == message) else {
            return Ok(None);
        };
        let refs = decode_refs(&row.message_id, row.refs.as_deref())?;
        Ok(Some(Message {
            sender: row.sender,
            body: row.body,
            refs: ports::Refs {
                nxf_ids: refs.nxf_ids,
            },
        }))
    }

    fn open_thread(
        &self,
        thread_id: &str,
        parent: Option<&str>,
        channel: &str,
        opener: &str,
    ) -> PortResult<()> {
        let mut store = self.store()?;
        // A declared channel is the substrate channel `decl:<name>`, materialised as the old engine does it
        // (6j6v.y6v7): the read filters resolve visibility and membership from that id, and a thread in a
        // channel the store does not know reads as `no such channel` to every reader.
        let channel_id = match self.defs.channel(channel) {
            Some(decl) => materialise_declared_channel(&mut store, decl, &self.origin, opener)?,
            None => channel.to_owned(),
        };
        let root = ThreadRoot {
            origin: self.origin.clone(),
            channel_id,
            opener: opener.to_owned(),
            created: V1Ports::now(),
            parent: parent.map(str::to_owned),
        };
        store
            .try_open_thread(thread_id, &root, opener)
            .map_err(refused)
    }
}

/// The substrate channel behind a declared one, `decl:<name>`, made to exist with its declared kind and its
/// cast as members, plus `opener` — idempotent, and through the store's `try_` writes, so a locked store is
/// an error the host retries and never a panic. The old engine's `ensure_declared_channel`, for the host.
fn materialise_declared_channel(
    store: &mut ChatStore,
    decl: &crate::channel::ChannelDecl,
    origin: &str,
    opener: &str,
) -> PortResult<String> {
    let channel_id = format!("decl:{}", decl.name);
    // Its fields once: a thread the host opens again after a crash writes nothing it already wrote.
    if !store.channel_exists(&channel_id).map_err(refused)? {
        for (field, value) in [
            ("kind", decl.kind.as_str()),
            ("origin", origin),
            ("name", decl.name.as_str()),
        ] {
            store
                .try_set_channel_field(&channel_id, field, value, opener)
                .map_err(refused)?;
        }
    }
    for handle in crate::channel::cast(decl)
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(opener))
    {
        if !store.is_member(&channel_id, handle).map_err(refused)? {
            store
                .try_add_member(&channel_id, handle, opener)
                .map_err(refused)?;
        }
    }
    Ok(channel_id)
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

    fn persona(&self, handle: &str) -> PortResult<PersonaDecl> {
        let decl = self.defs.role(handle).map_err(refused)?;
        Ok(PersonaDecl {
            handle: decl.handle.clone(),
            model: decl
                .effective_model()
                .and_then(|model| serde_json::to_value(model).ok())
                .and_then(|model| model.as_str().map(str::to_owned)),
            permissions: decl.permissions.clone(),
            tools: decl.tools.clone(),
            resume_session: matches!(decl.session, SessionPolicy::Continue),
        })
    }

    /// The old prime, composed as a spawned persona's is, for a session of a run on the new engine: no reply
    /// obligation (the host writes its own into the turn) and no sentence that commissions (§5, §10).
    fn prompt(&self, handle: &str, purpose: PromptFor) -> PortResult<PromptBundle> {
        let PromptFor::V2 = purpose;
        let decl = self.defs.role(handle).map_err(refused)?;
        let prime_block = if decl.prime.is_primed() {
            let services = decl.prime.services();
            let siblings = match &self.module_primes {
                Some(primes) => primes
                    .module_primes(services, &self.db_path)
                    .map_err(refused)?,
                None => Vec::new(),
            };
            crate::facade::compose_persona_prime_as(
                &*self.store()?,
                &format!("{}/{}", self.origin, decl.handle),
                &decl.handle,
                services,
                &siblings,
                &V1Ports::now(),
                self.defs.source(),
                PrimeFor::V2,
            )
            .map_err(refused)?
        } else {
            String::new()
        };
        let claude_md = match decl.claude_md {
            ClaudeMd::Inherit => self.project_claude_md.as_deref(),
            _ => None,
        };
        let composed =
            crate::role::compose_system_prompt(decl, claude_md, None, &prime_block, None);
        Ok(PromptBundle {
            system_prompt: composed.system_prompt,
            use_claude_code_preset: composed.use_claude_code_preset,
            granted_tools: vec![crate::role::REPLY_OBLIGATION_TOOL.to_owned()],
            declaration_hash: crate::declaration_version::role_declaration_hash(decl),
        })
    }

    /// The parts that carry a message, read by id. Another agent's reply is framed exactly as a
    /// consolidator's input is — untrusted data under a boundary no answer contains — and the
    /// commissioner's own text (the commission, an answer) is handed as written.
    fn turn(&self, _handle: &str, parts: &[TurnPart]) -> PortResult<String> {
        let mut sections = Vec::new();
        let mut replies: Vec<CollectedReply> = Vec::new();
        for part in parts {
            // A part with no message is the host's own text (task, obligation, reminder): `start.rs` writes
            // those itself and hands this port only runs of message-bearing parts, so none is lost here.
            let Some(id) = part.message.as_deref() else {
                continue;
            };
            let message = self
                .read(id)?
                .ok_or_else(|| refused(format!("message {id} is not in the chat store")))?;
            match &part.kind {
                // No step mark: the frame renders one only with its pass, which a turn part does not
                // carry, and a mark the engine cannot vouch for is worse than none.
                TurnKind::Reply { .. } => replies.push(CollectedReply {
                    sender: message.sender,
                    body: message.body,
                    substituted: false,
                    step: None,
                    pass: None,
                    needs_rework: false,
                    thread: None,
                }),
                _ => {
                    flush_replies(&mut replies, &mut sections);
                    sections.push(message.body);
                }
            }
        }
        flush_replies(&mut replies, &mut sections);
        Ok(sections.join("\n\n"))
    }
}

fn flush_replies(replies: &mut Vec<CollectedReply>, sections: &mut Vec<String>) {
    if replies.is_empty() {
        return;
    }
    let boundary = crate::channel::block_boundary(replies);
    sections.push(format!(
        "{}\n\n{}\n\n{}",
        crate::channel::framing(&boundary),
        crate::channel::attribution_notice(&boundary),
        crate::channel::untrusted_block(replies, &boundary),
    ));
    replies.clear();
}

impl SidecarPort for V1Ports {
    fn bundle_path(&self) -> PortResult<PathBuf> {
        if let Some(bundle) = &self.sidecar {
            return Ok(bundle.clone());
        }
        crate::worker::installed_sidecar()
            .map_err(refused)?
            .ok_or_else(|| refused("this nxs carries no sidecar bundle"))
    }
}
