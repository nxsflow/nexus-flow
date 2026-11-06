// `kill(2)` and `waitpid(2)`, the same on both platforms; included by each, since a `#[path]` module has no sibling.

fn number(signal: Signal) -> libc::c_int {
    match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
        Signal::Usr2 => libc::SIGUSR2,
    }
}

pub(super) fn kill(target: i32, signal: Signal) -> bool {
    // SAFETY: `kill` takes two integers and touches no caller memory. The caller has refused 0, -1 and 1.
    unsafe { libc::kill(target, number(signal)) == 0 }
}

/// Only `ESRCH` is gone. `EPERM` is not: on Darwin it is what a group of an unreaped zombie answers (§8.3).
pub(super) fn group_is_gone(pgid: u32) -> bool {
    // SAFETY: as `kill`; signal 0 sends nothing. The caller has refused a pgid <= 1.
    let answer = unsafe { libc::kill(-(pgid as i32), 0) };
    answer != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

pub(super) fn reap(pid: u32) {
    let mut status: libc::c_int = 0;
    // SAFETY: an owned status word that outlives the call; WNOHANG makes it return at once. A pid that is not our
    // child answers ECHILD and nothing happens.
    unsafe { libc::waitpid(pid as i32, &raw mut status, libc::WNOHANG) };
}
