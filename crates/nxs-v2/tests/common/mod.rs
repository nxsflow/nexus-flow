//! What the nxs-v2 tests share: a scratch folder per test, and the options and run of `nxs` in
//! this process, with the device's keys in a file, since a test machine has no key store.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use nexus_flow_seam::{DataDir, DeviceKeys, Options, Platform};
use nxs_v2::Outcome;

/// A fresh folder for one test, holding an empty `repo/`; `data/` beside it is not created.
pub fn scratch(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nxs-v2-{test}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("repo")).unwrap();
    dir.canonicalize().unwrap()
}

/// The options for a data directory at `path`, as `NXS_DATA_DIR` names it, with the keys in a
/// file in it.
pub fn options(path: &Path) -> Options {
    let path = path.to_path_buf();
    let data = DataDir::resolve(Platform::Linux, move |name| {
        (name == "NXS_DATA_DIR").then(|| path.clone().into_os_string())
    })
    .unwrap();
    let keys = DeviceKeys::File(data.root().join("device.keys"));
    Options { data, keys }
}

/// Runs `nxs <args>` in `dir`, with the data directory `base/data`.
pub fn nxs(base: &Path, dir: &Path, args: &[&str]) -> Outcome {
    let data = base.join("data");
    let argv = std::iter::once("nxs").chain(args.iter().copied());
    nxs_v2::run(argv, dir, || Ok(options(&data)))
}

/// The outcome's stdout, as text.
pub fn stdout(outcome: &Outcome) -> String {
    String::from_utf8(outcome.stdout.clone()).expect("utf-8")
}

/// The outcome's stdout, as JSON.
pub fn json(outcome: &Outcome) -> serde_json::Value {
    serde_json::from_slice(&outcome.stdout)
        .unwrap_or_else(|e| panic!("stdout is JSON ({e}): {:?}", stdout(outcome)))
}
