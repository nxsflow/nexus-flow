//! **The executing machine** (nxf 6j6v.1c6k): which ONE machine runs a persona chat, and what the
//! others do instead — see `docs/specs/E4-executing-machine.md` for the decisions and their reasons.
//!
//! Three owner sentences shape everything here. A run started on a machine runs only there, and
//! every machine sees every message without starting anything because of it (2026-08-02). When the
//! executing machine is not there, the person is ASKED, and offered every machine that is online
//! (2026-09-17). The machine is set for the chat, or on the persona (2026-09-21).
//!
//! This module decides WHICH machine and whether to ask; it writes nothing. The designation itself
//! is the thread register [`crate::model::FIELD_MACHINE`], the pickup on the designated machine is
//! [`crate::orchestration::pick_up`], and this module's one seam to the world outside the library is
//! [`Machines`], which the host implements.

use serde::{Deserialize, Serialize};

use crate::error::{NxfError, Result};

/// A chat's `machine` register as read here, with whether the op that wrote it may carry an action
/// on this replica (nxf 6j6v.pzkb's `acting_ops`). A register nobody here can vouch for names a
/// machine that nobody here acts for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadMachine {
    pub machine_id: String,
    pub acts: bool,
}

/// One machine, by its id and its current display name. The id is what is designated and stored;
/// the name is for people and may change at any time (`nxs sync machine <name>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineRef {
    pub machine_id: String,
    pub name: String,
}

/// One machine the workspace's relay has seen, JUDGED — the verdict of
/// `nxs_sync::presence::judge`, which a host hands in as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSeen {
    pub machine_id: String,
    pub name: String,
    pub online: bool,
    /// Seconds since the relay last heard from it — what "last seen N min ago" is made of, which the
    /// question says rather than a bare yes/no for a machine near the edge of its window.
    pub age_secs: u64,
}

/// **What the host knows about machines, as one seam** (nxf 6j6v.1c6k).
///
/// Injected, never resolved in here, for [`crate::orchestration::Ctx::worker`]'s reason one step
/// further out: which machine this is lives in the service home, who is online is a question to the
/// relay, and this library has neither a service home it may assume nor an HTTP client. `nxs`
/// implements it with the service home and the one presence read; a web board implements
/// [`presence`](Machines::presence) from its own relay tables and returns no [`here`](Machines::here).
///
/// **Presence is a claim, never an authorization** (6j6v.f0b5). It decides what the question
/// offers and nothing else: what lets a machine act on an order is the order's signature
/// (`acting_ops`), which no presence answer can change.
pub trait Machines: std::fmt::Debug + Send + Sync {
    /// Whether chats in the workspace at `db_path` are designated at all. `false` for a workspace
    /// that syncs nowhere (`nxs`: bound to no stream): it has exactly one machine, so a designation
    /// would decide nothing, and a chat there starts where it is written and records no machine —
    /// exactly as before the executing machine existed. The default is `true`.
    fn applies_to(&self, _db_path: &str) -> bool {
        true
    }
    /// This machine, when this host executes; `None` for a host that only shows and writes.
    fn here(&self) -> Option<MachineRef>;
    /// The machines the relay of the workspace at `db_path` has seen, judged, asked NOW. `Err` when
    /// that cannot be told — no relay bound, the relay unreachable — with the reason.
    fn presence(&self, db_path: &str) -> std::result::Result<Vec<MachineSeen>, String>;
    /// Claim one message of the workspace at `db_path` for execution on this MACHINE: `Ok(true)`
    /// exactly once per message id, whichever process and whichever replica of the stream asks
    /// first. Per machine rather than per workspace because one machine can hold two replicas of
    /// one stream (6j6v.f0b5, note 3). A host may answer `Ok(true)` without recording anything for
    /// a workspace nothing is ever picked up in (`nxs`: one bound to no stream).
    fn claim_order(&self, db_path: &str, message_id: &str) -> std::result::Result<bool, String>;
    /// Give a claim back after the spawn it was taken for failed, so the next pickup tries again.
    /// Infallible to the caller, who has nothing better to do than go on; a host whose release
    /// FAILS must say so where an operator reads it (`nxs`: stderr, i.e. the service log), because
    /// a claim left behind keeps that message from being picked up on this machine again.
    fn release_order(&self, db_path: &str, message_id: &str);
    /// Which replica of this machine serves a chat: the first `holder` (a workspace db path) to
    /// claim it keeps it, and that holder is returned to every later caller.
    fn claim_thread(&self, thread_id: &str, holder: &str) -> std::result::Result<String, String>;
}

/// Where a chat's machine came from, highest precedence first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignationSource {
    /// Named with this very call — `--machine` on `send` or `reply`.
    Choice,
    /// The chat's own register, written when it started or last handed over.
    Chat,
    /// The persona's `machine:` declaration.
    Persona,
    /// Nothing was named, and this is the machine that started the chat (the owner's default).
    StartedHere,
}

/// The machine a chat runs on, resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExecutingMachine {
    pub machine_id: String,
    /// The name it goes by, when this machine or the relay knows one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub source: DesignationSource,
    /// Whether it is the machine answering this call — the one that starts the persona right away.
    pub here: bool,
    /// Whether the relay reports it online; `None` for this machine (it is, by asking) and when
    /// presence could not be read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_secs: Option<u64>,
}

impl ExecutingMachine {
    /// "mac-studio (01j…)" — the name when there is one, always the id.
    pub fn label(&self) -> String {
        match &self.name {
            Some(n) if !n.is_empty() => format!("{n} ({})", self.machine_id),
            _ => self.machine_id.clone(),
        }
    }
}

/// **The question, as data** — what [`resolve`] decided, and everything a surface needs to ask it
/// (`Engine::machine`, `nxc machine`). Declared field order is the JSON contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MachineAnswer {
    /// The machine the chat runs on. `None` when the host names no machine (the chat starts where
    /// it is written, as before) or when nothing could be resolved and the person must choose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine: Option<ExecutingMachine>,
    /// Whether starting now would ASK instead: the machine is not online, a name did not resolve,
    /// or this host executes nothing and nobody named a machine.
    pub must_ask: bool,
    /// Why, in one sentence — the text the refusal carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The machines to choose from: every one the relay reports online, this one included.
    pub online: Vec<MachineSeen>,
    /// Why the online machines could not be read, when they could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_error: Option<String>,
}

impl MachineAnswer {
    fn legacy() -> MachineAnswer {
        MachineAnswer {
            machine: None,
            must_ask: false,
            reason: None,
            online: Vec::new(),
            presence_error: None,
        }
    }

    /// Whether the chat runs on this machine now — true when no machine is designated at all.
    pub fn runs_here(&self) -> bool {
        match &self.machine {
            Some(m) => m.here,
            None => true,
        }
    }

    /// The refusal a verb returns instead of writing anything when [`must_ask`](Self::must_ask).
    /// `retry` names the flag that answers it on this surface (`--machine <id|name>`).
    pub fn question(&self, retry: &str) -> NxfError {
        let why = self
            .reason
            .clone()
            .unwrap_or_else(|| "no machine can be named for this chat".to_string());
        let offer = if self.online.is_empty() {
            match &self.presence_error {
                Some(e) => format!(" Which machines are online could not be read: {e}."),
                None => " No machine is online for this workspace right now.".to_string(),
            }
        } else {
            let list: Vec<String> = self
                .online
                .iter()
                .map(|m| {
                    format!(
                        "{} ({}, last seen {})",
                        m.name,
                        m.machine_id,
                        seen_ago(m.age_secs)
                    )
                })
                .collect();
            format!(" Online: {}.", list.join("; "))
        };
        NxfError::validation(format!(
            "{why}. Nothing was sent. Choose the machine for this chat with {retry}.{offer}"
        ))
    }
}

/// "just now", "4 min ago", "3 h ago", "2 d ago".
pub fn seen_ago(age_secs: u64) -> String {
    match age_secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{} min ago", age_secs / 60),
        3600..=86_399 => format!("{} h ago", age_secs / 3600),
        _ => format!("{} d ago", age_secs / 86_400),
    }
}

/// What a caller wants, before resolution: the machine named with this call, and the one recorded
/// (the chat's register) or declared (the persona's `machine:`) — whichever applies to the verb.
#[derive(Debug, Clone, Copy, Default)]
pub struct Wanted<'a> {
    /// `--machine` on this call.
    pub choice: Option<&'a str>,
    /// The chat's register, already known to act here.
    pub chat: Option<&'a str>,
    /// The persona's declaration.
    pub persona: Option<&'a str>,
}

/// **Which machine runs this chat, and whether to ask first** (nxf 6j6v.1c6k). Pure over what the
/// host answers: it reads no store and writes nothing.
///
/// Precedence: the choice made with this call, then the chat's register, then the persona's
/// declaration, then the machine asking. A choice made with this call is honoured even for a machine
/// that is not online — a person chose it knowingly, and the order waits in the log until the
/// machine is back ("late, not lost"). Anything else that names a machine that is not online, and a
/// name that does not resolve, ASKS.
pub fn resolve(
    machines: Option<&dyn Machines>,
    db_path: &str,
    wanted: Wanted<'_>,
) -> MachineAnswer {
    let Some(m) = machines.filter(|m| m.applies_to(db_path)) else {
        return MachineAnswer::legacy();
    };
    let here = m.here();
    let named = [
        (wanted.choice, DesignationSource::Choice),
        (wanted.chat, DesignationSource::Chat),
        (wanted.persona, DesignationSource::Persona),
    ]
    .into_iter()
    .find_map(|(w, src)| w.filter(|w| !w.trim().is_empty()).map(|w| (w.trim(), src)));

    // This machine answers for itself without asking the relay: it is online by asking.
    let is_here = |w: &str| {
        here.as_ref()
            .is_some_and(|h| h.machine_id == w || h.name.eq_ignore_ascii_case(w))
    };
    let here_machine = |source| {
        here.as_ref().map(|h| ExecutingMachine {
            machine_id: h.machine_id.clone(),
            name: Some(h.name.clone()),
            source,
            here: true,
            online: None,
            age_secs: None,
        })
    };
    match named {
        None => {
            if let Some(machine) = here_machine(DesignationSource::StartedHere) {
                return MachineAnswer {
                    machine: Some(machine),
                    ..MachineAnswer::legacy()
                };
            }
            let (online, presence_error) = online_machines(m, db_path);
            MachineAnswer {
                machine: None,
                must_ask: true,
                reason: Some(
                    "this host runs no agent sessions itself, and neither the chat nor the \
                     persona names a machine"
                        .to_string(),
                ),
                online,
                presence_error,
            }
        }
        Some((w, source)) if is_here(w) => MachineAnswer {
            machine: here_machine(source),
            ..MachineAnswer::legacy()
        },
        Some((w, source)) => {
            let seen = m.presence(db_path);
            let (all, presence_error) = match seen {
                Ok(all) => (all, None),
                Err(e) => (Vec::new(), Some(e)),
            };
            let online: Vec<MachineSeen> = all.iter().filter(|s| s.online).cloned().collect();
            let by_id = all.iter().find(|s| s.machine_id == w);
            let by_name: Vec<&MachineSeen> = all
                .iter()
                .filter(|s| s.name.eq_ignore_ascii_case(w))
                .collect();
            let found = by_id.or(match by_name.as_slice() {
                [one] => Some(*one),
                _ => None,
            });
            let Some(found) = found else {
                let reason = match (&presence_error, by_name.len()) {
                    (Some(_), _) => format!(
                        "{} names machine '{w}', and whether it is online cannot be told",
                        source_phrase(source)
                    ),
                    (None, n) if n > 1 => format!(
                        "{} names machine '{w}', and {n} machines go by that name — name it by its id",
                        source_phrase(source)
                    ),
                    (None, _) => format!(
                        "{} names machine '{w}', which this workspace's relay has not seen",
                        source_phrase(source)
                    ),
                };
                return MachineAnswer {
                    machine: None,
                    must_ask: true,
                    reason: Some(reason),
                    online,
                    presence_error,
                };
            };
            let machine = ExecutingMachine {
                machine_id: found.machine_id.clone(),
                name: Some(found.name.clone()),
                source,
                here: false,
                online: Some(found.online),
                age_secs: Some(found.age_secs),
            };
            let must_ask = !found.online && source != DesignationSource::Choice;
            let reason = must_ask.then(|| {
                format!(
                    "{} names {}, which is not online (last seen {})",
                    source_phrase(source),
                    machine.label(),
                    seen_ago(found.age_secs)
                )
            });
            MachineAnswer {
                machine: Some(machine),
                must_ask,
                reason,
                online,
                presence_error,
            }
        }
    }
}

fn source_phrase(source: DesignationSource) -> &'static str {
    match source {
        DesignationSource::Choice => "the machine chosen for this chat",
        DesignationSource::Chat => "this chat",
        DesignationSource::Persona => "the persona's declaration",
        DesignationSource::StartedHere => "this machine",
    }
}

fn online_machines(m: &dyn Machines, db_path: &str) -> (Vec<MachineSeen>, Option<String>) {
    match m.presence(db_path) {
        Ok(all) => (all.into_iter().filter(|s| s.online).collect(), None),
        Err(e) => (Vec::new(), Some(e)),
    }
}

/// An in-memory [`Machines`] for tests and for hosts that want to script one: a fixed `here`, a
/// fixed presence answer, and claims held in the value itself.
#[derive(Debug)]
pub struct FixedMachines {
    pub here: Option<MachineRef>,
    pub presence: std::result::Result<Vec<MachineSeen>, String>,
    claims: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

impl FixedMachines {
    pub fn new(
        here: Option<MachineRef>,
        presence: std::result::Result<Vec<MachineSeen>, String>,
    ) -> FixedMachines {
        FixedMachines {
            here,
            presence,
            claims: Default::default(),
        }
    }

    /// Whether `message_id` is claimed — for a test to look at what a pickup took.
    pub fn claimed(&self, message_id: &str) -> bool {
        self.claims
            .lock()
            .expect("claims")
            .contains_key(&format!("msg-{message_id}"))
    }
}

impl Machines for FixedMachines {
    fn here(&self) -> Option<MachineRef> {
        self.here.clone()
    }
    fn presence(&self, _db_path: &str) -> std::result::Result<Vec<MachineSeen>, String> {
        self.presence.clone()
    }
    fn claim_order(&self, _db_path: &str, message_id: &str) -> std::result::Result<bool, String> {
        let mut c = self.claims.lock().expect("claims");
        let key = format!("msg-{message_id}");
        if c.contains_key(&key) {
            return Ok(false);
        }
        c.insert(key, String::new());
        Ok(true)
    }
    fn release_order(&self, _db_path: &str, message_id: &str) {
        self.claims
            .lock()
            .expect("claims")
            .remove(&format!("msg-{message_id}"));
    }
    fn claim_thread(&self, thread_id: &str, holder: &str) -> std::result::Result<String, String> {
        let mut c = self.claims.lock().expect("claims");
        Ok(c.entry(format!("thread-{thread_id}"))
            .or_insert_with(|| holder.to_string())
            .clone())
    }
}

/// Resolve and refuse in one step: `Ok(answer)` when the chat may start, the question as an error
/// when it must ask. What the writing verbs call BEFORE they write anything.
pub fn resolve_or_ask(
    machines: Option<&dyn Machines>,
    db_path: &str,
    wanted: Wanted<'_>,
    retry: &str,
) -> Result<MachineAnswer> {
    let answer = resolve(machines, db_path, wanted);
    if answer.must_ask {
        return Err(answer.question(retry));
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laptop() -> MachineRef {
        MachineRef {
            machine_id: "01jlaptop".into(),
            name: "laptop".into(),
        }
    }

    fn seen(id: &str, name: &str, online: bool, age: u64) -> MachineSeen {
        MachineSeen {
            machine_id: id.into(),
            name: name.into(),
            online,
            age_secs: age,
        }
    }

    fn room() -> FixedMachines {
        FixedMachines::new(
            Some(laptop()),
            Ok(vec![
                seen("01jlaptop", "laptop", true, 5),
                seen("01jstudio", "studio", true, 90),
                seen("01jold", "old-mac", false, 7200),
            ]),
        )
    }

    #[test]
    fn a_host_that_names_no_machine_designates_nothing_and_runs_here() {
        let a = resolve(None, "db", Wanted::default());
        assert_eq!(a.machine, None);
        assert!(!a.must_ask && a.runs_here());
    }

    #[test]
    fn nothing_named_is_the_machine_that_starts_the_chat() {
        let m = room();
        let a = resolve(Some(&m), "db", Wanted::default());
        let machine = a.machine.clone().unwrap();
        assert_eq!(machine.machine_id, "01jlaptop");
        assert_eq!(machine.source, DesignationSource::StartedHere);
        assert!(machine.here && !a.must_ask && a.runs_here());
    }

    #[test]
    fn the_choice_beats_the_chat_beats_the_persona() {
        let m = room();
        let a = resolve(
            Some(&m),
            "db",
            Wanted {
                choice: Some("studio"),
                chat: Some("01jlaptop"),
                persona: Some("01jold"),
            },
        );
        assert_eq!(a.machine.unwrap().source, DesignationSource::Choice);
        let a = resolve(
            Some(&m),
            "db",
            Wanted {
                choice: None,
                chat: Some("01jstudio"),
                persona: Some("01jlaptop"),
            },
        );
        let machine = a.machine.unwrap();
        assert_eq!(
            (machine.machine_id.as_str(), machine.source, machine.here),
            ("01jstudio", DesignationSource::Chat, false)
        );
    }

    #[test]
    fn a_persona_naming_this_machine_by_name_runs_here_without_asking_the_relay() {
        let m = FixedMachines::new(Some(laptop()), Err("relay unreachable".into()));
        let a = resolve(
            Some(&m),
            "db",
            Wanted {
                persona: Some("LAPTOP"),
                ..Wanted::default()
            },
        );
        assert!(a.runs_here() && !a.must_ask);
        assert_eq!(a.machine.unwrap().source, DesignationSource::Persona);
    }

    #[test]
    fn a_designated_machine_that_is_online_elsewhere_is_handed_the_chat() {
        let m = room();
        let a = resolve(
            Some(&m),
            "db",
            Wanted {
                persona: Some("studio"),
                ..Wanted::default()
            },
        );
        assert!(!a.must_ask && !a.runs_here());
        let machine = a.machine.unwrap();
        assert_eq!(machine.machine_id, "01jstudio");
        assert_eq!(machine.online, Some(true));
    }

    #[test]
    fn a_designated_machine_that_is_not_online_asks_and_offers_the_online_ones() {
        let m = room();
        let a = resolve(
            Some(&m),
            "db",
            Wanted {
                persona: Some("01jold"),
                ..Wanted::default()
            },
        );
        assert!(a.must_ask);
        let ids: Vec<&str> = a.online.iter().map(|s| s.machine_id.as_str()).collect();
        assert_eq!(ids, ["01jlaptop", "01jstudio"]);
        let msg = a.question("--machine <id|name>").msg;
        assert!(
            msg.contains("old-mac (01jold), which is not online (last seen 2 h ago)"),
            "{msg}"
        );
        assert!(msg.contains("Nothing was sent"), "{msg}");
        assert!(
            msg.contains("studio (01jstudio, last seen 1 min ago)"),
            "{msg}"
        );
    }

    #[test]
    fn a_machine_chosen_for_the_chat_is_honoured_even_when_it_is_not_online() {
        let m = room();
        let a = resolve(
            Some(&m),
            "db",
            Wanted {
                choice: Some("old-mac"),
                ..Wanted::default()
            },
        );
        assert!(!a.must_ask);
        assert_eq!(a.machine.unwrap().online, Some(false));
    }

    #[test]
    fn a_name_the_relay_never_saw_or_two_machines_share_asks() {
        let m = room();
        let a = resolve(
            Some(&m),
            "db",
            Wanted {
                choice: Some("typo"),
                ..Wanted::default()
            },
        );
        assert!(a.must_ask && a.machine.is_none());
        let twins = FixedMachines::new(
            Some(laptop()),
            Ok(vec![
                seen("01ja", "mac", true, 1),
                seen("01jb", "mac", true, 1),
            ]),
        );
        let a = resolve(
            Some(&twins),
            "db",
            Wanted {
                choice: Some("mac"),
                ..Wanted::default()
            },
        );
        assert!(a.must_ask);
        assert!(a.reason.unwrap().contains("name it by its id"));
    }

    #[test]
    fn a_host_that_executes_nothing_and_names_nothing_asks() {
        let web = FixedMachines::new(None, Ok(vec![seen("01jstudio", "studio", true, 3)]));
        let a = resolve(Some(&web), "db", Wanted::default());
        assert!(a.must_ask && a.machine.is_none());
        assert_eq!(a.online.len(), 1);
    }

    #[test]
    fn an_order_is_claimed_once_and_a_thread_keeps_its_first_holder() {
        let m = room();
        assert!(m.claim_order("db", "m-1").unwrap());
        assert!(!m.claim_order("db", "m-1").unwrap());
        m.release_order("db", "m-1");
        assert!(m.claim_order("db", "m-1").unwrap());
        assert_eq!(m.claim_thread("t-1", "a.db").unwrap(), "a.db");
        assert_eq!(m.claim_thread("t-1", "b.db").unwrap(), "a.db");
    }

    #[test]
    fn seen_ago_reads_like_a_person_says_it() {
        assert_eq!(seen_ago(10), "just now");
        assert_eq!(seen_ago(125), "2 min ago");
        assert_eq!(seen_ago(7300), "2 h ago");
        assert_eq!(seen_ago(200_000), "2 d ago");
    }
}
