//! **A chat runs on the machine it is designated to — the real binary, two machines, a real relay**
//! (nxf 6j6v.1c6k).
//!
//! Two service homes are two machines (6j6v.f0b5). Each holds a workspace bound to one stream on a
//! real relay (the `nxf-relay` router, served in-process on a loopback socket with a clock the test
//! moves), each trusts the other's key, and each runs its real background service. A chat the laptop
//! starts FOR the studio starts nothing on the laptop; the studio's service peeks, pulls, and starts
//! the persona there. A reply into that chat after the studio has gone asks instead of posting.
//!
//! The worker is the dry one (`NXC_WORKER=dry`): a start is a line in a log, which is what shows on
//! WHICH machine the persona was started.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nxs_test_support::PinHome;
use serde_json::Value;
use tempfile::TempDir;

const STREAM: &str = "stream-executing-machine-e2e";

fn serve(router: axum::Router) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, router).await.unwrap();
        });
    });
    format!("http://{addr}")
}

fn relay_with_clock() -> (String, Arc<AtomicI64>) {
    let now = Arc::new(AtomicI64::new(1_000_000));
    let clock_now = Arc::clone(&now);
    let router = nxs_server::app::app_with_clock(
        Arc::new(nxs_server::store::SqliteOpStore::open_in_memory().unwrap()),
        Arc::new(nxs_server::registry::SqlitePrefixRegistry::open_in_memory().unwrap()),
        Arc::new(move || clock_now.load(Ordering::SeqCst)),
    );
    (serve(router), now)
}

struct Machine {
    home: TempDir,
    ws: PathBuf,
    _ws_dir: TempDir,
}

impl Machine {
    fn dry_log(&self) -> PathBuf {
        self.home.path().join("starts.log")
    }

    fn starts(&self) -> String {
        std::fs::read_to_string(self.dry_log()).unwrap_or_default()
    }

    fn nxs(&self) -> Command {
        nxs_test_support::assert_multicall_binary_fresh();
        let mut c = Command::new(assert_cmd::cargo::cargo_bin("nxs"));
        c.current_dir(&self.ws)
            .pin_home(self.home.path())
            .env("NXC_WORKER", "dry")
            .env("NXC_DRY_LOG", self.dry_log())
            .env("NXC_TIMER", "dry")
            .env("NXC_ACTOR", "alice")
            .env_remove("NXC_SESSION");
        c
    }

    fn json(&self, args: &[&str]) -> Value {
        let out = self.nxs().args(args).output().unwrap();
        assert!(
            out.status.success(),
            "nxs {args:?}: exit {:?}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
    }
}

fn machine(relay: &str, name: &str) -> Machine {
    let home = TempDir::new().unwrap();
    let ws_dir = TempDir::new().unwrap();
    let ws = std::fs::canonicalize(ws_dir.path()).unwrap();
    let m = Machine {
        home,
        ws,
        _ws_dir: ws_dir,
    };
    m.json(&["init", "--module", "chat", "--json"]);
    std::fs::create_dir_all(m.ws.join(".nxs-personas")).unwrap();
    std::fs::write(
        m.ws.join(".nxs-personas/coder.yaml"),
        "handle: coder\njob_title: Coder\njob_description: Implements a work order.\n\
         system_prompt: You are coder.\n",
    )
    .unwrap();
    m.json(&[
        "sync",
        "bind",
        "--join",
        STREAM,
        "--endpoint",
        relay,
        "--no-daemon",
        "--json",
    ]);
    m.json(&["sync", "machine", name, "--json"]);
    m
}

struct Service {
    child: Child,
    log: PathBuf,
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Service {
    fn log(&self) -> String {
        format!(
            "--- {} ---\n{}",
            self.log.display(),
            std::fs::read_to_string(&self.log).unwrap_or_default()
        )
    }
}

fn start_service(m: &Machine, interval_secs: u64) -> Service {
    let log = m.home.path().join("service.log");
    let out = std::fs::File::create(&log).unwrap();
    let err = out.try_clone().unwrap();
    let mut cmd = m.nxs();
    cmd.current_dir(m.home.path());
    Service {
        child: cmd
            .args(["sync", "daemon", "--interval", &interval_secs.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(err))
            .spawn()
            .expect("the service starts"),
        log,
    }
}

fn wait_for(secs: u64, what: &str, services: &[&Service], mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let logs: Vec<String> = services.iter().map(|s| s.log()).collect();
    assert!(f(), "timed out waiting for {what}\n{}", logs.join("\n"));
}

fn online_names(m: &Machine) -> Vec<String> {
    let reading = m.json(&["sync", "machines", "--json"]);
    let mut names: Vec<String> = reading["machines"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|m| m["online"] == true)
                .map(|m| m["name"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn trust_each_other(a: &Machine, b: &Machine) {
    for (from, to, name) in [(a, b, "a"), (b, a, "b")] {
        let key = from.json(&["sync", "key", "--json"]);
        let id = key["key_id"]
            .as_str()
            .unwrap_or_else(|| panic!("{key}"))
            .to_string();
        to.json(&["sync", "trust", "add", &id, "--name", name, "--json"]);
    }
}

#[test]
fn a_chat_sent_to_another_machine_starts_there_and_only_there() {
    let (relay, now) = relay_with_clock();
    let laptop = machine(&relay, "laptop");
    let studio = machine(&relay, "studio");
    trust_each_other(&laptop, &studio);
    // The laptop passes every 2 s, so it keeps announcing itself after the relay's clock moves
    // below. The studio keeps the production 300 s safety net: what brings the chat to it within
    // seconds is the peek, not its interval.
    let laptop_service = start_service(&laptop, 2);
    let studio_service = start_service(&studio, 300);
    wait_for(
        60,
        "both machines to announce themselves",
        &[&laptop_service, &studio_service],
        || online_names(&laptop) == ["laptop", "studio"],
    );

    // The question, as data, before anything is sent.
    let asked = laptop.json(&[
        "chat",
        "machine",
        "--to",
        "coder",
        "--machine",
        "studio",
        "--json",
    ]);
    assert_eq!(asked["must_ask"], false, "{asked}");
    assert_eq!(asked["machine"]["name"], "studio", "{asked}");
    assert_eq!(asked["machine"]["here"], false, "{asked}");

    let sent = laptop.json(&[
        "chat",
        "send",
        "--to",
        "coder",
        "--machine",
        "studio",
        "--no-ref",
        "--json",
        "Add a --since flag to the export command.",
    ]);
    assert_eq!(sent["spawned"], false, "{sent}");
    assert_eq!(sent["handed_to"]["name"], "studio", "{sent}");
    assert!(
        sent.get("session").is_none(),
        "no session on the laptop: {sent}"
    );
    let thread = sent["thread_id"].as_str().unwrap().to_string();

    let handed_at = Instant::now();
    wait_for(
        90,
        "the studio's service to pick the chat up",
        &[&laptop_service, &studio_service],
        || studio.starts().contains("Add a --since flag"),
    );
    assert!(
        handed_at.elapsed() < Duration::from_secs(60),
        "within the peek, not the 300 s interval: {:?}",
        handed_at.elapsed()
    );
    assert!(
        laptop.starts().is_empty(),
        "nothing started on the laptop: {}",
        laptop.starts()
    );
    assert!(
        studio_service.log().contains("picking it up"),
        "{}",
        studio_service.log()
    );
    // Once, not once per pass.
    std::thread::sleep(Duration::from_secs(20));
    assert_eq!(
        studio.starts().matches("Add a --since flag").count(),
        1,
        "{}",
        studio.starts()
    );

    // The chat, read back on the laptop: it runs on the studio, because the chat says so.
    let on = laptop.json(&["chat", "machine", "--thread", &thread, "--json"]);
    assert_eq!(on["machine"]["source"], "chat", "{on}");
    assert_eq!(on["machine"]["name"], "studio", "{on}");

    // The studio goes away. On the relay's clock a whole window passes; the laptop's next pass
    // stamps itself with the moved clock, the studio's last sighting ages out.
    drop(studio_service);
    now.fetch_add(2 * 300 + 60 + 1, Ordering::SeqCst);
    wait_for(
        60,
        "the studio to read as not online",
        &[&laptop_service],
        || online_names(&laptop) == ["laptop"],
    );
    let before = laptop.json(&["chat", "threads", "show", &thread, "--json"]);
    let out = laptop
        .nxs()
        .args(["chat", "reply", "--thread", &thread, "Are you still there?"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "a reply to an absent machine asks");
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("not online"), "{said}");
    assert!(
        said.contains("laptop ("),
        "offers the machines that are online: {said}"
    );
    let after = laptop.json(&["chat", "threads", "show", &thread, "--json"]);
    assert_eq!(
        before["messages"].as_array().map(Vec::len),
        after["messages"].as_array().map(Vec::len),
        "asking posts nothing"
    );

    // Answering the question: hand the chat to the laptop. It starts HERE now, with the
    // conversation so far.
    let handed = laptop.json(&[
        "chat",
        "reply",
        "--thread",
        &thread,
        "--machine",
        "laptop",
        "--json",
        "Carry on here, please.",
    ]);
    assert!(handed.get("handed_to").is_none(), "{handed}");
    assert!(
        laptop.starts().contains("Carry on here"),
        "{}",
        laptop.starts()
    );
    assert!(
        laptop.starts().contains("Add a --since flag"),
        "the persona nobody answered for yet is handed the whole conversation: {}",
        laptop.starts()
    );
}
