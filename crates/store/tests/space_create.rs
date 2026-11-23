//! `nxs init`'s store half (RFC 0001, "Creating and finding a space", "The local data on disk",
//! "Migration"): the store opens the device member's database under `device/` in the data
//! directory, creates the repository space with the device as its first admin, and records the
//! space's id in `nexus-flow.toml`.

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use nexus_flow_store::config::Config;
use nexus_flow_store::data_dir::{DataDir, Platform};
use nexus_flow_store::repo_root::{RepoRoot, RootError};
use nexus_flow_store::space::{init, DeviceKeys, InitError, InitReport, SyncMode};

/// Runs `future` to completion on this thread. A database without a relay does no I/O that
/// needs a runtime.
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

/// A fresh folder for one test, holding `repo/` and `data/` side by side.
fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-space-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("repo")).unwrap();
    dir.canonicalize().unwrap()
}

fn data_dir(path: &Path) -> DataDir {
    let path = path.to_path_buf();
    DataDir::resolve(Platform::Linux, move |name| {
        (name == "NXS_DATA_DIR").then(|| path.clone().into_os_string())
    })
    .unwrap()
}

/// The device's keys in a file beside its database: the test machine has no keystore.
fn keys(data: &DataDir) -> DeviceKeys {
    DeviceKeys::File(data.root().join("device.keys"))
}

fn init_in(base: &Path) -> Result<InitReport, InitError> {
    let data = data_dir(&base.join("data"));
    block_on(init(&base.join("repo"), &data, keys(&data)))
}

#[test]
fn a_new_repository_gets_a_space_recorded_in_nexus_flow_toml() {
    let base = scratch("new");
    let report = init_in(&base).unwrap();

    assert!(report.created, "{report:?}");
    let root = RepoRoot::find(&base.join("repo")).unwrap();
    assert_eq!(Config::read(&root).unwrap().repository, report.space);
    assert!(
        base.join("data").join("device").is_dir(),
        "the device member's database lives under device/"
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn creating_twice_creates_one_space_and_reports_the_existing_id() {
    let base = scratch("twice");
    let first = init_in(&base).unwrap();
    let file = base.join("repo").join("nexus-flow.toml");
    let written = fs::read(&file).unwrap();

    let second = init_in(&base).unwrap();
    assert!(!second.created, "{second:?}");
    assert_eq!(second.space, first.space);
    assert_eq!(fs::read(&file).unwrap(), written, "the file is left as it was");
    assert!(
        second.lines().iter().any(|l| l.contains(&first.space)),
        "{:?}",
        second.lines()
    );

    // From a folder inside the repository, the same space is found.
    fs::create_dir_all(base.join("repo").join("src")).unwrap();
    let data = data_dir(&base.join("data"));
    let nested = block_on(init(&base.join("repo").join("src"), &data, keys(&data))).unwrap();
    assert_eq!((nested.created, nested.space), (false, first.space));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn without_a_relay_the_space_is_local_in_island_mode() {
    let base = scratch("island");
    let report = init_in(&base).unwrap();

    assert_eq!(report.sync, SyncMode::Island);
    assert!(
        report.lines().iter().any(|l| l.contains("island mode")),
        "{:?}",
        report.lines()
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_v1_workspace_is_left_untouched_beside_an_empty_space() {
    let base = scratch("v1");
    let nxs = base.join("repo").join(".nxs");
    fs::create_dir_all(nxs.join("plugins")).unwrap();
    fs::write(nxs.join("db.sqlite"), b"v1 bytes").unwrap();
    fs::write(nxs.join("plugins").join("issue-tracker.json"), b"{}").unwrap();

    let report = init_in(&base).unwrap();

    assert!(report.created && report.v1_workspace, "{report:?}");
    assert!(
        report
            .lines()
            .iter()
            .any(|l| l.contains(".nxs") && l.contains("not moved")),
        "{:?}",
        report.lines()
    );
    assert_eq!(fs::read(nxs.join("db.sqlite")).unwrap(), b"v1 bytes");
    assert_eq!(
        fs::read(nxs.join("plugins").join("issue-tracker.json")).unwrap(),
        b"{}"
    );
    let mut entries: Vec<_> = fs::read_dir(&nxs)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    entries.sort();
    assert_eq!(entries, ["db.sqlite", "plugins"], "nothing was added to .nxs/");
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_repository_without_v1_data_gets_no_line_about_it() {
    let base = scratch("no-v1");
    let report = init_in(&base).unwrap();
    assert!(!report.v1_workspace);
    assert!(!report.lines().iter().any(|l| l.contains(".nxs")));
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn the_report_recommends_a_second_admin_device() {
    let base = scratch("admin");
    let report = init_in(&base).unwrap();
    assert!(
        report
            .lines()
            .iter()
            .any(|l| l.contains("second admin device")),
        "{:?}",
        report.lines()
    );
    fs::remove_dir_all(&base).unwrap();
}

#[test]
fn a_data_directory_inside_the_repository_is_refused_before_anything_is_written() {
    let base = scratch("inside");
    let data = data_dir(&base.join("repo").join(".data"));

    let err = block_on(init(&base.join("repo"), &data, keys(&data))).unwrap_err();
    assert!(
        matches!(err, InitError::Root(RootError::DataDirInside { .. })),
        "{err:?}"
    );
    assert!(!base.join("repo").join(".data").exists());
    assert!(!base.join("repo").join("nexus-flow.toml").exists());
    fs::remove_dir_all(&base).unwrap();
}
