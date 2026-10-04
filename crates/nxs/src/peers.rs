//! **The other workspaces on this machine** — the `nxs` binary's answer to
//! [`nexus_chat::border::Peers`] (nxf 6j6v.4gp2).
//!
//! The chat library carries a conversation across the border of a workspace; it cannot know which
//! workspaces this machine has (that is the service registry, `~/.nexusflow/workspaces.toml`) nor
//! how to run the other side's handover (that is this binary, started again in the other
//! workspace's directory). This is the composition root's half, like `crate::machines`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nexus_chat::border::{PeerWorkspace, Peers};
use nxs_foundation::workspace::{discover, Workspace};
use nxs_foundation::workspace_name;

/// The one instance the multicall dispatch hands the chat CLI.
pub static SERVICE_PEERS: ServicePeers = ServicePeers;

/// How long a peer's handover may take before this side stops waiting for it. It starts a persona
/// session at most, which it does not wait for itself; the service's pass covers whatever is left.
const HANDOVER_WAIT: Duration = Duration::from_secs(60);

/// The variables that tell a process WHICH workspace and WHICH session it acts for. A peer's
/// handover runs in the peer's directory and must find the peer's workspace there — inheriting
/// these from a persona session of this one would make it act on this workspace again, as that
/// persona.
const WORKSPACE_ENV: &[&str] = &[
    "NXC_DB",
    "NXS_DB",
    "NXF_DB",
    "NXM_DB",
    "NXC_SESSION",
    "NXC_ACTOR",
    "NXS_ACTOR",
    "NXF_ACTOR",
    "NXC_ORIGIN",
    "NXC_HOP",
];

/// The `nxs` binary's [`Peers`].
#[derive(Debug)]
pub struct ServicePeers;

fn registered_roots() -> Result<Vec<PathBuf>, String> {
    let entries = crate::workspaces::load_registered().map_err(|e| e.msg)?;
    let mut roots: Vec<PathBuf> = Vec::new();
    for e in entries {
        let root = PathBuf::from(&e.path);
        if !root.join(".nxs").join("replica.toml").is_file() {
            continue;
        }
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        if !roots
            .iter()
            .any(|r| r.canonicalize().unwrap_or_else(|_| r.clone()) == canonical)
        {
            roots.push(root);
        }
    }
    Ok(roots)
}

impl Peers for ServicePeers {
    fn here(&self, db_path: &str) -> Result<Option<String>, String> {
        let ws = Workspace::resolve(Some(db_path), Path::new(".")).map_err(|e| e.msg)?;
        workspace_name::ensure_name(&ws).map_err(|e| e.msg)
    }

    fn find(&self, name: &str) -> Result<Vec<PeerWorkspace>, String> {
        let mut found = Vec::new();
        for root in registered_roots()? {
            let Ok(ws) = discover(&root) else { continue };
            if workspace_name::name_of(&ws).as_deref() == Some(name) {
                found.push(PeerWorkspace {
                    name: name.to_string(),
                    root,
                });
            }
        }
        Ok(found)
    }

    fn hand_over_in(&self, peer: &PeerWorkspace) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| format!("this binary has no path: {e}"))?;
        let mut cmd = std::process::Command::new(exe);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            cmd.arg0("nxs");
        }
        cmd.args(["chat", "handover"])
            .current_dir(&peer.root)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        for var in WORKSPACE_ENV {
            cmd.env_remove(var);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start the handover in {}: {e}", peer.name))?;
        let deadline = Instant::now() + HANDOVER_WAIT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => {
                    return Err(format!(
                        "the handover in {} exited with {status}",
                        peer.name
                    ))
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Ok(None) => {
                    return Err(format!(
                        "the handover in {} is still running after {}s; it goes on by itself",
                        peer.name,
                        HANDOVER_WAIT.as_secs()
                    ))
                }
                Err(e) => return Err(format!("waiting for the handover in {}: {e}", peer.name)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handover_that_cannot_start_in_the_other_workspace_is_an_error_not_a_silence() {
        // Review of PR #14, Test Quality #2: the failure path of the peer's handover. A root that
        // does not exist (a moved or deleted clone still in the registry) cannot be started in.
        let peer = PeerWorkspace {
            name: "test/gone".into(),
            root: std::env::temp_dir().join("nxs-peers-test-no-such-workspace-6j6v"),
        };
        let err = SERVICE_PEERS
            .hand_over_in(&peer)
            .expect_err("nothing to start in");
        assert!(err.contains("test/gone"), "{err}");
    }
}
