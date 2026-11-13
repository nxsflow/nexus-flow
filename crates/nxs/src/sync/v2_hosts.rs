//! **The new engine's hosts in the background service** (6j6v.pn9c; epic 6j6v.zjyv §8.1, §10).
//!
//! `Host::tick` runs once per iteration of `serve`, per registered workspace, beside `fire_deadlines`. A
//! workspace that never ran on the new engine has no `.nxs/run.sqlite` and costs one `stat`; one that has it
//! gets a host, built once and kept across ticks — a host is its lease's owner, and a fresh one per tick
//! would wait out its own predecessor's lease every second. Its sessions run through the Claude adapter, its
//! tree operations on its own worker thread, so a tick never waits on a session or on git.
// v2-bridge: deleted with the old engine (6j6v.2vtw) — when the old engine goes, this is the service's only engine.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use nexus_chat::facade::ModulePrimes;
use nexus_run_host::{Host, LogSink};
use nxs_foundation::workspace::Workspace;

/// One host per registered workspace with a `run.sqlite`, kept for the life of the service.
pub struct V2Hosts {
    hosts: BTreeMap<PathBuf, Host>,
    /// Workspaces whose host could not be built, so the reason is logged once and not every second.
    refused: BTreeSet<PathBuf>,
    primes: Arc<dyn ModulePrimes>,
    /// The service's log, which every host writes its lines to as well.
    log: LogSink,
}

impl V2Hosts {
    pub fn new(primes: Arc<dyn ModulePrimes>, log: LogSink) -> Self {
        Self {
            hosts: BTreeMap::new(),
            refused: BTreeSet::new(),
            primes,
            log,
        }
    }

    /// One tick of every attended workspace's host, building the ones that are missing.
    pub fn tick(&mut self, resolved: &[Workspace]) {
        let attended: BTreeSet<PathBuf> = resolved.iter().filter_map(root_of).collect();
        self.hosts.retain(|root, _| attended.contains(root));
        self.refused.retain(|root| attended.contains(root));
        for ws in resolved {
            let Some(root) = root_of(ws) else {
                continue;
            };
            if !self.hosts.contains_key(&root) {
                match nexus_chat::v2_bridge::service_host(ws, self.primes.clone()) {
                    Ok(None) => {
                        // No `run.sqlite`: the one `stat` this workspace costs, and a store moved aside is
                        // tried afresh once it is back.
                        self.refused.remove(&root);
                        continue;
                    }
                    Ok(Some(mut host)) => {
                        self.refused.remove(&root);
                        host.set_log_sink(self.log.clone());
                        self.hosts.insert(root.clone(), host);
                    }
                    Err(error) => {
                        if self.refused.insert(root.clone()) {
                            (self.log)(&format!(
                                "workspace {}: the new engine's runs are not stepped: {error}",
                                root.display()
                            ));
                        }
                        continue;
                    }
                }
            }
            if let Some(host) = self.hosts.get_mut(&root) {
                let now = nexus_run_host::ports::Clock::now(&nexus_run_host::ports::SystemClock);
                host.tick(now);
            }
        }
    }

    /// Whether a session of an open run in any attended workspace may still have a process. A store that does
    /// not answer counts as nothing alive: holding a machine awake on a guess costs its battery (`serve`).
    pub fn any_live(&self) -> bool {
        self.hosts
            .values()
            .any(|host| host.live_sessions().is_ok_and(|live| !live.is_empty()))
    }

    /// The workspaces that have a host, by root.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.hosts.keys().cloned().collect()
    }

    /// The backend a workspace's host runs its sessions through.
    pub fn backend_of(&self, root: &std::path::Path) -> Option<String> {
        self.hosts.get(root).map(Host::backend_name)
    }

    /// A host built elsewhere, for a test that steps it with a scripted backend.
    #[cfg(test)]
    pub(crate) fn insert(&mut self, root: PathBuf, host: Host) {
        self.hosts.insert(root, host);
    }
}

/// The root a workspace's `.nxs` sits in.
fn root_of(ws: &Workspace) -> Option<PathBuf> {
    ws.dir.parent().map(std::path::Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nexus_chat::engine::{Engine, EngineConfig};
    use nexus_chat::orchestration::Caller;
    use nexus_chat::surface::{SendToRefs, SendToRequest};
    use nexus_chat::worker::WorkerConfig;
    use nexus_run_contracts::RunStore;
    use std::sync::Mutex;
    use tempfile::TempDir;

    const CHANNELS: &str = "\
- name: coding-v2
  engine: v2
  members: []
  working_tree: exclusive
  steps:
    - id: build
      target: coder
";

    /// A chat workspace with the new engine's channel declared.
    fn workspace() -> (TempDir, Workspace) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        nexus_chat::workspace::setup(&root, &nexus_chat::workspace::chat_config()).unwrap();
        let personas = root.join(".nxs-personas");
        std::fs::create_dir_all(&personas).unwrap();
        std::fs::write(
            personas.join("coder.yaml"),
            "handle: coder\njob_title: Implementer\naddressable: none\nsystem_prompt: You code.\n",
        )
        .unwrap();
        std::fs::write(personas.join("channels.yaml"), CHANNELS).unwrap();
        let ws = Workspace::resolve(None, &root).unwrap();
        (tmp, ws)
    }

    fn root_of(ws: &Workspace) -> PathBuf {
        ws.dir.parent().unwrap().to_path_buf()
    }

    fn run_sqlite(ws: &Workspace) -> PathBuf {
        ws.dir.join("run.sqlite")
    }

    /// The owner at the terminal commissions `coding-v2`: the bridge writes `run.sqlite`.
    fn commission(ws: &Workspace) {
        let engine = Engine::open_with(
            None,
            &root_of(ws),
            EngineConfig {
                worker: WorkerConfig::Dry { log: None },
                ..EngineConfig::default()
            },
        )
        .unwrap();
        engine
            .send_to(
                Caller {
                    session: None,
                    actor: Some("owner"),
                    now: None,
                },
                SendToRequest {
                    to: "coding-v2",
                    body: "build it",
                    refs: SendToRefs::declared(nexus_chat::model::Refs {
                        nxf_ids: vec!["6j6v.pn9c".to_owned()],
                        ..nexus_chat::model::Refs::default()
                    }),
                    machine: None,
                },
            )
            .unwrap();
    }

    /// The hosts, and every line they log.
    fn hosts() -> (V2Hosts, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let log: LogSink = Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_owned()));
        (
            V2Hosts::new(Arc::new(crate::prime::RegistrySiblingPrimes), log),
            seen,
        )
    }

    #[test]
    fn a_workspace_that_never_ran_on_the_new_engine_gets_no_host_and_no_file() {
        let (_tmp, ws) = workspace();
        let (mut v2, _) = hosts();
        v2.tick(std::slice::from_ref(&ws));
        assert!(v2.roots().is_empty());
        assert!(!run_sqlite(&ws).exists(), "a tick creates no run store");
        assert!(!v2.any_live());
    }

    #[test]
    fn a_workspace_with_a_run_store_is_ticked_with_the_claude_adapter_under_its_chat_origin() {
        let (_tmp, ws) = workspace();
        commission(&ws);
        let (mut v2, _) = hosts();
        // A few service ticks: the first takes the lease and folds the commission, a later one opens the root.
        for _ in 0..5 {
            v2.tick(std::slice::from_ref(&ws));
        }
        let root = root_of(&ws);
        assert_eq!(v2.roots(), [root.clone()]);
        assert_eq!(v2.backend_of(&root).as_deref(), Some("claude"));
        // The host marked itself seen under the id the bridge commissioned with and reads (§8.1).
        let store = nexus_run_host::bridge::open_store(&run_sqlite(&ws))
            .unwrap()
            .unwrap();
        let origin = nexus_chat::workspace::origin_of(&ws);
        assert!(
            store.host_seen(origin).unwrap().is_some(),
            "a service attends {origin}"
        );
        // And it stepped the run: the host opened the root, which only an owner of the run's lease does.
        let run = store.open_runs().unwrap()[0].run.clone();
        let kinds: Vec<String> = store
            .load(&run, 0)
            .unwrap()
            .iter()
            .map(|event| event.body.kind().to_owned())
            .collect();
        assert!(
            kinds.iter().any(|kind| kind == "root_opened"),
            "the tick stepped the run: {kinds:?}"
        );
    }

    #[test]
    fn a_workspace_no_longer_attended_loses_its_host() {
        let (_tmp, ws) = workspace();
        commission(&ws);
        let (mut v2, _) = hosts();
        v2.tick(std::slice::from_ref(&ws));
        assert_eq!(v2.roots().len(), 1);
        v2.tick(&[]);
        assert!(
            v2.roots().is_empty(),
            "an unregistered workspace is not ticked"
        );
    }

    #[test]
    fn a_host_that_cannot_be_built_is_named_once_and_not_every_tick() {
        let (_tmp, ws) = workspace();
        // A run store that is no database at all.
        std::fs::create_dir_all(run_sqlite(&ws)).unwrap();
        let (mut v2, seen) = hosts();
        for _ in 0..3 {
            v2.tick(std::slice::from_ref(&ws));
        }
        assert!(v2.roots().is_empty());
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "once: {seen:?}");
        assert!(
            seen[0].contains(&root_of(&ws).display().to_string()),
            "{seen:?}"
        );
    }

    #[test]
    fn a_live_session_of_a_run_on_the_new_engine_holds_the_machine_awake() {
        use nexus_run_host::test_support::scripted_backend::{Cue, SessionInfo};
        use nexus_run_host::test_support::{Rig, CLOCK_START};
        fn silent(_: &SessionInfo) -> Cue {
            Cue::Silent
        }
        let dir = TempDir::new().unwrap();
        let mut rig = Rig::scripted(dir.path(), CLOCK_START, Box::new(silent));
        let run = rig.commission("build it").run;
        rig.run_until(400, |r| {
            r.state(&run).slots.iter().any(|s| s.state.is_live())
        });
        let Rig { host, .. } = rig;
        let (mut v2, _) = hosts();
        assert!(!v2.any_live());
        v2.insert(dir.path().join("root"), host);
        assert!(v2.any_live(), "a live v2 session holds the machine awake");
        assert!(super::super::daemon::a_run_is_alive(&[], &v2));
    }
}
