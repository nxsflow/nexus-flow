//! `StartSession` (§6.4 item 3): the step thread under an id the ledger keeps, the turn composed from ids and
//! posted as the step's commission, the routes a reply needs, all before the spawn. The backend's own ledger
//! answers for a session a crash forgot: `start` on a key it knows does not spawn.

use nexus_run_contracts::{
    EndCause, EnvVar, Epoch, EventBody, MessageId, Origin, Prompt, RunId, RunStore, SessionFact,
    Slot, StartOutcome, StartSpec, StoreError, Timestamp, Turn, CONTRACT_V,
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
        let Some(thread) = self.open_step_thread(state, epoch, &slot, &step, session_key, turn)?
        else {
            return Ok(());
        };
        let Some(text) = self.compose_turn(&slot, &step, turn, &thread) else {
            return Ok(());
        };
        let Some(spec) = self.start_spec(state, &slot, &step, session_key, thread, text, resume)?
        else {
            return Ok(());
        };
        crash_at(Boundary::BeforeSpawn);
        wrote("spawn");
        match self.backend.start(&spec) {
            Ok(StartOutcome::Spawned | StartOutcome::Reattached | StartOutcome::Waiting) => {
                self.cache(run).started.insert(session_key.to_owned());
            }
            Ok(StartOutcome::Lost) => {
                self.cache(run).started.insert(session_key.to_owned());
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
            Err(_) => self.log(run, "a session did not start:", session_key),
        }
        Ok(())
    }

    /// The ledger keeps the id before the open; a restart reads it back and repeats the open, which is the
    /// same op under the same id (§8.3). `Ok(None)` is a port that refused: the effect stays owed.
    fn open_step_thread(
        &mut self,
        state: &RunState,
        epoch: Epoch,
        slot: &Slot,
        step: &StepDef,
        session_key: &str,
        turn: &[TurnPart],
    ) -> Result<Option<MessageId>, StoreError> {
        let run = &state.run;
        let ledger = format!("thread/{session_key}");
        let fresh = format!("{PENDING}:{}", mint_thread_id());
        let entry = self.entry_or_write(run, &ledger, epoch, &fresh)?;
        if let Some(id) = entry.strip_prefix(DONE) {
            return Ok(Some(id.to_owned()));
        }
        let Some(thread) = entry
            .strip_prefix(&format!("{PENDING}:"))
            .map(str::to_owned)
        else {
            return Ok(None);
        };
        crash_at(Boundary::AfterThreadIdBeforeOpen);
        wrote("open");
        let root = state.root_thread.as_deref();
        let opened = self
            .ports
            .messages
            .open_thread(&thread, root, &step.channel, &step.opener);
        if opened.is_err() {
            return Ok(None);
        }
        crash_at(Boundary::AfterOpenBeforeTurnPost);
        let post_key = format!("turn/{session_key}");
        let posted = self.find_or_post(
            run,
            epoch,
            &thread,
            &step.opener,
            &Refs::default(),
            &post_key,
            |host| host.compose_turn(slot, step, turn, &thread),
        );
        if posted.is_none() {
            return Ok(None);
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
        Ok(Some(thread))
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
    /// obligation come from the definition and from here. `None` leaves the effect owed.
    fn compose_turn(
        &self,
        slot: &Slot,
        step: &StepDef,
        turn: &[TurnPart],
        thread: &str,
    ) -> Option<String> {
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
        Some(sections.join("\n\n"))
    }

    fn flush_framed(
        &self,
        handle: &str,
        framed: &mut Vec<TurnPart>,
        sections: &mut Vec<String>,
    ) -> Option<()> {
        if !framed.is_empty() {
            sections.push(self.ports.declarations.turn(handle, framed).ok()?);
            framed.clear();
        }
        Some(())
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
    ) -> Result<Option<StartSpec>, StoreError> {
        let handle = slot.member.as_str();
        let declarations = &self.ports.declarations;
        let (prompt, tools, model) = if handle == SYNTH {
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
                model,
            )
        } else {
            let (Ok(bundle), Ok(persona)) = (
                declarations.prompt(handle, PromptFor::V2),
                declarations.persona(handle),
            ) else {
                return Ok(None);
            };
            (
                Prompt {
                    system_prompt: bundle.system_prompt,
                    use_claude_code_preset: bundle.use_claude_code_preset,
                    declaration_hash: bundle.declaration_hash,
                },
                bundle.granted_tools,
                persona.model,
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
            return Ok(None);
        };
        let layout = &self.ports.layout;
        Ok(Some(StartSpec {
            v: CONTRACT_V,
            session_key: session_key.to_owned(),
            cwd: layout.v2_dir().to_string_lossy().into_owned(),
            env: session_env(&layout.session_env, session_key, handle, &bundle_path),
            prompt,
            turn: Turn { thread, text },
            resume,
            model,
            tools,
            ledger_dir: layout
                .ledger_dir(session_key)
                .to_string_lossy()
                .into_owned(),
        }))
    }
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
