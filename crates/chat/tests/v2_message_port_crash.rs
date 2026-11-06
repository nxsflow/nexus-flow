//! The crash test over the REAL chat store (§12.4, round 3 TQ #5), because the layering gate keeps the real port
//! out of the crash box: post, die before the `done`, restart, find; open the same thread id twice and read one
//! thread.
//!
//! The port here is the `MessagePort` contract over `ChatStore`'s `try_` forms, kept in this test because the old
//! side does not import the host yet: `ports_v1` is ticket b7ck's, and with it `Refs` gains `run_key`. Until then
//! the key rides in `Refs.pr`, the same JSON refs column, so what this proves about a post that survives a killed
//! process and is found by a key in its refs holds for that field too.

use std::process::{Command, Stdio};
use std::sync::Mutex;

use nexus_chat::model::{Disposition, MessageEnvelope, MessageKind, Priority, Refs, ThreadRoot};
use nexus_chat::store::ChatStore;
use nexus_run_contracts::MessageId;
use nexus_run_host::ports::{self, Message, MessagePort, PortError, PortResult};

const SITE: i64 = 1;
const STORE_VAR: &str = "NXR_PORT_CRASH_STORE";
const THREAD_VAR: &str = "NXR_PORT_CRASH_THREAD";
const KEY: &str = "result/r-01JAAAAAAAAAAAAAAAAAAAAAAA";
const CHANNEL: &str = "coding-v2";
const OPENER: &str = "__channel__";

fn refused(error: impl std::fmt::Display) -> PortError {
    PortError {
        what: error.to_string(),
    }
}

/// `MessagePort` over the chat store.
struct ChatPort(Mutex<ChatStore>);

impl ChatPort {
    fn open(path: &str) -> Self {
        Self(Mutex::new(
            ChatStore::open(path, SITE).expect("the chat store opens"),
        ))
    }

    fn mint_thread_id(&self) -> String {
        self.0.lock().unwrap().mint_thread_id()
    }

    fn posts_in(&self, thread: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .messages_in_thread(thread)
            .unwrap()
            .len()
    }

    fn opened_threads(&self) -> Vec<String> {
        self.0.lock().unwrap().threads_opened_by(OPENER).unwrap()
    }
}

impl MessagePort for ChatPort {
    fn post(
        &self,
        thread: &str,
        sender: &str,
        body: &str,
        refs: &ports::Refs,
        key: &str,
    ) -> PortResult<MessageId> {
        let envelope = MessageEnvelope {
            origin: "local".to_owned(),
            channel_id: CHANNEL.to_owned(),
            sender: sender.to_owned(),
            kind: MessageKind::Task,
            priority: Priority::Normal,
            disposition: Disposition::InTurn,
            thread_id: Some(thread.to_owned()),
            refs: Refs {
                nxf_ids: refs.nxf_ids.clone(),
                pr: Some(key.to_owned()),
                ..Refs::default()
            },
            body: body.to_owned(),
        };
        self.0
            .lock()
            .unwrap()
            .try_post_message(&envelope)
            .map_err(refused)
    }

    fn find(&self, thread: &str, key: &str) -> PortResult<Option<MessageId>> {
        let rows = self
            .0
            .lock()
            .unwrap()
            .messages_in_thread(thread)
            .map_err(refused)?;
        Ok(rows.into_iter().find_map(|row| {
            let refs: Refs = serde_json::from_str(row.refs.as_deref()?).ok()?;
            (refs.pr.as_deref() == Some(key)).then_some(row.message_id)
        }))
    }

    fn read(&self, message: &str) -> PortResult<Option<Message>> {
        let store = self.0.lock().unwrap();
        let Some((_, Some(thread))) = store.message_channel(message) else {
            return Ok(None);
        };
        let rows = store.messages_in_thread(&thread).map_err(refused)?;
        Ok(rows.into_iter().find_map(|row| {
            (row.message_id == message).then(|| Message {
                sender: row.sender,
                body: row.body,
                refs: ports::Refs::default(),
            })
        }))
    }

    fn open_thread(
        &self,
        thread_id: &str,
        parent: Option<&str>,
        channel: &str,
        opener: &str,
    ) -> PortResult<()> {
        let root = ThreadRoot {
            origin: "local".to_owned(),
            channel_id: channel.to_owned(),
            opener: opener.to_owned(),
            created: String::new(),
            parent: parent.map(str::to_owned),
        };
        self.0
            .lock()
            .unwrap()
            .try_open_thread(thread_id, &root, opener)
            .map_err(refused)
    }
}

/// The host's find-or-post, as `Deliver` runs it (§8.3).
fn find_or_post(port: &ChatPort, thread: &str) -> MessageId {
    match port.find(thread, KEY).expect("find answers") {
        Some(found) => found,
        None => port
            .post(thread, OPENER, "the result", &ports::Refs::default(), KEY)
            .expect("the post goes through"),
    }
}

/// The child of the test below: it posts and dies before it can record `done`, with no unwinding and no flush.
fn die_after_the_post(store: &str, thread: &str) -> ! {
    let port = ChatPort::open(store);
    port.open_thread(thread, None, CHANNEL, OPENER).unwrap();
    find_or_post(&port, thread);
    std::process::abort();
}

#[test]
fn a_post_survives_a_kill_before_done_and_is_found_again_never_posted_twice() {
    if let (Ok(store), Ok(thread)) = (std::env::var(STORE_VAR), std::env::var(THREAD_VAR)) {
        die_after_the_post(&store, &thread);
    }
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("db.sqlite");
    let store = store.to_str().unwrap();
    let thread = ChatPort::open(store).mint_thread_id();

    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_post_survives_a_kill_before_done_and_is_found_again_never_posted_twice",
            "--nocapture",
        ])
        .env(STORE_VAR, store)
        .env(THREAD_VAR, &thread)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(
        !status.success() && status.code().is_none(),
        "the child aborted: {status:?}"
    );

    // The restart: `pending` is reconciled by `find` first.
    let port = ChatPort::open(store);
    let found = port.find(&thread, KEY).unwrap();
    let found = found.expect("the post survived the kill, and its key with it");
    assert_eq!(port.posts_in(&thread), 1);
    assert_eq!(
        find_or_post(&port, &thread),
        found,
        "found, not posted again"
    );
    assert_eq!(port.posts_in(&thread), 1);
    let read = port.read(&found).unwrap().expect("the message reads by id");
    assert_eq!(read.body, "the result");
    assert_eq!(read.sender, OPENER);
    assert_eq!(port.find(&thread, "another/key").unwrap(), None);
}

/// A thread is found by its id: a second open of the same id, across a restart as well, is the same thread.
#[test]
fn opening_the_same_thread_id_twice_holds_one_thread() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("db.sqlite");
    let store = store.to_str().unwrap();
    let port = ChatPort::open(store);
    let thread = port.mint_thread_id();
    port.open_thread(&thread, None, CHANNEL, OPENER).unwrap();
    port.open_thread(&thread, None, CHANNEL, OPENER).unwrap();
    drop(port);

    let port = ChatPort::open(store);
    port.open_thread(&thread, None, CHANNEL, OPENER).unwrap();
    assert_eq!(
        port.opened_threads(),
        std::slice::from_ref(&thread),
        "one thread, opened three times"
    );
    assert_eq!(
        port.0.lock().unwrap().thread_channel(&thread).as_deref(),
        Some(CHANNEL)
    );
}
