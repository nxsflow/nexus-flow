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
use nexus_run_host::{Host, LogSink, TickReport};
use nxs_foundation::workspace::Workspace;

/// One host per registered workspace with a `run.sqlite`, kept for the life of the service.
pub struct V2Hosts {
    hosts: BTreeMap<PathBuf, Host>,
    /// Workspaces whose host could not be built, so the reason is logged once and not every second.
    refused: BTreeSet<PathBuf>,
    primes: Arc<dyn ModulePrimes>,
    /// The service's log, which every host writes its lines to as well.
    log: LogSink,
    /// How many ticks one measures line covers: the smoke run reads the bounds it observed from it (§12.10).
    pub(crate) stats_every: usize,
    window: Window,
    /// Workspaces whose host panicked: how many times, and how many service ticks to wait before a rebuild.
    panicked: BTreeMap<PathBuf, Panicked>,
    /// The pause after a panic, in service ticks.
    pub(crate) panic_backoff: u32,
}

/// A host that panicked, until one of its ticks succeeds again.
struct Panicked {
    count: u32,
    wait: u32,
}

/// A minute of the service's ticks: a host that panics every tick is rebuilt once a minute, not every second.
const PANIC_BACKOFF: u32 = 60;

/// What the ticks of one window measured, across every host.
#[derive(Default)]
struct Window {
    /// Service ticks in the window: the cadence, whatever the number of hosts.
    service_ticks: usize,
    /// One duration per host tick.
    ticks: Vec<std::time::Duration>,
    busy: u32,
    yielded: u32,
    folded: u64,
}

/// Ten minutes of the service's one-second ticks, counted once per service tick however many hosts it steps.
const STATS_EVERY: usize = 600;

impl V2Hosts {
    pub fn new(primes: Arc<dyn ModulePrimes>, log: LogSink) -> Self {
        Self {
            hosts: BTreeMap::new(),
            refused: BTreeSet::new(),
            primes,
            log,
            stats_every: STATS_EVERY,
            window: Window::default(),
            panicked: BTreeMap::new(),
            panic_backoff: PANIC_BACKOFF,
        }
    }

    /// One tick of every attended workspace's host, building the ones that are missing.
    pub fn tick(&mut self, resolved: &[Workspace]) {
        let attended: BTreeSet<PathBuf> = resolved.iter().map(root_of).collect();
        self.hosts.retain(|root, _| attended.contains(root));
        self.refused.retain(|root| attended.contains(root));
        self.panicked.retain(|root, _| attended.contains(root));
        for ws in resolved {
            let root = root_of(ws);
            if let Some(panicked) = self.panicked.get_mut(&root) {
                if panicked.wait > 0 {
                    panicked.wait -= 1;
                    continue;
                }
            }
            if !self.ensure_host(ws, &root) {
                continue;
            }
            let now = nexus_run_host::ports::Clock::now(&nexus_run_host::ports::SystemClock);
            self.tick_one(&root, |host| host.tick(now));
        }
        if !self.hosts.is_empty() {
            self.window.service_ticks += 1;
        }
        if self.window.service_ticks >= self.stats_every && !self.window.ticks.is_empty() {
            let window = std::mem::take(&mut self.window);
            (self.log)(&window.line());
        }
    }

    /// The host of `ws`, built when it is missing: `false` where there is none to tick (no `run.sqlite`, or a
    /// build that failed and was named).
    pub(crate) fn ensure_host(&mut self, ws: &Workspace, root: &PathBuf) -> bool {
        if self.hosts.contains_key(root) {
            return true;
        }
        match nexus_chat::v2_bridge::service_host(ws, self.primes.clone()) {
            Ok(None) => {
                // No `run.sqlite`: the one `stat` this workspace costs, and a store moved aside is
                // tried afresh once it is back.
                self.refused.remove(root);
                return false;
            }
            Ok(Some(mut host)) => {
                self.refused.remove(root);
                host.set_log_sink(self.log.clone());
                self.hosts.insert(root.clone(), host);
            }
            // Retried every tick, deliberately: the cost is a stat, a declarations read and an open,
            // once a second, and the fix a person makes (a store moved back, a declaration repaired)
            // is picked up at the next tick without a restart. Logged once until it changes.
            Err(error) => {
                if self.refused.insert(root.clone()) {
                    (self.log)(&format!(
                        "workspace {}: the new engine's runs are not stepped: {error}",
                        root.display()
                    ));
                }
                return false;
            }
        }
        true
    }

    /// One host's tick, isolated: a panic outside a run's own boundary (the host catches those) must not take the
    /// service down with the old engine's deadlines and handovers. The host is dropped and named on its first
    /// panic; it is rebuilt after `panic_backoff` service ticks, from the store, as a restart would; later panics
    /// are counted, and the first tick that succeeds again says how many there were.
    pub(crate) fn tick_one(&mut self, root: &PathBuf, tick: impl FnOnce(&mut Host) -> TickReport) {
        let Some(host) = self.hosts.get_mut(root) else {
            return;
        };
        let started = std::time::Instant::now();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(host))) {
            Ok(report) => {
                self.window.ticks.push(started.elapsed());
                self.window.busy += u32::from(report.busy);
                self.window.yielded += u32::from(report.yielded);
                self.window.folded += u64::from(report.folded);
                if let Some(panicked) = self.panicked.remove(root) {
                    (self.log)(&format!(
                        "workspace {}: the new engine's host recovered after {} panics",
                        root.display(),
                        panicked.count
                    ));
                }
            }
            Err(_) => {
                self.hosts.remove(root);
                let panicked = self
                    .panicked
                    .entry(root.clone())
                    .or_insert(Panicked { count: 0, wait: 0 });
                panicked.count += 1;
                panicked.wait = self.panic_backoff;
                if panicked.count == 1 {
                    (self.log)(&format!(
                        "workspace {}: the new engine's host panicked; it is built afresh in {} ticks, and further \
                         panics are counted until it recovers",
                        root.display(),
                        self.panic_backoff
                    ));
                }
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
    #[cfg(test)]
    pub(crate) fn roots(&self) -> Vec<PathBuf> {
        self.hosts.keys().cloned().collect()
    }

    /// The backend a workspace's host runs its sessions through.
    #[cfg(test)]
    pub(crate) fn backend_of(&self, root: &std::path::Path) -> Option<String> {
        self.hosts.get(root).map(Host::backend_name)
    }

    /// A host built elsewhere, for a test that steps it with a scripted backend.
    #[cfg(test)]
    pub(crate) fn insert(&mut self, root: PathBuf, host: Host) {
        self.hosts.insert(root, host);
    }
}

impl Window {
    /// Ids and numbers only, like every host line (M8).
    fn line(mut self) -> String {
        self.ticks.sort();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
        let n = self.ticks.len();
        let p99 = self.ticks[(n * 99).div_ceil(100).saturating_sub(1)];
        format!(
            "run-host stats: ticks={} host_ticks={n} p99_ms={:.1} max_ms={:.1} busy={} yielded={} folded={}",
            self.service_ticks,
            ms(p99),
            ms(self.ticks[n - 1]),
            self.busy,
            self.yielded,
            self.folded
        )
    }
}

/// The registered root of a workspace: the one derivation the bridge and the service share.
fn root_of(ws: &Workspace) -> PathBuf {
    nexus_chat::v2_bridge::workspace_root_of(ws)
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

    fn test_root(ws: &Workspace) -> PathBuf {
        ws.dir.parent().unwrap().to_path_buf()
    }

    fn run_sqlite(ws: &Workspace) -> PathBuf {
        ws.dir.join("run.sqlite")
    }

    /// The owner at the terminal commissions `coding-v2`: the bridge writes `run.sqlite`.
    fn commission(ws: &Workspace) {
        let engine = Engine::open_with(
            None,
            &test_root(ws),
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
        // Service ticks until the root is opened, bounded: the first takes the lease and folds the commission.
        let run_store = || {
            nexus_run_host::bridge::open_store(&run_sqlite(&ws))
                .unwrap()
                .unwrap()
        };
        let opened = || {
            let store = run_store();
            let run = store.open_runs().unwrap()[0].run.clone();
            store
                .load(&run, 0)
                .unwrap()
                .iter()
                .any(|event| event.body.kind() == "root_opened")
        };
        let ceiling = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while !opened() {
            assert!(
                std::time::Instant::now() < ceiling,
                "the service never opened the root"
            );
            v2.tick(std::slice::from_ref(&ws));
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let root = test_root(&ws);
        assert_eq!(v2.roots(), std::slice::from_ref(&root));
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
            seen[0].contains(&test_root(&ws).display().to_string()),
            "{seen:?}"
        );
    }

    #[test]
    fn the_hosts_measures_are_logged_once_per_window_of_ticks() {
        let (_tmp, ws) = workspace();
        commission(&ws);
        let (mut v2, seen) = hosts();
        v2.stats_every = 4;
        for _ in 0..9 {
            v2.tick(std::slice::from_ref(&ws));
        }
        let stats: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("run-host stats:"))
            .cloned()
            .collect();
        assert_eq!(stats.len(), 2, "one line per 4 ticks: {stats:?}");
        for field in [
            "ticks=4", "p99_ms=", "max_ms=", "busy=", "yielded=", "folded=",
        ] {
            assert!(stats[0].contains(field), "{field} in {}", stats[0]);
        }
    }

    #[test]
    fn a_window_counts_service_ticks_whatever_the_number_of_hosts() {
        let (_a, alpha) = workspace();
        let (_b, beta) = workspace();
        commission(&alpha);
        commission(&beta);
        let (mut v2, seen) = hosts();
        v2.stats_every = 4;
        for _ in 0..4 {
            v2.tick(&[alpha.clone(), beta.clone()]);
        }
        let stats: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("run-host stats:"))
            .cloned()
            .collect();
        assert_eq!(
            stats.len(),
            1,
            "one line per 4 service ticks, with two hosts: {stats:?}"
        );
        assert!(
            stats[0].contains("ticks=4 ") && stats[0].contains("host_ticks=8"),
            "{}",
            stats[0]
        );
    }

    #[test]
    fn a_panicking_host_is_dropped_named_once_rebuilt_after_a_pause_and_its_recovery_said() {
        let (_a, alpha) = workspace();
        let (_b, beta) = workspace();
        commission(&alpha);
        commission(&beta);
        let (mut v2, seen) = hosts();
        v2.panic_backoff = 2;
        let both = [alpha.clone(), beta.clone()];
        v2.tick(&both);
        assert_eq!(v2.roots().len(), 2);
        let lines = |what: &str| -> Vec<String> {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|line| line.contains(what))
                .cloned()
                .collect()
        };
        let a = test_root(&alpha);
        v2.tick_one(&a, |_| panic!("a test's host panics"));
        assert_eq!(v2.roots(), [test_root(&beta)], "dropped, the other kept");
        // The pause: no rebuild for `panic_backoff` service ticks.
        v2.tick(&both);
        v2.tick(&both);
        assert_eq!(v2.roots().len(), 1, "not rebuilt during the pause");
        // A host that panics on every tick: rebuilt, and panicking again before any tick of it succeeds.
        assert!(v2.ensure_host(&alpha, &a), "rebuilt");
        v2.tick_one(&a, |_| panic!("a test's host panics"));
        assert_eq!(
            lines("panicked").len(),
            1,
            "named once: {:?}",
            lines("panicked")
        );
        assert!(lines("panicked")[0].contains(&a.display().to_string()));
        // After the next pause it is rebuilt, its tick succeeds, and the recovery says how many panics there were.
        for _ in 0..3 {
            v2.tick(&both);
        }
        let recovered = lines("recovered");
        assert_eq!(recovered.len(), 1, "{recovered:?}");
        assert!(recovered[0].contains("2 panics"), "{}", recovered[0]);
    }

    #[test]
    fn the_p99_and_the_longest_tick_are_the_ones_of_the_window() {
        let window = |n: u64| Window {
            service_ticks: n as usize,
            ticks: (1..=n)
                .rev()
                .map(std::time::Duration::from_millis)
                .collect(),
            ..Window::default()
        };
        assert!(
            window(1).line().contains("p99_ms=1.0 max_ms=1.0"),
            "{}",
            window(1).line()
        );
        assert!(
            window(100).line().contains("p99_ms=99.0 max_ms=100.0"),
            "{}",
            window(100).line()
        );
        assert!(
            window(101).line().contains("p99_ms=100.0 max_ms=101.0"),
            "{}",
            window(101).line()
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
