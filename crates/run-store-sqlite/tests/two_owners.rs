//! Two owners on one store (§12.5, DoD 10): only the lease holder appends, the other's append is
//! `Fenced`, a takeover comes within one expiry with a higher epoch, the stale holder's next append is
//! `Fenced`, and the run completes once.
//!
//! Time is an injected clock throughout, so nothing here depends on how loaded the machine is. A `Busy`
//! is retried, as the host's next tick would. The process variant re-runs this test binary as the second
//! host, and drives both hosts line by line over the child's stdin.

mod fixtures;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Barrier;

use nexus_run_contracts::{EventBody, RunId, RunStore, StoreError};
use nexus_run_store_sqlite::{Lease, SqliteRunStore};

const EXPIRY_MS: i64 = 30_000;
const STORE_PATH_VAR: &str = "NXR_TWO_OWNERS_STORE";

fn retry_busy<T>(mut call: impl FnMut() -> Result<T, StoreError>) -> Result<T, StoreError> {
    loop {
        match call() {
            Err(StoreError::Busy) => std::thread::yield_now(),
            other => return other,
        }
    }
}

struct StopsOnDrop<'a>(&'a AtomicBool);

impl Drop for StopsOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn commission_with_inputs(store: &SqliteRunStore, run: &RunId, inputs: usize) -> Vec<String> {
    store
        .enqueue(run, &fixtures::commissioned(run), &[])
        .unwrap();
    let mut keys = vec![fixtures::commissioned(run).key];
    for n in 0..inputs {
        let key = format!("input/{n}");
        store
            .enqueue(run, &fixtures::input(run, &key), &[])
            .unwrap();
        keys.push(key);
    }
    keys
}

/// Moves one input into the log under `lease`, as the owner's tick does. `Ok(None)` is an empty inbox.
fn step(store: &SqliteRunStore, run: &RunId, lease: &Lease) -> Result<Option<String>, StoreError> {
    let Some(input) = store.next_input(run)? else {
        return Ok(None);
    };
    let last_seq = store.load(run, 0)?.len() as u64;
    store.append(
        run,
        last_seq,
        lease.epoch,
        std::slice::from_ref(&input),
        std::slice::from_ref(&input.key),
        &[],
    )?;
    Ok(Some(input.key))
}

fn count_finished(store: &SqliteRunStore, run: &RunId) -> usize {
    let events = store.load(run, 0).unwrap();
    events
        .iter()
        .filter(|event| {
            matches!(
                event.body,
                EventBody::Decision(nexus_run_contracts::Decision::Finished { .. })
            )
        })
        .count()
}

fn assert_completed_once(store: &SqliteRunStore, run: &RunId, inputs: &[String]) {
    let events = store.load(run, 0).unwrap();
    let keys: Vec<_> = events.iter().map(|event| event.key.as_str()).collect();
    let mut expected: Vec<_> = inputs.iter().map(String::as_str).collect();
    expected.push("end");
    assert_eq!(keys, expected, "every input once, in order, then one end");
    let seqs: Vec<_> = events.iter().map(|event| event.seq).collect();
    let gap_free: Vec<_> = (1..=events.len() as u64).collect();
    assert_eq!(seqs, gap_free);
    assert_eq!(count_finished(store, run), 1);
    assert!(store.open_runs().unwrap().is_empty(), "the run is closed");
    assert_eq!(store.next_input(run).unwrap(), None);
}

#[test]
fn two_threads_only_the_holder_appends_and_the_other_takes_over_after_it_stops_renewing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let host_a = fixtures::open(&path);
    let host_b = fixtures::open(&path);
    let run = fixtures::run_id();
    let inputs = commission_with_inputs(&host_a, &run, 12);
    let clock = AtomicI64::new(0);
    let a_lease = host_a
        .acquire_or_renew(&run, "host-a", fixtures::at(0))
        .unwrap();
    assert_eq!(a_lease.epoch, 1);
    let a_stopped = AtomicBool::new(false);

    let (a_appended, a_last, b_appended, b_lease) = std::thread::scope(|scope| {
        let holder = scope.spawn(|| {
            // Keeps stepping, and renewing across half an expiry, then stops renewing halfway through.
            // The guard says so on a panic too, or the other host would wait for it forever.
            let _stops = StopsOnDrop(&a_stopped);
            let mut appended = Vec::new();
            let mut lease = a_lease.clone();
            for _ in 0..6 {
                clock.fetch_add(EXPIRY_MS / 3, Ordering::SeqCst);
                let now = fixtures::at(clock.load(Ordering::SeqCst));
                lease = retry_busy(|| host_a.acquire_or_renew(&run, "host-a", now)).unwrap();
                assert_eq!(lease.epoch, 1, "a renewal keeps the epoch");
                let key = retry_busy(|| step(&host_a, &run, &lease)).unwrap().unwrap();
                appended.push(key);
            }
            (appended, lease)
        });
        let other = scope.spawn(|| {
            let mut appended = Vec::new();
            // While the holder lives, this host is refused the lease, and a fence it guesses is refused.
            let lease = loop {
                let now = fixtures::at(clock.load(Ordering::SeqCst));
                match retry_busy(|| host_b.acquire_or_renew(&run, "host-b", now)) {
                    Ok(lease) => break lease,
                    Err(StoreError::CasFailed { .. }) => {}
                    Err(other) => panic!("{other:?}"),
                }
                let guess = fixtures::input(&run, "a-guessed-fence");
                let fenced = retry_busy(|| {
                    host_b.append(&run, 0, 0, std::slice::from_ref(&guess), &[], &[])
                });
                assert_eq!(fenced.unwrap_err(), StoreError::Fenced);
                if a_stopped.load(Ordering::SeqCst) {
                    // The holder is gone and has stopped moving the clock, so it moves past the last
                    // renewal by exactly one expiry, and not a millisecond more.
                    let stored = host_b.get(&format!("owner/{run}")).unwrap().unwrap();
                    let last: Lease = serde_json::from_slice(&stored).unwrap();
                    let expires = last.expires_at.unix_millis() - fixtures::t0().unix_millis();
                    clock.store(expires, Ordering::SeqCst);
                }
                std::thread::yield_now();
            };
            while let Some(key) = retry_busy(|| step(&host_b, &run, &lease)).unwrap() {
                appended.push(key);
            }
            (appended, lease)
        });
        let (a_appended, a_last) = holder.join().unwrap();
        let (b_appended, b_lease) = other.join().unwrap();
        (a_appended, a_last, b_appended, b_lease)
    });

    assert_eq!(b_lease.epoch, a_last.epoch + 1, "a higher epoch");
    assert_eq!(
        b_lease.expires_at,
        fixtures::at(a_last.expires_at.unix_millis() - fixtures::t0().unix_millis() + EXPIRY_MS),
        "taken at the instant the old lease expired"
    );
    assert_eq!(a_appended.len(), 6, "every step of the holder landed");
    assert_eq!(a_appended.len() + b_appended.len(), inputs.len());

    // The stale holder wakes up and is refused, with nothing written.
    let late = fixtures::input(&run, "late");
    let seq = host_a.load(&run, 0).unwrap().len() as u64;
    assert_eq!(
        host_a
            .append(
                &run,
                seq,
                a_last.epoch,
                std::slice::from_ref(&late),
                &[],
                &[]
            )
            .unwrap_err(),
        StoreError::Fenced
    );

    host_b
        .append(
            &run,
            seq,
            b_lease.epoch,
            &[fixtures::finished(&run, "end")],
            &[],
            &[],
        )
        .unwrap();
    assert_completed_once(&host_b, &run, &inputs);
}

#[test]
fn two_threads_racing_for_a_new_lease_leave_exactly_one_holder_at_epoch_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let (a, b) = (fixtures::open(&path), fixtures::open(&path));
    for round in 0..40 {
        let run = RunId::parse(&format!("r-01JAAAAAAAAAAAAAAAAAAAAA{round:02}")).unwrap();
        let barrier = Barrier::new(2);
        let race = |store: &SqliteRunStore, owner: &str| {
            barrier.wait();
            retry_busy(|| store.acquire_or_renew(&run, owner, fixtures::t0()))
        };
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| race(&a, "host-a"));
            let second = scope.spawn(|| race(&b, "host-b"));
            (first.join().unwrap(), second.join().unwrap())
        });
        let winner = match (first, second) {
            (Ok(lease), Err(StoreError::CasFailed { .. }))
            | (Err(StoreError::CasFailed { .. }), Ok(lease)) => lease,
            other => panic!("not exactly one winner: {other:?}"),
        };
        assert_eq!(winner.epoch, 1);
    }
}

#[test]
fn two_threads_racing_one_sequence_number_land_exactly_one_append() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let (a, b) = (fixtures::open(&path), fixtures::open(&path));
    let run = fixtures::run_id();
    let fence = fixtures::owned_run(&a, &run);
    for round in 1..=40_u64 {
        let barrier = Barrier::new(2);
        let race = |store: &SqliteRunStore, key: String| {
            barrier.wait();
            let event = fixtures::input(&run, &key);
            retry_busy(|| store.append(&run, round, fence, std::slice::from_ref(&event), &[], &[]))
        };
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| race(&a, format!("a/{round}")));
            let second = scope.spawn(|| race(&b, format!("b/{round}")));
            (first.join().unwrap(), second.join().unwrap())
        });
        match (first, second) {
            (Ok(appended), Err(StoreError::SeqMismatch))
            | (Err(StoreError::SeqMismatch), Ok(appended)) => {
                assert_eq!(appended.last_seq, round + 1);
            }
            other => panic!("not exactly one append: {other:?}"),
        }
    }
    assert_eq!(a.load(&run, 0).unwrap().len(), 41);
}

/// The second host: not a test of its own. `two_processes_...` runs it with the store's path in the
/// environment and speaks to it over stdin, one command per line, one `reply:` line back.
#[test]
#[ignore = "the child of two_processes_only_the_holder_appends_and_the_run_completes_once"]
fn second_host_process() {
    // `--ignored` run by hand: there is no parent to speak to.
    let Ok(path) = std::env::var(STORE_PATH_VAR) else {
        return;
    };
    let store = fixtures::open(Path::new(&path));
    let run = fixtures::run_id();
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let words: Vec<&str> = line.split_whitespace().collect();
        let reply = match words[..] {
            ["acquire", now] => {
                let now = fixtures::at(now.parse().unwrap());
                match retry_busy(|| store.acquire_or_renew(&run, "host-b", now)) {
                    Ok(lease) => format!("epoch {}", lease.epoch),
                    Err(err) => err.kind().to_owned(),
                }
            }
            ["append", key, fence] => {
                let fence: u64 = fence.parse().unwrap();
                let event = fixtures::input(&run, key);
                let last_seq = store.load(&run, 0).unwrap().len() as u64;
                let consume = [key.to_owned()];
                let appended = retry_busy(|| {
                    store.append(
                        &run,
                        last_seq,
                        fence,
                        std::slice::from_ref(&event),
                        &consume,
                        &[],
                    )
                });
                match appended {
                    Ok(appended) => format!("seq {}", appended.last_seq),
                    Err(err) => err.kind().to_owned(),
                }
            }
            ["finish", fence] => {
                let fence: u64 = fence.parse().unwrap();
                let last_seq = store.load(&run, 0).unwrap().len() as u64;
                let end = fixtures::finished(&run, "end");
                let appended = retry_busy(|| {
                    store.append(&run, last_seq, fence, std::slice::from_ref(&end), &[], &[])
                });
                match appended {
                    Ok(appended) => format!("seq {}", appended.last_seq),
                    Err(err) => err.kind().to_owned(),
                }
            }
            ["quit"] => break,
            _ => panic!("no such command: {line}"),
        };
        println!("reply: {reply}");
    }
}

struct SecondHost {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl SecondHost {
    fn spawn(store: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "second_host_process", "--ignored", "--nocapture"])
            .env(STORE_PATH_VAR, store)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn ask(&mut self, command: &str) -> String {
        writeln!(self.stdin, "{command}").unwrap();
        self.stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).unwrap();
            assert_ne!(read, 0, "the second host died on: {command}");
            if let Some((_, reply)) = line.trim().split_once("reply: ") {
                return reply.to_owned();
            }
        }
    }

    fn quit(mut self) {
        writeln!(self.stdin, "quit").unwrap();
        self.stdin.flush().unwrap();
        drop(self.stdin);
        assert!(
            self.child.wait().unwrap().success(),
            "the second host failed"
        );
    }
}

#[test]
fn two_processes_only_the_holder_appends_and_the_run_completes_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.sqlite");
    let host_a = fixtures::open(&path);
    let run = fixtures::run_id();
    let inputs = commission_with_inputs(&host_a, &run, 2);
    let mut host_b = SecondHost::spawn(&path);

    let a_lease = host_a
        .acquire_or_renew(&run, "host-a", fixtures::at(0))
        .unwrap();
    assert_eq!(host_b.ask("acquire 1000"), "cas_failed", "a live lease");
    assert_eq!(host_b.ask("append guess 0"), "fenced");
    assert_eq!(host_b.ask("append guess 2"), "fenced");

    assert_eq!(
        step(&host_a, &run, &a_lease).unwrap(),
        Some(inputs[0].clone())
    );
    assert_eq!(
        step(&host_a, &run, &a_lease).unwrap(),
        Some(inputs[1].clone())
    );

    // The holder stops renewing. Not a millisecond before the expiry, and at it.
    assert_eq!(
        host_b.ask(&format!("acquire {}", EXPIRY_MS - 1)),
        "cas_failed"
    );
    assert_eq!(host_b.ask(&format!("acquire {EXPIRY_MS}")), "epoch 2");

    // The stale holder's next append is fenced, and it cannot get the lease back while it is live.
    let waiting = store_next_key(&host_a, &run);
    assert_eq!(
        step(&host_a, &run, &a_lease).unwrap_err(),
        StoreError::Fenced
    );
    assert_eq!(store_next_key(&host_a, &run), waiting, "its input stays");
    assert_eq!(
        host_a
            .acquire_or_renew(&run, "host-a", fixtures::at(EXPIRY_MS + 1))
            .unwrap_err()
            .kind(),
        "cas_failed"
    );

    let last_input = inputs[2].clone();
    assert_eq!(host_b.ask(&format!("append {last_input} 2")), "seq 3");
    assert_eq!(host_b.ask("finish 2"), "seq 4");
    host_b.quit();

    assert_completed_once(&host_a, &run, &inputs);
}

fn store_next_key(store: &SqliteRunStore, run: &RunId) -> String {
    store.next_input(run).unwrap().unwrap().key
}
