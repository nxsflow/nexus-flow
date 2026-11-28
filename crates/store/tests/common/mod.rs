//! What the tests that open a database share: a scratch folder per test, a data directory in
//! it, a key file for a machine without a key store, and a way to run a future on this thread.

#![allow(dead_code)]

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use nexus_flow_store::data_dir::{DataDir, Platform};
use nexus_flow_store::id::SpaceId;
use nexus_flow_store::model;
use nexus_flow_store::space::DeviceKeys;
use nocentraldb::{Db, OpenOptions};

/// Runs `future` to completion on this thread. A database without a relay does no I/O that
/// needs a runtime.
pub fn block_on<F: Future>(future: F) -> F::Output {
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

/// A fresh folder for one test, holding an empty `repo/`; `data/` beside it is not created.
pub fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-store-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("repo")).unwrap();
    dir.canonicalize().unwrap()
}

/// The data directory at `path`, as `NXS_DATA_DIR` names it.
pub fn data_dir(path: &Path) -> DataDir {
    let path = path.to_path_buf();
    DataDir::resolve(Platform::Linux, move |name| {
        (name == "NXS_DATA_DIR").then(|| path.clone().into_os_string())
    })
    .unwrap()
}

/// The device's keys in a file in the data directory: the test machine has no key store.
pub fn keys(data: &DataDir) -> DeviceKeys {
    DeviceKeys::File(data.root().join("device.keys"))
}

/// Creates a space without the marker that names its kind, as one created before the marker
/// existed, in the database of the device in `data`, and returns its id.
pub fn unmarked_space(data: &DataDir) -> SpaceId {
    let options = OpenOptions::new(model::LOCK)
        .path(data.device())
        .key_file(data.root().join("device.keys"));
    let db = block_on(Db::open(options)).unwrap();
    let space = block_on(db.spaces().create()).unwrap();
    space.id().to_string().parse().unwrap()
}
