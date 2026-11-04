//! One fixture per rule of the layering gate (spec 6j6v.zjyv §11). Each builds the smallest metadata
//! or source that breaks exactly that rule, beside a legal neighbour, so a rule that stops biting and
//! a rule that bites too much both turn a test red.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::*;

fn dep(name: &str) -> Value {
    json!({ "name": name, "kind": null, "rename": null, "features": [] })
}

fn dev_dep(name: &str) -> Value {
    json!({ "name": name, "kind": "dev", "rename": null, "features": [] })
}

fn pkg(name: &str, deps: Vec<Value>) -> Value {
    json!({ "name": name, "dependencies": deps, "features": {} })
}

/// The new crates as §5 draws them, and the existing crates that may be involved.
fn legal() -> Vec<Value> {
    vec![
        pkg(
            "nexus-run-contracts",
            ["serde", "serde_json", "time", "sha2", "ulid"]
                .map(dep)
                .to_vec(),
        ),
        pkg(
            "nexus-run-engine",
            vec![dep("nexus-run-contracts"), dep("serde")],
        ),
        pkg(
            "nexus-run-store-sqlite",
            vec![
                dep("nexus-run-contracts"),
                dep("rusqlite"),
                dev_dep("tempfile"),
            ],
        ),
        pkg(
            "nexus-run-session-claude",
            vec![dep("nexus-run-contracts"), dep("libc")],
        ),
        pkg(
            "nexus-run-host",
            vec![
                dep("nexus-run-contracts"),
                dep("nexus-run-engine"),
                dep("nexus-run-store-sqlite"),
                dep("nexus-run-session-claude"),
            ],
        ),
        pkg(
            "nexus-chat",
            vec![dep("nexus-run-host"), dep("nexus-flow-core")],
        ),
        pkg("nxs", vec![dep("nexus-run-host"), dep("nexus-chat")]),
        pkg("nexus-flow-core", vec![dep("nexus-run-contracts")]),
        pkg("xtask", vec![dep("serde")]),
    ]
}

fn metadata(packages: Vec<Value>) -> Metadata {
    serde_json::from_value(json!({ "packages": packages })).unwrap()
}

/// `legal()` with one package's dependencies extended.
fn with_dep(package: &str, extra: Value) -> Metadata {
    let mut packages = legal();
    let target = packages
        .iter_mut()
        .find(|p| p["name"] == package)
        .unwrap_or_else(|| panic!("no fixture package {package}"));
    target["dependencies"].as_array_mut().unwrap().push(extra);
    metadata(packages)
}

fn refused(result: Result<()>) -> String {
    format!(
        "{:#}",
        result.expect_err("the gate should have refused this")
    )
}

#[test]
fn the_crates_as_the_specification_draws_them_pass() {
    check(&metadata(legal())).unwrap();
}

#[test]
fn an_engine_that_imports_nexus_chat_is_refused_naming_the_edge() {
    let message = refused(check(&with_dep("nexus-run-engine", dep("nexus-chat"))));
    assert!(
        message.contains("nexus-run-engine -> nexus-chat"),
        "{message}"
    );
}

#[test]
fn a_dev_dependency_on_an_existing_crate_is_refused_too() {
    let message = refused(check(&with_dep(
        "nexus-run-store-sqlite",
        dev_dep("nexus-flow-core"),
    )));
    assert!(
        message.contains("nexus-run-store-sqlite -> nexus-flow-core"),
        "{message}"
    );
}

#[test]
fn the_contracts_crate_may_import_no_workspace_crate_at_all() {
    let message = refused(check(&with_dep(
        "nexus-run-contracts",
        dep("nexus-run-engine"),
    )));
    assert!(
        message.contains("nexus-run-contracts -> nexus-run-engine"),
        "{message}"
    );
}

#[test]
fn an_edge_between_new_crates_outside_the_section_5_table_is_refused() {
    let message = refused(check(&with_dep(
        "nexus-run-engine",
        dep("nexus-run-store-sqlite"),
    )));
    assert!(
        message.contains("nexus-run-engine -> nexus-run-store-sqlite"),
        "{message}"
    );
}

#[test]
fn a_new_crate_the_gate_has_no_row_for_is_refused() {
    let mut packages = legal();
    packages.push(pkg("nexus-run-other", vec![]));
    let message = refused(check(&metadata(packages)));
    assert!(message.contains("nexus-run-other"), "{message}");
}

#[test]
fn an_existing_crate_other_than_chat_and_nxs_importing_the_host_is_refused() {
    let message = refused(check(&with_dep("nexus-flow-core", dep("nexus-run-host"))));
    assert!(
        message.contains("nexus-flow-core -> nexus-run-host"),
        "{message}"
    );
}

#[test]
fn a_dev_edge_into_the_host_from_a_crate_not_listed_is_refused() {
    let message = refused(check(&with_dep("xtask", dev_dep("nexus-run-host"))));
    assert!(message.contains("xtask -> nexus-run-host"), "{message}");
}

#[test]
fn an_existing_crate_importing_a_new_crate_other_than_host_and_contracts_is_refused() {
    let message = refused(check(&with_dep("nexus-chat", dep("nexus-run-engine"))));
    assert!(
        message.contains("nexus-chat -> nexus-run-engine"),
        "{message}"
    );
}

#[test]
fn any_existing_crate_may_import_the_contracts() {
    check(&with_dep("xtask", dep("nexus-run-contracts"))).unwrap();
}

#[test]
fn an_external_dependency_outside_the_crates_list_is_refused() {
    let message = refused(check(&with_dep("nexus-run-engine", dep("rand"))));
    assert!(message.contains("nexus-run-engine -> rand"), "{message}");
}

#[test]
fn an_external_dependency_on_another_crates_list_is_refused() {
    let message = refused(check(&with_dep("nexus-run-contracts", dep("rusqlite"))));
    assert!(
        message.contains("nexus-run-contracts -> rusqlite"),
        "{message}"
    );
}

#[test]
fn an_external_dev_dependency_outside_the_list_is_refused() {
    let message = refused(check(&with_dep("nexus-run-contracts", dev_dep("tempfile"))));
    assert!(
        message.contains("nexus-run-contracts -> tempfile"),
        "{message}"
    );
}

fn with_host_feature(package: &str, kind: Option<&str>) -> Metadata {
    with_dep(
        package,
        json!({
            "name": "nexus-run-host",
            "kind": kind,
            "rename": null,
            "features": ["test-support"],
        }),
    )
}

#[test]
fn a_dev_edge_may_enable_test_support() {
    check(&with_host_feature("nexus-chat", Some("dev"))).unwrap();
}

#[test]
fn a_normal_edge_enabling_test_support_is_refused() {
    let message = refused(check(&with_host_feature("nexus-chat", None)));
    assert!(message.contains("nexus-chat"), "{message}");
    assert!(message.contains("test-support"), "{message}");
}

#[test]
fn a_build_edge_enabling_test_support_is_refused() {
    let message = refused(check(&with_host_feature("nxs", Some("build"))));
    assert!(message.contains("nxs"), "{message}");
    assert!(message.contains("test-support"), "{message}");
}

fn with_feature(package: &str, feature: &str, enables: &str) -> Metadata {
    let mut packages = legal();
    let target = packages.iter_mut().find(|p| p["name"] == package).unwrap();
    target["features"][feature] = json!([enables]);
    metadata(packages)
}

#[test]
fn a_feature_that_reaches_test_support_is_refused_because_the_lean_build_could_enable_it() {
    for forwarded in [
        "nexus-run-host/test-support",
        "nexus-run-host?/test-support",
    ] {
        let message = refused(check(&with_feature("nexus-chat", "default", forwarded)));
        assert!(message.contains("nexus-chat"), "{message}");
        assert!(message.contains("test-support"), "{message}");
    }
}

#[test]
fn a_feature_that_reaches_test_support_under_a_renamed_dependency_is_refused() {
    let mut packages = legal();
    let chat = packages
        .iter_mut()
        .find(|p| p["name"] == "nexus-chat")
        .unwrap();
    chat["dependencies"][0]["rename"] = json!("run_host");
    chat["features"]["cli"] = json!(["run_host/test-support"]);
    let message = refused(check(&metadata(packages)));
    assert!(message.contains("test-support"), "{message}");
}

#[test]
fn the_host_may_declare_test_support_but_not_turn_it_on_by_default() {
    let mut packages = legal();
    let host = packages
        .iter_mut()
        .find(|p| p["name"] == "nexus-run-host")
        .unwrap();
    host["features"]["test-support"] = json!([]);
    check(&metadata(packages.clone())).unwrap();

    let host = packages
        .iter_mut()
        .find(|p| p["name"] == "nexus-run-host")
        .unwrap();
    host["features"]["default"] = json!(["test-support"]);
    let message = refused(check(&metadata(packages)));
    assert!(message.contains("default"), "{message}");
}

#[test]
fn no_other_new_crate_may_declare_a_feature() {
    let message = refused(check(&with_feature(
        "nexus-run-engine",
        "extra",
        "serde/derive",
    )));
    assert!(message.contains("nexus-run-engine"), "{message}");
    assert!(message.contains("extra"), "{message}");
}

fn source(path: &str, text: &str) -> Vec<Source> {
    vec![Source {
        path: path.to_owned(),
        text: text.to_owned(),
    }]
}

#[test]
fn a_file_of_700_lines_passes_and_of_701_lines_fails() {
    let path = "crates/run-store-sqlite/src/append.rs";
    check_sources(&source(path, &"// line\n".repeat(700))).unwrap();
    let message = refused(check_sources(&source(path, &"// line\n".repeat(701))));
    assert!(message.contains(path), "{message}");
    assert!(message.contains("701"), "{message}");
}

#[test]
fn the_size_budget_holds_for_the_gate_itself_and_not_for_other_xtask_files() {
    let long = "// line\n".repeat(701);
    let message = refused(check_sources(&source(GATE_FILE, &long)));
    assert!(message.contains(GATE_FILE), "{message}");
    check_sources(&source("xtask/src/changelog.rs", &long)).unwrap();
}

#[test]
fn an_inline_test_module_fails() {
    let path = "crates/run-contracts/src/events.rs";
    let message = refused(check_sources(&source(
        path,
        "#[cfg(test)]\nmod tests {\n}\n",
    )));
    assert!(message.contains(path), "{message}");
    assert!(message.contains("inline test"), "{message}");
}

#[test]
fn an_inline_test_module_fails_in_every_spelling_of_it() {
    let path = "crates/run-host/src/host.rs";
    for text in [
        "#[cfg(test)] mod tests { }",
        "#[cfg(test)]\n\n// a comment\nmod a_name {\n}",
        "#[cfg(test)]\n#[allow(dead_code)]\npub(crate) mod tests {\n}",
        "#[cfg(all(test, unix))]\nmod tests {\n}",
    ] {
        let message = refused(check_sources(&source(path, text)));
        assert!(message.contains("inline test"), "{text}: {message}");
    }
}

#[test]
fn a_test_function_in_a_production_file_fails() {
    let path = "crates/run-host/src/host.rs";
    let message = refused(check_sources(&source(path, "#[test]\nfn a_test() {}\n")));
    assert!(message.contains("inline test"), "{message}");
}

#[test]
fn the_out_of_line_form_and_its_tests_file_pass() {
    check_sources(&source(
        "crates/run-host/src/host.rs",
        "#[cfg(test)]\nmod tests;\n#[cfg(not(test))]\nmod real {\n}\n",
    ))
    .unwrap();
    check_sources(&source(
        "crates/run-host/src/host/tests.rs",
        "use super::*;\n#[test]\nfn a_test() {}\n",
    ))
    .unwrap();
}

#[test]
fn the_tokens_in_a_comment_or_a_string_are_not_code() {
    check_sources(&source(
        "crates/run-engine/src/decide.rs",
        concat!(
            "//! never `std::time::Instant` and never a `HashMap`\n",
            "/* #[cfg(test)] mod tests { } /* nested std::fs */ */\n",
            "const A: &str = \"std::env::var and #[test]\";\n",
            "const B: &str = r#\"a \" quote then std::thread::spawn\"#;\n",
            "const C: char = '\"';\n",
            "fn f<'a>(x: &'a str) -> &'a str { x }\n",
        ),
    ))
    .unwrap();
}

#[test]
fn engine_sources_touching_the_clock_the_environment_the_disk_threads_or_randomness_fail() {
    let path = "crates/run-engine/src/decide.rs";
    for (text, named) in [
        ("use std::time::Instant;", "std::time"),
        ("let _ = std::time::SystemTime::now();", "std::time"),
        ("let _ = std::env::var(\"X\");", "std::env"),
        ("let _ = std::fs::read(\"x\");", "std::fs"),
        ("std::thread::spawn(|| ());", "std::thread"),
        ("let _ = std :: fs :: read(\"x\");", "std::fs"),
        ("use std::{collections::BTreeMap, fs};", "std::fs"),
        ("use std::{\n  fmt,\n  time::{Duration},\n};", "std::time"),
        ("let _ = Ulid::new();", "Ulid::new"),
        ("let _ = OffsetDateTime::now_utc();", "now_utc"),
        ("let _ = RandomState::new();", "RandomState"),
    ] {
        let message = refused(check_sources(&source(path, text)));
        assert!(message.contains(path), "{text}: {message}");
        assert!(message.contains(named), "{text}: {message}");
    }
}

#[test]
fn the_purity_rule_names_the_line() {
    let path = "crates/run-engine/src/evolve.rs";
    let message = refused(check_sources(&source(
        path,
        "// one\n// two\nuse std::fs;\n",
    )));
    assert!(message.contains(&format!("{path}:3")), "{message}");
}

#[test]
fn engine_sources_may_use_what_is_not_impure() {
    check_sources(&source(
        "crates/run-engine/src/state.rs",
        "use std::collections::BTreeMap;\nuse std::fmt;\nstruct NotAStd { env: u8, fs: u8 }\n",
    ))
    .unwrap();
}

#[test]
fn the_purity_rule_is_for_the_engine_only() {
    check_sources(&source(
        "crates/run-host/src/lease.rs",
        "use std::time::Instant;\nuse std::fs;\nuse std::collections::HashMap;\n",
    ))
    .unwrap();
}

#[test]
fn a_hash_collection_in_the_engine_fails_because_its_iteration_order_differs_per_process() {
    for text in [
        "use std::collections::HashMap;",
        "fn f(x: &std::collections::HashSet<u8>) {}",
    ] {
        let path = "crates/run-engine/src/decide.rs";
        let message = refused(check_sources(&source(path, text)));
        assert!(message.contains("iteration order"), "{text}: {message}");
    }
}

#[test]
fn a_hash_collection_in_a_contract_type_fails_because_decide_would_iterate_it() {
    let path = "crates/run-contracts/src/events.rs";
    let message = refused(check_sources(&source(
        path,
        "struct S { m: HashMap<u8, u8> }",
    )));
    assert!(message.contains("iteration order"), "{message}");
}

#[test]
fn every_problem_is_named_and_not_only_the_first() {
    let sources = vec![
        Source {
            path: "crates/run-engine/src/a.rs".into(),
            text: "use std::fs;\nuse std::env;\n".into(),
        },
        Source {
            path: "crates/run-store-sqlite/src/b.rs".into(),
            text: "#[cfg(test)]\nmod tests {}\n".into(),
        },
    ];
    let message = refused(check_sources(&sources));
    for named in ["std::fs", "std::env", "run-store-sqlite/src/b.rs"] {
        assert!(message.contains(named), "{named}: {message}");
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

#[test]
fn this_repository_passes_its_own_gate() {
    run(&repo_root()).unwrap();
}

#[test]
fn the_gate_walks_the_new_crates_and_itself() {
    let paths: Vec<String> = collect_sources(&repo_root())
        .unwrap()
        .into_iter()
        .map(|s| s.path)
        .collect();
    assert!(
        paths.contains(&"crates/run-contracts/src/events.rs".to_owned()),
        "{paths:?}"
    );
    assert!(paths.contains(&GATE_FILE.to_owned()), "{paths:?}");
}

#[test]
fn the_real_metadata_names_the_contracts_crate_and_it_has_a_row() {
    let metadata = read_metadata(&repo_root()).unwrap();
    assert!(metadata
        .packages
        .iter()
        .any(|p| p.name == "nexus-run-contracts"));
    assert!(metadata.packages.iter().any(|p| p.name == "nexus-chat"));
}

#[test]
fn the_lint_job_runs_the_gate_and_the_determinism_step_before_clippy() {
    let workflow = std::fs::read_to_string(repo_root().join(".github/workflows/ci.yml")).unwrap();
    let workflow: serde_yaml::Value = serde_yaml::from_str(&workflow).unwrap();
    let steps = workflow["jobs"]["lint"]["steps"].as_sequence().unwrap();
    let position = |command: &str| {
        steps
            .iter()
            .position(|s| s["run"].as_str().is_some_and(|run| run.contains(command)))
            .unwrap_or_else(|| panic!("no lint step runs {command}"))
    };
    let clippy = position("cargo clippy");
    assert!(position("cargo xtask layering check") < clippy);
    assert!(position(DETERMINISM_SCRIPT) < clippy);
    assert_ne!(
        position("cargo xtask layering check"),
        position(DETERMINISM_SCRIPT)
    );
}

/// The determinism step's script, run against a fake `cargo`, so what it demands of the engine's
/// test can be proved before the engine exists.
#[cfg(unix)]
mod determinism_step {
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Output};

    use super::*;

    fn run_script(engine_test: bool, fake_cargo: &str) -> (Output, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        if engine_test {
            std::fs::create_dir_all(dir.path().join("crates/run-engine/tests")).unwrap();
            std::fs::write(
                dir.path().join("crates/run-engine/tests/determinism.rs"),
                "",
            )
            .unwrap();
        }
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let cargo = bin.join("cargo");
        std::fs::write(
            &cargo,
            format!(
                "#!/bin/sh\necho $$ >> \"{}/pids\"\n{fake_cargo}\n",
                dir.path().display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        let output = Command::new("bash")
            .arg(repo_root().join(DETERMINISM_SCRIPT))
            .current_dir(dir.path())
            .env("PATH", path)
            .env("NXR_DETERMINISM_PAUSE_S", "1")
            .output()
            .unwrap();
        (output, dir)
    }

    fn stderr(output: &Output) -> String {
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    #[test]
    fn before_the_engine_exists_the_step_says_so_and_passes() {
        let (output, _dir) = run_script(false, "exit 99");
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(String::from_utf8_lossy(&output.stdout).contains("not built yet"));
    }

    #[test]
    fn an_engine_without_the_determinism_test_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("crates/run-engine/src")).unwrap();
        let output = Command::new("bash")
            .arg(repo_root().join(DETERMINISM_SCRIPT))
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("determinism.rs"),
            "{}",
            stderr(&output)
        );
    }

    #[test]
    fn two_processes_that_write_the_same_decisions_pass() {
        let (output, dir) = run_script(true, "printf 'the list' > \"$NXR_DETERMINISM_OUT\"");
        assert!(output.status.success(), "{}", stderr(&output));
        let pids = std::fs::read_to_string(dir.path().join("pids")).unwrap();
        let pids: std::collections::BTreeSet<_> = pids.lines().collect();
        assert_eq!(pids.len(), 2, "two separate processes");
    }

    #[test]
    fn a_decide_that_reads_the_environment_fails() {
        let (output, _dir) = run_script(true, "printf \"$TZ\" > \"$NXR_DETERMINISM_OUT\"");
        assert!(!output.status.success());
        assert!(stderr(&output).contains("differ"), "{}", stderr(&output));
    }

    #[test]
    fn a_decide_that_reads_the_clock_fails() {
        let (output, _dir) = run_script(true, "date +%s > \"$NXR_DETERMINISM_OUT\"");
        assert!(!output.status.success());
        assert!(stderr(&output).contains("differ"), "{}", stderr(&output));
    }

    #[test]
    fn a_test_that_writes_nothing_proves_nothing_and_fails() {
        let (output, _dir) = run_script(true, ": > \"$NXR_DETERMINISM_OUT\"");
        assert!(!output.status.success());
        assert!(
            stderr(&output).contains("wrote nothing"),
            "{}",
            stderr(&output)
        );
    }

    #[test]
    fn a_red_test_fails_the_step() {
        let (output, _dir) = run_script(true, "exit 101");
        assert!(!output.status.success());
    }
}
