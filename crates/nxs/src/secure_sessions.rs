//! **The restart's real hook** (nxf 6j6v.wxz0, section B): before `nxs self-update` or `nxs sync
//! daemon install` boots the background service out, every agent session the service started is put
//! on a `service_restart` hold, stopped, and given a grace — so the new service takes it up again in
//! place instead of the restart killing it with nothing on record (the field case 6j6v.8n9a).
//!
//! `nexus-flow-cli` cannot run this itself — it does not link the chat engine — so the binary
//! injects it ([`nexus_flow_cli::PrepareForRestart`]). The work is
//! [`nexus_chat::restart::secure_the_instance`]; what is decided here is WHICH instance: the one
//! whose heartbeat names a process that is proven to be running, by its process group.

use std::path::PathBuf;
use std::time::Duration;

use nexus_chat::interruption::{
    SERVICE_RESTART_GRACE_SECS, SERVICE_RESTART_MARGIN_SECS, SERVICE_RESTART_POLL_SECS,
};
use nexus_flow_cli::RestartCause;
use nxs_service::restart::RestartPreparation;
use nxs_service::ServiceHome;

/// **The hook the binary hands `self-update` and `daemon install`**: the real grace, poll and
/// clock, and the timer the background service honours (`NXC_TIMER` decides, as for every `nxc`).
///
/// `SIGINT` is ignored while it runs (round-2 IR #9): a Ctrl-C in the middle would leave some
/// sessions held and stopped and the restart not done; the grace is bounded, so the wait is too.
pub fn prepare_for_restart(home: &ServiceHome, cause: &RestartCause) -> RestartPreparation {
    let _ctrl_c = IgnoreInterrupt::now();
    let timer =
        nexus_chat::timer::TimerConfig::from_ambient(|k| std::env::var(k).ok()).unwrap_or_default();
    prepare_with(
        home,
        cause,
        Hook {
            grace_secs: SERVICE_RESTART_GRACE_SECS,
            poll: Duration::from_secs(SERVICE_RESTART_POLL_SECS as u64),
            timer,
        },
    )
}

/// What [`prepare_with`] takes beyond the home and the cause — the grace is injectable so a test
/// does not wait two minutes.
pub(crate) struct Hook {
    pub(crate) grace_secs: i64,
    pub(crate) poll: Duration,
    pub(crate) timer: nexus_chat::timer::TimerConfig,
}

/// [`prepare_for_restart`] with the grace, poll and timer given.
///
/// **Which sessions are this instance's**: the service's pid is read exactly as self-update's
/// ancestor guard reads it — from the heartbeat, and only when that pid is PROVEN to be the
/// service's process (`Running`) — and its process group is the instance. With no such heartbeat
/// there is no instance to work on: nothing is held or stopped, the report says why, and the
/// restart proceeds as before.
pub(crate) fn prepare_with(
    home: &ServiceHome,
    cause: &RestartCause,
    hook: Hook,
) -> RestartPreparation {
    let group = match instance_group(home) {
        Ok(group) => group,
        Err(why) => return RestartPreparation::nothing(why),
    };
    let workspaces: Vec<PathBuf> = match home.attended() {
        Ok(entries) => entries.into_iter().map(|e| PathBuf::from(e.path)).collect(),
        Err(e) => {
            return RestartPreparation::nothing(format!(
                "the workspaces this service attends could not be read ({e})"
            ))
        }
    };
    let detail = cause.to_string();
    nexus_chat::restart::secure_the_instance(nexus_chat::restart::SecureTheInstance {
        workspaces: &workspaces,
        pgid: group,
        detail: &detail,
        grace_secs: hook.grace_secs,
        margin_secs: SERVICE_RESTART_MARGIN_SECS,
        poll: hook.poll,
        timer: hook.timer,
        clock: &time::OffsetDateTime::now_utc,
        sleep: &std::thread::sleep,
        on_poll: &|_| {},
    })
}

/// The process group of the running service, or the sentence saying why there is none.
fn instance_group(home: &ServiceHome) -> Result<i32, String> {
    let heartbeat = nxs_service::heartbeat::read_from(&home.heartbeat())
        .ok()
        .flatten();
    if nxs_service::heartbeat::state(heartbeat.as_ref()) != nxs_service::ServiceState::Running {
        return Err(
            "no running background service was found for this instance, so no session of it is \
             running either"
                .to_string(),
        );
    }
    let pid = heartbeat.map(|hb| hb.pid).unwrap_or_default();
    process_group_of(pid)
}

/// The group of `pid`, through the chat engine's one wrapper of `getpgid`.
fn process_group_of(pid: u32) -> Result<i32, String> {
    nexus_chat::worker::process_group_of(pid).map_err(|errno| match errno {
        Some(errno) => format!(
            "the running service's process group could not be read ({})",
            std::io::Error::from_raw_os_error(errno)
        ),
        None => "the running service's process is gone".to_string(),
    })
}

/// `SIGINT` ignored for as long as this lives, and restored after.
struct IgnoreInterrupt {
    #[cfg(unix)]
    previous: libc::sighandler_t,
}

impl IgnoreInterrupt {
    fn now() -> IgnoreInterrupt {
        #[cfg(unix)]
        {
            // SAFETY: installing SIG_IGN for SIGINT touches no memory of this process; the previous
            // disposition is kept and put back on drop.
            let previous = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
            IgnoreInterrupt { previous }
        }
        #[cfg(not(unix))]
        IgnoreInterrupt {}
    }
}

impl Drop for IgnoreInterrupt {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: restores the disposition read in `now`.
        unsafe {
            libc::signal(libc::SIGINT, self.previous);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// A service home whose heartbeat names THIS process — alive, and stamped with its real start,
    /// so it reads `Running` — and that attends one chat workspace with one minted session.
    fn a_running_service_attending_one_session() -> (tempfile::TempDir, ServiceHome, PathBuf, String)
    {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = ServiceHome::at(tmp.path().join(".nexusflow"));
        std::fs::create_dir_all(home.root()).unwrap();
        let pid = std::process::id();
        let started = nxs_service::heartbeat::recorded_start(pid, std::time::SystemTime::now());
        std::fs::write(
            home.heartbeat(),
            format!(
                r#"{{"pid":{pid},"started_at":"{}","last_pass_at":"{}","workspaces":[]}}"#,
                nxs_service::heartbeat::rfc3339(started),
                nxs_service::heartbeat::rfc3339(std::time::SystemTime::now())
            ),
        )
        .unwrap();
        let root = tmp.path().join("work");
        std::fs::create_dir_all(&root).unwrap();
        nexus_chat::workspace::setup(&root, &nexus_chat::workspace::chat_config()).unwrap();
        use nexus_chat::workspace::ChatWorkspaceExt;
        let mut store = nexus_chat::workspace::Workspace::resolve(None, &root)
            .unwrap()
            .open_chat_store()
            .unwrap();
        store.create_pending_session("m-coder", "coder").unwrap();
        home.register(&root, &root).unwrap();
        (tmp, home, root, "m-coder".to_string())
    }

    fn hook(dir: &std::path::Path) -> Hook {
        std::env::set_var("NXC_TIMER_LOG", dir.join("timer.log"));
        Hook {
            grace_secs: 2,
            poll: Duration::from_millis(100),
            timer: nexus_chat::timer::TimerConfig::Dry,
        }
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// **DoD 5's hook, for real**: the running service's session is held under the restart's limit
    /// — its detail naming `nxs sync daemon install` — and stopped; it goes at the stop and is
    /// reported `held`.
    #[test]
    fn a_session_of_the_running_service_is_held_with_the_cause_and_stopped() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (tmp, home, root, session) = a_running_service_attending_one_session();
        let mut child = Command::new("sleep")
            .arg("600")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        // Reaped as it goes, the way a parent reaps: this test process IS the child's parent, and
        // an unreaped zombie still reads as alive on Linux for the whole grace.
        let pid = child.id();
        let reaper = std::thread::spawn(move || {
            let _ = child.wait();
        });
        let logs = nexus_chat::worker::agent_logs_dir(&root);
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(
            logs.join(format!("{session}.pid")),
            nexus_chat::worker::session_claim_for(pid),
        )
        .unwrap();

        let report = prepare_with(&home, &RestartCause::DaemonInstall, hook(tmp.path()));

        // SAFETY: a signal to the child this test started (already gone if the stop landed).
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        let _ = reaper.join();
        assert_eq!(
            report
                .held
                .iter()
                .map(|p| p.session.as_str())
                .collect::<Vec<_>>(),
            [session.as_str()],
            "{report:?}"
        );
        use nexus_chat::workspace::ChatWorkspaceExt;
        let hold = nexus_chat::workspace::Workspace::resolve(None, &root)
            .unwrap()
            .open_chat_store()
            .unwrap()
            .current_interruption(&session)
            .unwrap()
            .expect("the hold stands");
        assert_eq!(hold.limit, nexus_chat::interruption::SERVICE_RESTART_LIMIT);
        assert_eq!(
            hold.detail,
            "nxs sync daemon install restarted the background service"
        );
    }

    /// **DoD 6, the second half**: no `Running` heartbeat, no instance to work on — the hook holds
    /// and stops nothing, reports empty, and says why.
    #[test]
    fn with_no_running_service_nothing_is_prepared_and_the_report_says_why() {
        let _env = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (tmp, home, _root, _session) = a_running_service_attending_one_session();
        std::fs::remove_file(home.heartbeat()).unwrap();

        let report = prepare_with(&home, &RestartCause::DaemonInstall, hook(tmp.path()));

        assert!(report.is_empty(), "{report:?}");
        assert!(
            report
                .note
                .as_deref()
                .is_some_and(|n| n.contains("no running")),
            "{report:?}"
        );
    }
}
