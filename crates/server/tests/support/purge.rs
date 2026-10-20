//! Shared harness of the purge suites (`purge_sqlite`, `purge_pg`, `purge_ddb`,
//! `purge_acceptance`): run `nxf-relay purge` as a process, and read what it said.
#![allow(dead_code)] // each suite uses a different subset

use std::path::Path;
use std::process::Command;

use serde_json::Value;

/// log-id v1 C9's key; every suite runs the purge under it unless a test says otherwise.
pub const C9_KEY: &str = "t1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

/// Every test-seam variable, removed from each purge process unless a test sets it.
pub const SEAM_VARS: &[&str] = &[
    "NXF_RELAY_TEST_PURGE_STOP_AFTER",
    "NXF_RELAY_TEST_PURGE_INJECT_AT",
    "NXF_RELAY_TEST_PURGE_FAIL_AT",
    "NXF_RELAY_TEST_PURGE_PAUSE_AT",
    "NXF_RELAY_TEST_PURGE_PAGE",
    "NXF_RELAY_TEST_PURGE_LEASE_SECS",
];

/// One finished `nxf-relay purge` process.
#[derive(Debug)]
pub struct Purged {
    pub code: Option<i32>,
    pub report: Value,
    pub stdout: String,
    pub stderr: String,
}

impl Purged {
    pub fn digest(&self) -> String {
        self.report["plan_digest"]
            .as_str()
            .unwrap_or_else(|| panic!("no plan_digest in {}", self.stdout))
            .to_string()
    }

    pub fn identity_digest(&self) -> String {
        self.report["store"]["identity_digest"]
            .as_str()
            .unwrap_or_else(|| panic!("no identity_digest in {}", self.stdout))
            .to_string()
    }

    /// `found` of one kind.
    pub fn found(&self, kind: &str) -> u64 {
        self.kind(kind)["found"].as_u64().unwrap_or(0)
    }

    pub fn deleted(&self, kind: &str) -> u64 {
        self.kind(kind)["deleted"].as_u64().unwrap_or(0)
    }

    pub fn kind(&self, kind: &str) -> Value {
        self.report["kinds"]
            .as_array()
            .and_then(|kinds| kinds.iter().find(|k| k["kind"] == kind).cloned())
            .unwrap_or(Value::Null)
    }

    /// `residue` as `kind -> count`.
    pub fn residue(&self) -> std::collections::BTreeMap<String, u64> {
        self.report["residue"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .map(|e| {
                        (
                            e["kind"].as_str().unwrap().to_string(),
                            e["count"].as_u64().unwrap(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Fail with everything the process said.
    pub fn expect(&self, code: i32, what: &str) -> &Purged {
        assert_eq!(
            self.code,
            Some(code),
            "{what}: expected exit {code}\nstdout: {}\nstderr: {}",
            self.stdout,
            self.stderr
        );
        self
    }
}

/// `nxf-relay purge <args>` with exactly `env` (plus PATH/HOME), every seam variable and every
/// backend variable of the caller's environment removed first.
pub fn purge(args: &[&str], env: &[(&str, &str)]) -> Purged {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("nxf-relay"));
    cmd.arg("purge").args(args);
    for var in SEAM_VARS.iter().chain(
        [
            "NXF_RELAY_BACKEND",
            "NXF_RELAY_DB",
            "NXF_RELAY_REGISTRY_DB",
            "NXF_RELAY_PG_URL",
            "DATABASE_URL",
            "NXF_RELAY_DDB_TABLE",
            "NXF_RELAY_DDB_REGISTRY_TABLE",
            "NXF_RELAY_DDB_REGION",
            "NXF_RELAY_DDB_ENDPOINT",
            "NXS_LOG_HMAC_KEY",
        ]
        .iter(),
    ) {
        cmd.env_remove(var);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run nxf-relay purge");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    // A run aborted by the stop seam prints nothing; every other run prints one report.
    let report = if stdout.is_empty() && output.status.code().is_none() {
        Value::Null
    } else {
        serde_json::from_str(&stdout).unwrap_or_else(|e| {
            panic!("stdout is not one JSON report ({e}):\nstdout: {stdout}\nstderr: {stderr}")
        })
    };
    Purged {
        code: output.status.code(),
        report,
        stdout,
        stderr,
    }
}

/// Whether `needle` occurs in `hay`.
pub fn contains_bytes(hay: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
}

/// Whether the file at `path` contains `needle` (a missing file contains nothing).
pub fn file_contains(path: &Path, needle: &str) -> bool {
    std::fs::read(path)
        .map(|bytes| contains_bytes(&bytes, needle.as_bytes()))
        .unwrap_or(false)
}
