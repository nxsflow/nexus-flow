//! NEX-212: until NEX-97 gives it commands, v2's binary prints a placeholder and exits 0.

use std::process::Command;

#[test]
fn prints_a_placeholder_and_exits_0() {
    let out = Command::new(env!("CARGO_BIN_EXE_nxs-v2"))
        .output()
        .expect("run nxs-v2");
    assert!(out.status.success(), "exit status {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf-8");
    assert!(
        stdout.contains("nxs-v2"),
        "a placeholder naming the binary: {stdout:?}"
    );
}
