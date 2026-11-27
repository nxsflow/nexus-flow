//! The members' tests (RFC 0001, "Members", "The role record", "Roles in a space"). They are unit
//! tests, not integration tests, so they have `crate::testing` without the feature
//! `test-support`, which an integration test could only enable through a dependency on the store
//! itself.

mod add;
mod list;
mod remove;
mod roles;
mod tags;

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use crate::data_dir::{DataDir, Platform};
use crate::space::DeviceKeys;

/// Runs `future` to completion on this thread. A database without a relay does no I/O that
/// needs a runtime.
pub(super) fn block_on<F: Future>(future: F) -> F::Output {
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
pub(super) fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-store-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("repo")).unwrap();
    dir.canonicalize().unwrap()
}

/// The data directory at `path`, as `NXS_DATA_DIR` names it.
pub(super) fn data_dir(path: &Path) -> DataDir {
    let path = path.to_path_buf();
    DataDir::resolve(Platform::Linux, move |name| {
        (name == "NXS_DATA_DIR").then(|| path.clone().into_os_string())
    })
    .unwrap()
}

/// The device's keys in a file in the data directory: the test machine has no key store.
pub(super) fn keys(data: &DataDir) -> DeviceKeys {
    DeviceKeys::File(data.root().join("device.keys"))
}
