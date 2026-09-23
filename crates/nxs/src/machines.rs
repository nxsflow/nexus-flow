//! **This machine, who else is online, and the per-machine claims** — the `nxs` binary's answer to
//! [`nexus_chat::machine::Machines`] (nxf 6j6v.1c6k).
//!
//! The chat library decides which machine runs a persona chat and when to ask; it cannot know
//! which machine it is standing on (that is the service home's `machine.toml`) nor who is online
//! (that is a question to the relay, and the library has no HTTP client). This is the composition
//! root's half: the service home for the first and for the claims, the ONE presence read
//! (`nxs sync machines`'s own, judged by `nxs_sync::presence`) for the second — asked live, at the
//! moment a chat starts, never from a cache (`docs/specs/E4-executing-machine.md` §2.6).

use std::path::Path;
use std::sync::OnceLock;

use nexus_chat::machine::{MachineRef, MachineSeen, Machines};
use nxs_foundation::workspace::Workspace;
use nxs_service::ServiceHome;
use nxs_sync::presence::Presence;

/// The one instance the multicall dispatch hands `nexus_chat::run_from_with` — `'static` for the
/// same reason `prime::SIBLING_PRIMES` is.
pub static SERVICE_MACHINES: ServiceMachines = ServiceMachines {
    here: OnceLock::new(),
};

/// The `nxs` binary's [`Machines`]: the service home this binary resolves (the running binary
/// decides the instance) and the workspace's relay.
#[derive(Debug)]
pub struct ServiceMachines {
    here: OnceLock<Option<MachineRef>>,
}

/// Whether the workspace at `db_path` is bound to a stream. **Only then is anything claimed**: the
/// service picks orders up only in a workspace it syncs, and nothing reaches an unbound one from
/// another replica — so a claim there would guard against nothing, and every `nxc send` in a
/// local-only workspace would leave a file in the service home for no reader. The cost, named: a
/// workspace bound AFTER a chat started here leaves that chat's messages unclaimed, and a pickup
/// then finds the persona's own session and resumes it with them once more (held while it works).
fn syncs(db_path: &str) -> bool {
    Workspace::resolve(Some(db_path), Path::new("."))
        .ok()
        .and_then(|ws| crate::sync::load(&ws).ok().flatten())
        .is_some()
}

fn home() -> Result<ServiceHome, String> {
    ServiceHome::resolve().map_err(|e| e.msg)
}

impl Machines for ServiceMachines {
    fn applies_to(&self, db_path: &str) -> bool {
        syncs(db_path)
    }

    fn here(&self) -> Option<MachineRef> {
        self.here
            .get_or_init(
                || match home().and_then(|h| h.machine().map_err(|e| e.msg)) {
                    Ok(m) => Some(MachineRef {
                        machine_id: m.id,
                        name: m.name,
                    }),
                    Err(e) => {
                        eprintln!(
                        "warning: this machine's identity could not be read ({e}), so no chat can \
                         be designated to it"
                    );
                        None
                    }
                },
            )
            .clone()
    }

    fn presence(&self, db_path: &str) -> Result<Vec<MachineSeen>, String> {
        let ws = Workspace::resolve(Some(db_path), Path::new(".")).map_err(|e| e.msg)?;
        let reading = crate::sync::read_machines(&ws, None).map_err(|e| e.msg)?;
        match reading.presence {
            Presence::Reported { machines, .. } => Ok(machines
                .into_iter()
                .map(|m| MachineSeen {
                    machine_id: m.machine_id,
                    name: m.name,
                    online: m.online,
                    age_secs: m.age_secs,
                })
                .collect()),
            Presence::Unsupported => Err(format!(
                "the relay at {} does not record which machines sync this workspace",
                reading.endpoint
            )),
        }
    }

    fn claim_order(&self, db_path: &str, message_id: &str) -> Result<bool, String> {
        if !syncs(db_path) {
            return Ok(true);
        }
        home()?.claim_order(message_id).map_err(|e| e.msg)
    }

    fn release_order(&self, db_path: &str, message_id: &str) {
        if !syncs(db_path) {
            return;
        }
        // Said, never swallowed (review of PR #492, Integrity #1): a claim that stays behind keeps
        // this message from ever being picked up here again, and stderr is where the service's log
        // and a terminal both see it.
        match home().and_then(|h| h.release_order(message_id).map_err(|e| e.msg)) {
            Ok(()) => {}
            Err(e) => eprintln!("warning: {e}"),
        }
    }

    fn claim_thread(&self, thread_id: &str, holder: &str) -> Result<String, String> {
        if !syncs(holder) {
            return Ok(holder.to_string());
        }
        home()?.claim_thread(thread_id, holder).map_err(|e| e.msg)
    }
}
