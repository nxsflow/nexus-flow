//! The members' tests (RFC 0001, "Members", "The role record", "Roles in a space"). They are unit
//! tests, not integration tests, so they have `crate::testing` without the feature
//! `test-support`, which an integration test could only enable through a dependency on the store
//! itself.

mod add;
mod list;
mod owner;
mod remove;
mod retag;
mod role_file;
mod roles;
mod tagged;
mod tags;

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use crate::data_dir::{DataDir, Platform};
use crate::members::{self, Member, MemberError};
use crate::opened::{self, Opened};
use crate::space::{init, DeviceKeys};
use crate::testing;

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

/// The tag the device holds in every [`Repo`]: `init` picks the device's tag by its random id,
/// so the tests give it one that no test names (NEX-248).
pub(super) const DEVICE_TAG: &str = "zy";

/// A repository with its space, the device's data directory and the space's id.
pub(super) struct Repo {
    pub base: PathBuf,
    pub data: DataDir,
    pub space: String,
}

impl Repo {
    pub fn new(test: &str) -> Repo {
        let base = scratch(test);
        let data = data_dir(&base.join("data"));
        let space = block_on(init(&base.join("repo"), &data, keys(&data)))
            .unwrap()
            .space;
        role_files(&base.join("repo"), &["pm", "coder"]);
        Repo { base, data, space }
    }

    pub fn repo(&self) -> PathBuf {
        self.base.join("repo")
    }

    /// The repository's spaces, open as the device's member.
    pub fn opened(&self) -> Opened {
        block_on(opened::repository(
            &self.repo(),
            &self.data,
            keys(&self.data),
        ))
        .unwrap()
    }

    /// The invitation of a new agent paired into the device's account, in `name/`.
    pub fn agent(&self, name: &str) -> String {
        block_on(testing::paired_agent(
            &self.data,
            keys(&self.data),
            &self.base.join(name),
        ))
    }

    pub fn list(&self) -> Vec<Member> {
        block_on(members::list(&self.opened())).unwrap()
    }

    pub fn add(
        &self,
        invitation: &str,
        role: &str,
        ticket: Option<&str>,
    ) -> Result<Member, MemberError> {
        block_on(members::add(&self.opened(), invitation, role, ticket, None))
    }

    /// Adds with `--tag tag`.
    pub fn add_tagged(
        &self,
        invitation: &str,
        role: &str,
        tag: &str,
    ) -> Result<Member, MemberError> {
        block_on(members::add(
            &self.opened(),
            invitation,
            role,
            None,
            Some(tag),
        ))
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

/// Writes `factory/roles/<role>.md` for each of `roles` in the repository at `repo`: a session's
/// role names a role file (RFC 0006, "The factory's files").
pub(super) fn role_files(repo: &Path, roles: &[&str]) {
    let dir = repo.join("factory/roles");
    fs::create_dir_all(&dir).unwrap();
    for role in roles {
        fs::write(dir.join(format!("{role}.md")), format!("# {role}\n")).unwrap();
    }
}
