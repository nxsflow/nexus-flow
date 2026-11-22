//! The store opens a member database from the committed model lock (RFC 0001, "The model"):
//! the lock is compiled in, so building needs neither Node nor `ncdb`.

use std::future::Future;
use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use nexus_flow_store::model;
use nocentraldb::{Db, OpenOptions, Reference, Value};

/// Runs `future` to completion on this thread. A database without a relay does no I/O that
/// needs a runtime, so the store's tests need none.
fn block_on<F: Future>(future: F) -> F::Output {
    struct Unpark(Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

/// A fresh, empty data directory for one test, as `NXS_DATA_DIR` names one (RFC 0001, "The
/// local data on disk").
fn data_dir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-store-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_database_opens_from_the_committed_lock_under_the_data_dir() {
    let dir = data_dir("model-open");
    let device = dir.join("device");
    let options = OpenOptions::new(model::LOCK)
        .path(&device)
        .key_file(dir.join("device.keys"));
    let db = block_on(Db::open(options)).expect("the committed lock opens a database");
    drop(db);
    assert!(
        device.is_dir(),
        "the database lives under the data directory"
    );

    let options = OpenOptions::new(model::LOCK)
        .path(&device)
        .key_file(dir.join("device.keys"));
    block_on(Db::open(options)).expect("the database opens again with the same lock");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_role_record_is_written_to_space_member_and_read_back() {
    let db = block_on(Db::open(OpenOptions::new(model::LOCK).in_memory())).unwrap();
    let space = block_on(db.spaces().create()).unwrap();
    let written = block_on(space.create(
        "spaceMember",
        [
            ("member", Value::from("m1")),
            ("role", Value::from("coder")),
        ],
    ))
    .expect("spaceMember is a collection of the model");
    let id = written.ids[0];
    let record = block_on(space.get("spaceMember", id)).unwrap().unwrap();
    assert_eq!(record.get::<&str>("member").unwrap(), Some("m1"));
    assert_eq!(record.get::<&str>("role").unwrap(), Some("coder"));
    assert_eq!(
        record.get::<Reference>("ticket").unwrap(),
        None,
        "a field left unset"
    );
}
