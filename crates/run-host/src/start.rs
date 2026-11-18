//! `StartSession` (§6.4 item 3): the step thread under an id the ledger keeps, the turn composed from ids and
//! posted as the step's commission, the routes a reply needs, all before the spawn. The backend's own ledger
//! answers for a session a crash forgot: `start` on a key it knows does not spawn.

use nexus_run_contracts::{
    EndCause, EnvVar, Epoch, EventBody, MessageId, Origin, Prompt, RunId, RunStore, SessionFact,
    Slot, StartOutcome, StartPort, StartRefusal, StartSpec, StoreError, Timestamp, Turn,
    CONTRACT_V,
};
use nexus_run_engine::{RunState, StepDef, StepKind, TurnKind, TurnPart, SYNTH};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::boundary::{crash_at, wrote, Boundary};
use crate::compose;
use crate::effects::{DONE, PENDING};
use crate::host::Host;
use crate::ports::{PromptFor, Refs};

/// The value of `thread/<id>`: which run this thread belongs to, and which session it waits on. A root
/// thread has none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRoute {
    pub run: RunId,
    pub session_key: Option<String>,
}

pub(crate) fn mint_thread_id() -> MessageId {
    format!("m-{}", Ulid::new())
}

impl Host {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_session(
        &mut self,
        state: &RunState,
        epoch: Epoch,
        session_key: &str,
        turn: &[TurnPart],
        resume: bool,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let run = &state.run;
        if self.cache(run).started.contains(session_key) {
            return Ok(());
        }
        let Some(index) = state.find(session_key) else {
            return Ok(());
        };
        let slot = state.slots[index].slot.clone();
        let Some(step) = state.def.step(&slot.step).cloned() else {
            return Ok(());
        };
        let handle = slot.member.clone();
        let thread = match self.open_step_thread(state, epoch, &slot, &step, session_key, turn)? {
            Ok(thread) => thread,
            Err(port) => return self.refused_start(run, session_key, &handle, port, now),
        };
        let text = match self.compose_turn(&slot, &step, turn, &thread) {
            Ok(text) => text,
            Err(port) => return self.refused_start(run, session_key, &handle, port, now),
        };
        let spec = match self.start_spec(state, &slot, &step, session_key, thread, text, resume)? {
            Ok(spec) => spec,
            Err(port) => return self.refused_start(run, session_key, &handle, port, now),
        };
        crash_at(Boundary::BeforeSpawn);
        wrote("spawn");
        match self.backend.start(&spec) {
            Ok(StartOutcome::Spawned | StartOutcome::Reattached | StartOutcome::Waiting) => {
                self.cache(run).started.insert(session_key.to_owned());
                self.clear_start_refusal(run, session_key)?;
            }
            Ok(StartOutcome::Lost) => {
                self.cache(run).started.insert(session_key.to_owned());
                self.clear_start_refusal(run, session_key)?;
                crash_at(Boundary::AfterEffectBeforeResult);
                let body = EventBody::SessionFact {
                    session_key: session_key.to_owned(),
                    fact: SessionFact::Ended {
                        cause: EndCause::Lost,
                    },
                };
                self.enqueue(
                    run,
                    format!("fact/{session_key}/start-lost"),
                    Origin::Adapter,
                    body,
                    now,
                );
            }
            // A spawn failure is a `started`-less slot: `start_deadline` parks it after its attempts (§13).
            Err(_) => return self.refused_start(run, session_key, &handle, StartPort::Spawn, now),
        }
        Ok(())
    }

    /// The ledger keeps the id before the open; a restart reads it back and repeats the open, which is the
    /// same op under the same id (§8.3). `Ok(Err(port))` is a port that refused: the effect stays owed.
    fn open_step_thread(
        &mut self,
        state: &RunState,
        epoch: Epoch,
        slot: &Slot,
        step: &StepDef,
        session_key: &str,
        turn: &[TurnPart],
    ) -> Result<Result<MessageId, StartPort>, StoreError> {
        let run = &state.run;
        let ledger = format!("thread/{session_key}");
        let fresh = format!("{PENDING}:{}", mint_thread_id());
        let entry = self.entry_or_write(run, &ledger, epoch, &fresh)?;
        if let Some(id) = entry.strip_prefix(DONE) {
            return Ok(Ok(id.to_owned()));
        }
        let Some(thread) = entry
            .strip_prefix(&format!("{PENDING}:"))
            .map(str::to_owned)
        else {
            // Only this file writes the entry, as `pending:<id>` or `done:<id>`: anything else is a damaged row.
            return Err(StoreError::DamagedRow {
                key: Self::effect_key(run, &ledger),
                seq: None,
            });
        };
        crash_at(Boundary::AfterThreadIdBeforeOpen);
        wrote("open");
        let root = state.root_thread.as_deref();
        let opened = self
            .ports
            .messages
            .open_thread(&thread, root, &step.channel, &step.opener);
        if opened.is_err() {
            return Ok(Err(StartPort::OpenThread));
        }
        crash_at(Boundary::AfterOpenBeforeTurnPost);
        let post_key = format!("turn/{session_key}");
        // The turn is composed only when the store has none yet; a refusal there is the declarations', not
        // the chat store's.
        let mut composing = None;
        let posted = self.find_or_post(
            run,
            epoch,
            &thread,
            &step.opener,
            &Refs::default(),
            &post_key,
            |host| {
                host.compose_turn(slot, step, turn, &thread)
                    .map_err(|port| composing = Some(port))
                    .ok()
            },
        );
        if posted.is_none() {
            return Ok(Err(composing.unwrap_or(StartPort::PostTurn)));
        }
        crash_at(Boundary::AfterTurnPostBeforeRoutes);
        self.write_route(run, &thread, Some(session_key))?;
        crash_at(Boundary::BetweenTheRoutes);
        let session = format!("session/{session_key}");
        if self.store.get(&session)?.is_none() {
            wrote("route");
            self.store
                .compare_and_set(&session, None, Some(thread.as_bytes()))?;
        }
        crash_at(Boundary::AfterRoutesBeforeSpawn);
        self.entry_set(run, &ledger, epoch, &format!("{DONE}{thread}"))?;
        Ok(Ok(thread))
    }

    fn write_route(
        &self,
        run: &RunId,
        thread: &str,
        session_key: Option<&str>,
    ) -> Result<(), StoreError> {
        let key = format!("thread/{thread}");
        if self.store.get(&key)?.is_some() {
            return Ok(());
        }
        let route = ThreadRoute {
            run: run.clone(),
            session_key: session_key.map(str::to_owned),
        };
        let value = serde_json::to_vec(&route).map_err(|error| StoreError::Backend {
            what: error.to_string(),
        })?;
        wrote("route");
        self.store.compare_and_set(&key, None, Some(&value))
    }

    /// The parts that carry a message go through the port, which reads and frames them; the task and the
    /// obligation come from the definition and from here. A refusal leaves the effect owed.
    fn compose_turn(
        &self,
        slot: &Slot,
        step: &StepDef,
        turn: &[TurnPart],
        thread: &str,
    ) -> Result<String, StartPort> {
        let handle = slot.member.as_str();
        let mut sections: Vec<String> = Vec::new();
        let mut framed: Vec<TurnPart> = Vec::new();
        for part in turn {
            if part.message.is_some() {
                framed.push(part.clone());
                continue;
            }
            self.flush_framed(handle, &mut framed, &mut sections)?;
            match &part.kind {
                TurnKind::Task => sections.extend(task_text(step, handle)),
                TurnKind::Obligation => sections.push(compose::obligation(thread, step)),
                TurnKind::Reminder => sections.push(compose::REMINDER.to_owned()),
                TurnKind::Root | TurnKind::Reply { .. } | TurnKind::Answer => {}
            }
        }
        self.flush_framed(handle, &mut framed, &mut sections)?;
        if slot.attempt > 1 && step.next.is_none() {
            sections.insert(0, compose::EARLIER_ATTEMPT.to_owned());
        }
        Ok(sections.join("\n\n"))
    }

    fn flush_framed(
        &self,
        handle: &str,
        framed: &mut Vec<TurnPart>,
        sections: &mut Vec<String>,
    ) -> Result<(), StartPort> {
        if !framed.is_empty() {
            let text = self
                .ports
                .declarations
                .turn(handle, framed)
                .map_err(|_| StartPort::Turn)?;
            sections.push(text);
            framed.clear();
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn start_spec(
        &self,
        state: &RunState,
        slot: &Slot,
        step: &StepDef,
        session_key: &str,
        thread: MessageId,
        text: String,
        resume: bool,
    ) -> Result<Result<StartSpec, StartPort>, StoreError> {
        let handle = slot.member.as_str();
        let declarations = &self.ports.declarations;
        let (prompt, tools, declared_tools, model, permissions) = if handle == SYNTH {
            let model = match &step.kind {
                StepKind::FanOut {
                    synthesis: Some(synthesis),
                    ..
                } => synthesis.model.clone(),
                _ => None,
            };
            (
                Prompt {
                    system_prompt: String::new(),
                    use_claude_code_preset: false,
                    declaration_hash: state.flow_hash.clone(),
                },
                vec!["Bash(nxc reply:*)".to_owned()],
                // No persona and so nothing declared of its own: the grant is all the base toolset holds.
                Some(Vec::new()),
                model,
                None,
            )
        } else {
            // The persona first: a handle the declarations do not know fails both, and "no such persona" is the
            // one that says why.
            let Ok(persona) = declarations.persona(handle) else {
                return Ok(Err(StartPort::Persona));
            };
            let Ok(bundle) = declarations.prompt(handle, PromptFor::V2) else {
                return Ok(Err(StartPort::Prompt));
            };
            (
                Prompt {
                    system_prompt: bundle.system_prompt,
                    use_claude_code_preset: bundle.use_claude_code_preset,
                    declaration_hash: bundle.declaration_hash,
                },
                bundle.granted_tools,
                persona.tools,
                persona.model,
                persona.permissions,
            )
        };
        let run = &state.run;
        let resume = if resume {
            self.store
                .get(&format!("persona_session/{run}/{handle}"))?
                .and_then(|bytes| String::from_utf8(bytes).ok())
        } else {
            None
        };
        let Ok(bundle_path) = self.ports.sidecar.bundle_path() else {
            return Ok(Err(StartPort::BundlePath));
        };
        let layout = &self.ports.layout;
        Ok(Ok(StartSpec {
            v: CONTRACT_V,
            session_key: session_key.to_owned(),
            cwd: layout.v2_dir().to_string_lossy().into_owned(),
            env: session_env(&layout.session_env, session_key, handle, &bundle_path),
            prompt,
            turn: Turn { thread, text },
            resume,
            model,
            tools,
            declared_tools,
            permissions,
            ledger_dir: layout
                .ledger_dir(session_key)
                .to_string_lossy()
                .into_owned(),
        }))
    }

    /// A start a port refused stays owed, and `start_deadline` counts its attempts (§13). What it waits for is
    /// said once per session and port, in the log and in `kv['start_refusal/<run>']`, where status reads it:
    /// ids and codes only, never what the port said (M8, PV1).
    fn refused_start(
        &mut self,
        run: &RunId,
        session_key: &str,
        handle: &str,
        port: StartPort,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        let mut refusals = self.start_refusals(run)?;
        if refusals
            .iter()
            .any(|known| known.session_key == session_key && known.port == port)
        {
            return Ok(());
        }
        let code = format!("{session_key} {}", port.as_str());
        refusals.retain(|known| known.session_key != session_key);
        refusals.push(StartRefusal {
            session_key: session_key.to_owned(),
            handle: handle.to_owned(),
            port,
            at: now,
        });
        self.put_start_refusals(run, refusals)?;
        self.log(run, "a session could not start:", &code);
        Ok(())
    }

    fn clear_start_refusal(&self, run: &RunId, session_key: &str) -> Result<(), StoreError> {
        let mut refusals = self.start_refusals(run)?;
        let before = refusals.len();
        refusals.retain(|known| known.session_key != session_key);
        if refusals.len() == before {
            return Ok(());
        }
        self.put_start_refusals(run, refusals)
    }

    fn start_refusals(&self, run: &RunId) -> Result<Vec<StartRefusal>, StoreError> {
        let key = start_refusal_key(run);
        let Some(bytes) = self.store.get(&key)? else {
            return Ok(Vec::new());
        };
        serde_json::from_slice(&bytes).map_err(|_| StoreError::DamagedRow { key, seq: None })
    }

    fn put_start_refusals(
        &self,
        run: &RunId,
        mut refusals: Vec<StartRefusal>,
    ) -> Result<(), StoreError> {
        let key = start_refusal_key(run);
        let current = self.store.get(&key)?;
        refusals.sort_by(|a, b| a.session_key.cmp(&b.session_key));
        let value = if refusals.is_empty() {
            None
        } else {
            Some(
                serde_json::to_vec(&refusals).map_err(|error| StoreError::Backend {
                    what: error.to_string(),
                })?,
            )
        };
        wrote("start_refusal");
        self.store
            .compare_and_set(&key, current.as_deref(), value.as_deref())
    }
}

/// The run's sessions a port refused to start: deleted when the run finishes.
pub(crate) fn start_refusal_key(run: &RunId) -> String {
    format!("start_refusal/{run}")
}

/// A step's task for its members; for the consolidator, its own prompt from the definition.
fn task_text(step: &StepDef, handle: &str) -> Option<String> {
    if handle == SYNTH {
        return match &step.kind {
            StepKind::FanOut {
                synthesis: Some(synthesis),
                ..
            } => Some(synthesis.prompt.clone()),
            _ => None,
        };
    }
    step.task.clone()
}

/// The allowlist: identity is the session's environment, set here (§6.4 item 3, contract (c)).
fn session_env(
    base: &[EnvVar],
    session_key: &str,
    handle: &str,
    bundle: &std::path::Path,
) -> Vec<EnvVar> {
    let var = |name: &str, value: String| EnvVar {
        name: name.to_owned(),
        value,
    };
    let mut env: Vec<EnvVar> = ["PATH", "HOME", "USER"]
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|value| var(name, value)))
        .collect();
    env.extend(base.iter().cloned());
    env.push(var("NXC_SIDECAR", bundle.to_string_lossy().into_owned()));
    env.push(var("NXC_SESSION", session_key.to_owned()));
    env.push(var("NXC_ACTOR", handle.to_owned()));
    env
}
