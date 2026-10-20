//! `nxf-relay log-hash <kind> <id>` as a process (log-id v1 C8, 6j6v.01wt unit 5, built by
//! 6j6v.f6yb): the token a log line carries, recomputed by whoever holds the current key. The
//! rules themselves are unit-tested in `log_id.rs`; this pins the binary's argv branch, its exit
//! codes and that it never binds a port or prints the id.

use std::process::Command;

const C9_KEY: &str = "t1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

fn relay() -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("nxf-relay"));
    // Nothing from the developer's or the runner's environment decides the outcome.
    cmd.env_remove("NXS_LOG_HMAC_KEY");
    cmd
}

#[test]
fn log_hash_prints_the_c9_token_for_the_c9_key() {
    let out = relay()
        .args(["log-hash", "sid", "6j6v"])
        .env("NXS_LOG_HMAC_KEY", C9_KEY)
        .output()
        .expect("run nxf-relay log-hash");
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "sid=h1:t1:d5bfa71653806fdbcfb1052f\n"
    );
}

#[test]
fn log_hash_without_a_key_exits_two_with_nothing_on_stdout() {
    let out = relay()
        .args(["log-hash", "sid", "6j6v"])
        .output()
        .expect("run nxf-relay log-hash");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("NXS_LOG_HMAC_KEY"));
}

#[test]
fn log_hash_with_an_unknown_kind_exits_two() {
    let out = relay()
        .args(["log-hash", "stream", "seed-stream-7q3x"])
        .env("NXS_LOG_HMAC_KEY", C9_KEY)
        .output()
        .expect("run nxf-relay log-hash");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("seed-stream-7q3x"));
}

#[test]
fn an_unknown_subcommand_is_a_usage_error_not_a_server() {
    let out = relay()
        .arg("serve-please")
        .env("NXF_RELAY_ADDR", "127.0.0.1:1")
        .output()
        .expect("run nxf-relay");
    assert_eq!(out.status.code(), Some(64), "{out:?}");
}
