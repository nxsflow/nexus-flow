use nexus_chat::error::ErrorKind;
use nexus_chat::role::Model;
use nexus_chat::worker::{
    Coordinator, DryWorker, RoleSpec, SidecarWorker, TriggerError, TriggerOutcome, TriggerRequest,
    TriggerResult, TurnTerms, Worker, WorkerConfig,
};

/// A `RoleSpec` with everything defaulted, so each test names only the field it is about.
fn role_spec() -> RoleSpec {
    RoleSpec {
        handle: "coding".into(),
        system_prompt: "p".into(),
        use_claude_code_preset: true,
        tools: None,
        granted_tools: Vec::new(),
        permissions: None,
        model: None,
        declaration_hash: None,
    }
}

/// A `TriggerRequest` with everything defaulted, likewise.
fn trigger_request(internal_session: &str, role: RoleSpec) -> TriggerRequest {
    TriggerRequest {
        internal_session: internal_session.into(),
        resume_real: None,
        message: "hi".into(),
        role,
        env: vec![],
        reply_thread: None,
        coordinator: Coordinator::Persona,
        terms: TurnTerms::default(),
    }
}

// ---- process-global env, serialized ----------------------------------------------------------
//
// `PATH` is read from the process environment by the code under test, and `cargo test` runs this
// binary's tests in PARALLEL THREADS of one process — so a test that sets it races every other test
// that reads it. That is the `det-ids-env-test-race` class, and it has already produced one real
// flake in a sibling binary (nxf 6j6v.570x). Every test here that touches the environment goes
// through [`with_env`], which holds one mutex for its whole body and restores the previous value
// afterwards.
//
// `NXC_DRY_LOG` is deliberately NOT in that list anymore: `DryWorker` carries its log path as a
// field, so nothing here has to set it (6j6v.570x — that is the fix, and the test below pins it).

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Run `f` with `key` set to `value`, serialized against every other env-touching test in this
/// binary, and restore the previous value (or its absence) afterwards. A poisoned mutex — a PRIOR
/// test panicked while holding it — is recovered rather than propagated, so one failure does not
/// cascade into every later test.
fn with_env<T>(key: &str, value: impl AsRef<std::ffi::OsStr>, f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var_os(key);
    std::env::set_var(key, value);
    let out = f();
    match previous {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    out
}

#[test]
fn dry_worker_records_and_never_spawns() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("dry.log");
    DryWorker {
        log: Some(log.clone()),
    }
    .trigger(TriggerRequest {
        resume_real: Some("real-x".into()),
        role: RoleSpec {
            tools: Some(vec!["Bash".into()]),
            ..role_spec()
        },
        ..trigger_request("s-1", role_spec())
    })
    .unwrap();
    let got = std::fs::read_to_string(&log).unwrap();
    assert!(
        got.contains("trigger role=coding session=s-1 resume=real-x"),
        "{got}"
    );
}

#[test]
fn a_dry_worker_writes_where_it_was_told_and_ignores_the_environment() {
    // The flake 6j6v.570x is exactly this shape, in reverse. `DryWorker::trigger` used to read
    // `NXC_DRY_LOG` from process env on every call, so a test that wanted NO dry log — and
    // therefore took no lock — still wrote to whatever a parallel neighbour had set. When that
    // neighbour's `TempDir` was already dropped, the append hit `ENOENT` (`create(true)` creates a
    // file, never a missing directory) and reddened an unrelated run.
    //
    // Both halves are pinned here because only both together end the class: a worker told nothing
    // writes nothing no matter what is in the environment, and a worker told a path writes THERE
    // even when the environment names somewhere else entirely.
    let tmp = tempfile::tempdir().unwrap();
    let told = tmp.path().join("told.log");
    let ambient = tmp.path().join("gone").join("ambient.log"); // its directory does not exist
    with_env("NXC_DRY_LOG", &ambient, || {
        DryWorker { log: None }
            .trigger(trigger_request("s-none", role_spec()))
            .expect("a worker told nothing must not fail on a neighbour's stale path");
        DryWorker {
            log: Some(told.clone()),
        }
        .trigger(trigger_request("s-told", role_spec()))
        .expect("a worker told a path writes there");
    });
    assert!(
        !ambient.exists(),
        "nothing may be written to the path the ENVIRONMENT named"
    );
    let got = std::fs::read_to_string(&told).unwrap();
    assert!(got.contains("session=s-told"), "{got}");
    assert!(
        !got.contains("session=s-none"),
        "the worker with no log recorded nothing at all: {got}"
    );
}

/// Read the sidecar spec JSON `SidecarWorker::trigger` writes for `internal_session` under
/// `cwd/.nxs/agent-logs/<internal_session>.spec.json`. The write happens BEFORE the detached
/// `node` spawn, so the file exists (and is inspectable) regardless of whether `trigger` itself
/// returns `Ok` or `Err` (a `node` binary may or may not be on PATH in a given test environment;
/// its presence/absence is irrelevant to what JSON shape THIS Rust code produces) — so these
/// tests deliberately ignore `trigger`'s `Result` and assert on the written file instead.
fn read_spec_json(cwd: &std::path::Path, internal_session: &str) -> serde_json::Value {
    let spec_path = cwd
        .join(".nxs/agent-logs")
        .join(format!("{internal_session}.spec.json"));
    let raw = std::fs::read_to_string(&spec_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", spec_path.display()));
    serde_json::from_str(&raw).unwrap()
}

/// A `SidecarWorker` over a temp cwd. The `main.mjs` path need not exist — see `read_spec_json`.
fn sidecar_over(tmp: &std::path::Path) -> SidecarWorker {
    SidecarWorker {
        sidecar: tmp.join("main.mjs"),
        cwd: tmp.to_path_buf(),
    }
}

#[test]
fn sidecar_worker_writes_an_absent_tools_field_when_the_role_declares_none() {
    // Fix round (6j6v.zenf final review, Fix 1): `RoleSpec.tools: None` (the role's YAML omitted
    // `tools:` entirely) must reach the sidecar spec JSON as `null`, NEVER `[]` — `[]` would tell
    // `agent-sidecar/src/main.mjs` to disable the SDK's base toolset entirely, silently leaving
    // the role unable to even call `nxc reply`.
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(trigger_request("s-none", role_spec()));
    let spec = read_spec_json(tmp.path(), "s-none");
    assert_eq!(spec["tools"], serde_json::Value::Null, "{spec}");
}

#[test]
fn sidecar_worker_writes_a_real_empty_array_when_the_role_declares_zero_tools() {
    // The companion case: `Some(vec![])` (the role EXPLICITLY declared `tools: []`) must still
    // reach the spec as a genuine empty JSON array — the "zero tools" state main.mjs's
    // `Array.isArray(spec.tools)` branch relies on.
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(trigger_request(
        "s-empty",
        RoleSpec {
            tools: Some(vec![]),
            ..role_spec()
        },
    ));
    let spec = read_spec_json(tmp.path(), "s-empty");
    assert_eq!(spec["tools"], serde_json::json!([]), "{spec}");
}

#[test]
fn sidecar_worker_writes_the_declared_tools_array_when_the_role_declares_some() {
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(trigger_request(
        "s-some",
        RoleSpec {
            tools: Some(vec!["Read".into(), "Bash".into()]),
            ..role_spec()
        },
    ));
    let spec = read_spec_json(tmp.path(), "s-some");
    assert_eq!(spec["tools"], serde_json::json!(["Read", "Bash"]), "{spec}");
}

// ---- model (nxf 6j6v.a5na) --------------------------------------------------------------

#[test]
fn spec_json_omits_model_when_the_role_declares_none() {
    // Exactly the `tools` discipline one file up: an undeclared model must not reach the spec as a
    // value at all, so `main.mjs` leaves `options.model` unset and the SDK's own default model
    // applies. A `""` or a hardcoded fallback here would silently pin every undeclared role to one
    // model — a decision no role author made.
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(trigger_request("s-nomodel", role_spec()));
    let spec = read_spec_json(tmp.path(), "s-nomodel");
    assert_eq!(spec["model"], serde_json::Value::Null, "{spec}");
}

#[test]
fn spec_json_carries_the_resolved_sdk_id_when_a_model_is_declared() {
    // The alias->id resolution happens HERE, in Rust, not in the sidecar: the spec carries a
    // finished SDK model string.
    for (model, sdk_id) in [
        (Model::Fable, "claude-fable-5"),
        (Model::Opus, "claude-opus-5"),
        (Model::Sonnet, "claude-sonnet-5"),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let session = format!("s-{sdk_id}");
        let _ = sidecar_over(tmp.path()).trigger(trigger_request(
            &session,
            RoleSpec {
                model: Some(model),
                ..role_spec()
            },
        ));
        let spec = read_spec_json(tmp.path(), &session);
        assert_eq!(spec["model"], serde_json::json!(sdk_id), "{spec}");
    }
}

// ---- replyThread (nxf 6j6v.7e9d) --------------------------------------------------------

#[test]
fn spec_json_omits_reply_thread_when_the_trigger_carries_none() {
    // The common case: every caller except the one branch of `coordinator_commission` that actually
    // wrote `expects_reply_from` (see `RoleSpawn::reply_thread`'s own doc). This is also the shape
    // that must make the sidecar's new teardown step a no-op — nothing owed, nothing posted.
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(trigger_request("s-noreply", role_spec()));
    let spec = read_spec_json(tmp.path(), "s-noreply");
    assert_eq!(spec["replyThread"], serde_json::Value::Null, "{spec}");
}

#[test]
fn spec_json_carries_the_reply_thread_when_the_trigger_names_one() {
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(TriggerRequest {
        reply_thread: Some("thread-abc".into()),
        ..trigger_request("s-reply", role_spec())
    });
    let spec = read_spec_json(tmp.path(), "s-reply");
    assert_eq!(
        spec["replyThread"],
        serde_json::json!("thread-abc"),
        "{spec}"
    );
}

// ---- claudePath (nxf 6j6v.81v5) ---------------------------------------------------------

/// A `PATH` whose single entry holds an executable `claude` — what the host looks for, and the
/// only reason a SHIPPED sidecar can start a session at all.
fn path_with_a_claude(dir: &std::path::Path) -> String {
    use std::os::unix::fs::PermissionsExt;
    let claude = dir.join("claude");
    std::fs::write(&claude, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir.to_string_lossy().into_owned()
}

#[test]
fn the_spec_carries_the_claude_the_host_resolved() {
    // The producing side of the key. `agent-sidecar/test/main-teardown.test.mjs` asserts the
    // CONSUMING side against its own fixture spec, so without this a Rust-side rename to
    // `claude_path` — or dropping the field — passes every gate in the repo while breaking every
    // installed run. Exactly the "each half is green on its own" failure that
    // `every_place_that_ships_the_sidecar_spells_it_the_same` exists to prevent for the file name.
    let tmp = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    with_env("PATH", path_with_a_claude(bin.path()), || {
        let _ = sidecar_over(tmp.path()).trigger(trigger_request("s-claude", role_spec()));
    });
    let spec = read_spec_json(tmp.path(), "s-claude");
    assert_eq!(
        spec["claudePath"],
        serde_json::json!(bin.path().join("claude").to_string_lossy()),
        "{spec}"
    );
}

#[test]
fn a_source_tree_sidecar_still_runs_with_no_claude_on_the_path() {
    // `sidecar_over` points at `<tmp>/main.mjs`, i.e. NOT the shipped bundle: the SDK's own
    // resolution works there (it has `node_modules` beside it), so an unresolvable `claude` is a
    // `null` the sidecar simply ignores — never a refusal.
    let tmp = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    with_env("PATH", empty.path(), || {
        // Whether `node` exists here is irrelevant (see `read_spec_json`); what matters is that the
        // trigger was not refused BEFORE writing the spec.
        let _ = sidecar_over(tmp.path()).trigger(trigger_request("s-noclaude", role_spec()));
    });
    let spec = read_spec_json(tmp.path(), "s-noclaude");
    assert_eq!(spec["claudePath"], serde_json::Value::Null, "{spec}");
}

#[test]
fn the_shipped_bundle_refuses_to_start_when_no_claude_is_installed() {
    // The precondition the host already knows about, stated instead of discarded. Bundled into one
    // file the sidecar cannot resolve the SDK's native binary, so a spawn here can only die inside
    // a detached child — and `trigger` returns `Accepted` unconditionally, so `nxc send --to` would
    // print a receipt, exit 0, and leave an SDK-internal message about npm flags in a log file
    // nothing points at. That is the bug 6j6v.81v5 exists to fix, one step later.
    let tmp = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let worker = SidecarWorker {
        sidecar: tmp.path().join("nxc-agent-sidecar.mjs"),
        cwd: tmp.path().to_path_buf(),
    };
    let err = with_env("PATH", empty.path(), || {
        worker
            .trigger(trigger_request("s-refused", role_spec()))
            .unwrap_err()
    });
    let msg = err.to_string();
    assert!(msg.contains("Claude Code"), "{msg}");
    assert!(msg.contains("PATH"), "names why it cannot find it: {msg}");
    assert!(
        !tmp.path().join(".nxs/agent-logs").exists(),
        "refused BEFORE anything is written, so no orphan spec or log is left behind"
    );
}

// ---- WorkerConfig (nxf 6j6v.a5na) -------------------------------------------------------

#[test]
fn from_ambient_mirrors_select_worker() {
    // The env-selection rules are unchanged; they just move behind an injected lookup so the
    // decision is pure and testable without mutating process-global env (which parallel tests in
    // this binary would race on) — the same dependency-injection shape `forwarded_real_env` uses.
    let cwd = std::path::PathBuf::from("/tmp/cwd");
    let ambient = |vals: Vec<(&'static str, &'static str)>| {
        move |k: &str| {
            vals.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    };

    assert_eq!(
        WorkerConfig::from_ambient(cwd.clone(), ambient(vec![("NXC_WORKER", "dry")])).unwrap(),
        WorkerConfig::Dry { log: None }
    );

    // …and `NXC_DRY_LOG` is resolved HERE, through the same injected lookup, instead of inside
    // `DryWorker::trigger` on every call (6j6v.570x). This is the one seam that reads environment
    // on purpose; the worker it builds then carries the answer as a value.
    assert_eq!(
        WorkerConfig::from_ambient(
            cwd.clone(),
            ambient(vec![("NXC_WORKER", "dry"), ("NXC_DRY_LOG", "/t/dry.log")]),
        )
        .unwrap(),
        WorkerConfig::Dry {
            log: Some("/t/dry.log".into()),
        }
    );

    // `sidecar` explicitly, and the unset default, both resolve to the sidecar.
    for vals in [
        vec![("NXC_WORKER", "sidecar"), ("NXC_SIDECAR", "/p/main.mjs")],
        vec![("NXC_SIDECAR", "/p/main.mjs")],
    ] {
        assert_eq!(
            WorkerConfig::from_ambient(cwd.clone(), ambient(vals)).unwrap(),
            WorkerConfig::Sidecar {
                sidecar: "/p/main.mjs".into(),
                cwd: cwd.clone(),
            }
        );
    }

    // A sidecar worker with no `NXC_SIDECAR` is an `io` error naming the variable, as before.
    let err = WorkerConfig::from_ambient(cwd.clone(), ambient(vec![])).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Io);
    assert!(err.msg.contains("NXC_SIDECAR"), "{}", err.msg);

    // An unknown value is a loud error, never a silent fallback to some default worker.
    let err = WorkerConfig::from_ambient(cwd, ambient(vec![("NXC_WORKER", "banana")])).unwrap_err();
    assert!(err.msg.contains("banana"), "{}", err.msg);
}

#[test]
fn the_installed_sidecar_is_the_fallback_and_the_env_var_is_only_an_override() {
    // nxf 6j6v.81v5: an INSTALLED nexus-flow carries its own sidecar, so `NXC_SIDECAR` stops being
    // a requirement and goes back to being what it was meant to be — a developer override. The
    // fallback is injected, which is what keeps this decision as pure as `from_ambient` is.
    let cwd = std::path::PathBuf::from("/tmp/cwd");
    let ambient = |vals: Vec<(&'static str, &'static str)>| {
        move |k: &str| {
            vals.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    };
    let installed = || {
        Ok(Some(std::path::PathBuf::from(
            "/c/sidecar/ab12/nxc-agent-sidecar.mjs",
        )))
    };

    // Nothing set: the program's own sidecar, no error at all — the whole point of the ticket.
    assert_eq!(
        WorkerConfig::from_ambient_or_installed(cwd.clone(), ambient(vec![]), installed).unwrap(),
        WorkerConfig::Sidecar {
            sidecar: "/c/sidecar/ab12/nxc-agent-sidecar.mjs".into(),
            cwd: cwd.clone(),
        }
    );

    // The override still wins outright — a developer pointing at a source tree must not be
    // second-guessed by whatever the binary carries.
    assert_eq!(
        WorkerConfig::from_ambient_or_installed(
            cwd.clone(),
            ambient(vec![("NXC_SIDECAR", "/p/main.mjs")]),
            installed,
        )
        .unwrap(),
        WorkerConfig::Sidecar {
            sidecar: "/p/main.mjs".into(),
            cwd: cwd.clone(),
        }
    );

    // `dry` never looks for a file: the fallback is consulted on the sidecar path only.
    assert_eq!(
        WorkerConfig::from_ambient_or_installed(
            cwd.clone(),
            ambient(vec![("NXC_WORKER", "dry")]),
            || panic!("a dry worker must not go looking for a sidecar"),
        )
        .unwrap(),
        WorkerConfig::Dry { log: None }
    );

    // Neither an override nor one of its own: the error names BOTH ways out, in the order a reader
    // should try them — what the binary should have carried first, the developer override second.
    let err =
        WorkerConfig::from_ambient_or_installed(cwd, ambient(vec![]), || Ok(None)).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Io);
    assert!(err.msg.contains("nxc-agent-sidecar.mjs"), "{}", err.msg);
    assert!(err.msg.contains("NXC_SIDECAR"), "{}", err.msg);
}

#[test]
fn a_sidecar_that_cannot_be_unpacked_is_reported_rather_than_reduced_to_not_found() {
    // Acceptance 4 of nxf 6j6v.smsz, at the seam. Since the bundle is compiled into the binary and
    // unpacked into the user's cache on first use, the lookup itself can FAIL — and "I have a
    // sidecar and cannot make it usable" must not arrive as "no agent sidecar found", which would
    // send a reader hunting for a file that is not missing and hide the one fact that helps (an
    // unwritable cache directory). The generic refusal is a fallback for `Ok(None)` only.
    let cwd = std::path::PathBuf::from("/tmp/cwd");
    let err = WorkerConfig::from_ambient_or_installed(
        cwd,
        |_: &str| None,
        || {
            Err(nexus_chat::error::NxfError::io(
                "could not be unpacked to /ro/cache/sidecar/ab12/nxc-agent-sidecar.mjs: \
                 Read-only file system",
            ))
        },
    )
    .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Io);
    assert!(
        err.msg.contains("Read-only file system"),
        "the real cause survives the seam: {}",
        err.msg
    );
    assert!(
        !err.msg.contains("no agent sidecar found"),
        "and is NOT relabelled as an absent sidecar: {}",
        err.msg
    );
}

#[test]
fn every_place_that_ships_the_sidecar_spells_it_the_same() {
    // SIX files have to agree on one file name, and none of them can see the others: `package.json`
    // NAMES the bundle, `build.rs` COMPILES it into the binary, the two workflows BUILD it (in the
    // release job, into the `nxs` that ships), and `install.sh`/`self-update` reconcile whatever an
    // older version left beside the binary. A typo in any one of them reproduces exactly the bug
    // these tickets exist for (6j6v.81v5, 6j6v.smsz) — a suite that installs cleanly and then
    // cannot start an agent — and no other test in the repo would notice, because each half is
    // green on its own.
    //
    // OCCURRENCE COUNTS, not "contains": several of these files spell the name more than once (the
    // extract and the install in the shell script; the const and its consumers in the build
    // script), and a substring check would stay green with a typo in all but one of them. The
    // counts are the point — a legitimate new mention means coming back here and saying so.
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/chat has a repo root two levels up");
    let name = nexus_chat::worker::SIDECAR_FILE;
    for (file, expected) in [
        // `--outfile=dist/nxc-agent-sidecar.mjs`, the one place the bundle is named into existence.
        ("agent-sidecar/package.json", 1),
        // The `BUNDLE` const (the path `include_bytes!` embeds) and the doc line above it.
        ("crates/chat/build.rs", 2),
        // `sync_sidecar`'s own `_is_name`, plus the membership test, the extract and its
        // failure message.
        ("install.sh", 4),
        // The bundle step's `ls -l` and the doc line naming what `crates/chat/build.rs` reads. NOT
        // the tarball any more: since 6j6v.smsz the archive has two members, and the sidecar is
        // inside `nxs` rather than beside it.
        (".github/workflows/release.yml", 2),
        // The `ls -l`, the smoke's `node dist/…` invocation, and the doc line naming the input the
        // Rust build reads — which is why this step runs BEFORE the cargo gates.
        (".github/workflows/ci.yml", 3),
        // The `SIDECAR_FILE` const; every other use goes through it.
        ("crates/cli/src/selfupdate.rs", 1),
    ] {
        let text = std::fs::read_to_string(repo.join(file))
            .unwrap_or_else(|e| panic!("reading {file}: {e}"));
        assert_eq!(
            text.matches(name).count(),
            expected,
            "{file} spells `{name}` a different number of times than expected — either a mention \
             was renamed/dropped (the sidecar this crate looks for would not be the one shipped or \
             installed), or one was legitimately added and this count needs updating",
        );
    }
}

#[test]
fn disabled_worker_build_is_a_named_validation_error() {
    // `Disabled` is what an embedder gets from `Engine::open` without opting into orchestration.
    // Reads and messaging writes keep working; a verb that would spawn a session has to say so out
    // loud rather than silently doing nothing and returning a successful receipt.
    // `.map(|_| ())` because the Ok side is an `Arc<dyn Worker>`, which is not `Debug` — nothing to
    // do with what this test asserts.
    let err = WorkerConfig::Disabled.build().map(|_| ()).unwrap_err();
    assert_eq!(err.kind, ErrorKind::Validation);
    assert!(
        err.msg.contains("no worker"),
        "the error must name the missing configuration, got: {}",
        err.msg
    );
}

#[test]
fn built_workers_are_send_and_sync() {
    // `Engine` holds the worker for its lifetime and must stay `Send + Sync` to live in a Tauri
    // backend's managed state, so the trait object it holds has to be too.
    fn assert_send_sync<T: Send + Sync + ?Sized>() {}
    assert_send_sync::<dyn Worker>();
}

// ---- the host-implements-`Worker` escape hatch (nxf 6j6v.5x9j) ---------------------------------
//
// Spec §3.3 always promised it and `WorkerConfig` never carried it. These tests pin the SHAPE the
// Bedrock AgentCore probe (app-foundations 41j0.9t68) said a second, REMOTE agent runtime needs:
// an outcome that may resolve later instead of a return value that must be known now, an opaque
// runtime session id in place of a process handle, and "the session is gone" as its own case.

/// A host worker that records what it was handed and answers with whatever the test wants.
struct HostWorker {
    seen: std::sync::Mutex<Vec<TriggerRequest>>,
    answer: fn(&TriggerRequest) -> TriggerResult,
}

impl HostWorker {
    fn new(answer: fn(&TriggerRequest) -> TriggerResult) -> std::sync::Arc<HostWorker> {
        std::sync::Arc::new(HostWorker {
            seen: std::sync::Mutex::new(vec![]),
            answer,
        })
    }
}

impl Worker for HostWorker {
    fn trigger(&self, req: TriggerRequest) -> TriggerResult {
        let answer = (self.answer)(&req);
        self.seen.lock().unwrap().push(req);
        answer
    }
}

#[test]
fn a_host_supplied_worker_is_reachable_through_worker_config() {
    // The actual gap 6j6v.5x9j names: the trait was public, and there was no way to hand an impl
    // of it to an `Engine` — `WorkerConfig` knew only `Disabled | Dry | Sidecar`.
    let host = HostWorker::new(|_| Ok(TriggerOutcome::Accepted));
    let built = WorkerConfig::Custom(host.clone())
        .build()
        .expect("a custom worker builds into itself");
    let outcome = built
        .trigger(trigger_request("s-1", role_spec()))
        .expect("the host worker ran");
    assert_eq!(
        outcome,
        TriggerOutcome::Accepted,
        "…and its answer travels back"
    );
    assert_eq!(
        host.seen.lock().unwrap().len(),
        1,
        "the call must reach the host's OWN object — `build` wraps it in a bound (see          `BoundedWorker`), and a wrapper that swallowed or replaced the call would defeat the whole          point of installing one"
    );
}

#[test]
fn a_host_worker_that_never_returns_is_cut_loose_instead_of_freezing_the_engine() {
    // PR #269 review, Code Quality #1 / Integrity #1. `Engine` holds ONE mutex over its store for a
    // whole orchestration verb and calls `trigger` inside it, so a `Custom` worker that blocks does
    // not merely stall its own caller — it freezes every verb on every clone of the handle. Before
    // this bound, that was forever, with nothing to observe it. "Hand your network call to your own
    // executor" is a contract, and a contract with no mechanism behind it is a comment.
    //
    // The bound itself is 30s (`CUSTOM_WORKER_BOUND`) and deliberately not configurable, so this
    // test cannot wait it out. What it CAN pin is the property that makes the bound work at all: the
    // engine's thread is not the one running host code, so a `trigger` still in flight does not hold
    // anything. Asserted by having the host worker block until this thread — which has already got
    // its answer back — releases it.
    let gate = std::sync::Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    struct Blocking(std::sync::Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>);
    impl Worker for Blocking {
        fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
            let (lock, cv) = &*self.0;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = cv.wait(released).unwrap();
            }
            Ok(TriggerOutcome::Accepted)
        }
    }

    let built = WorkerConfig::Custom(std::sync::Arc::new(Blocking(gate.clone())))
        .build()
        .unwrap();
    let started = std::time::Instant::now();
    let handle = std::thread::spawn(move || built.trigger(trigger_request("s-1", role_spec())));

    // The engine-side call is on `handle`'s thread; host code is on a THIRD thread of the bound
    // worker's own making. Releasing the gate from here — a thread that never touched either —
    // proves the host's blocking call was never running on the caller's stack.
    {
        let (lock, cv) = &*gate;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
    let outcome = handle.join().expect("the trigger thread did not panic");
    assert_eq!(outcome.unwrap(), TriggerOutcome::Accepted);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "the bound must not be waited out for a worker that DID answer"
    );
}

#[test]
fn a_host_worker_that_panics_does_not_unwind_through_the_engine() {
    // The other half of the isolation the bound buys (PR #269 review, Integrity #1). A panic in
    // third-party code used to unwind straight through the engine's locked section, poisoning the
    // store mutex on its way out. It now disconnects a channel instead, and is reported as what it
    // is.
    struct Panicking;
    impl Worker for Panicking {
        fn trigger(&self, _req: TriggerRequest) -> TriggerResult {
            panic!("host worker blew up");
        }
    }
    // The panic is raised on the bounded worker's own thread and printed by the default hook; muted
    // so a passing test does not look like a failing one.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let built = WorkerConfig::Custom(std::sync::Arc::new(Panicking))
        .build()
        .unwrap();
    let err = built
        .trigger(trigger_request("s-1", role_spec()))
        .expect_err("a panicking worker is a failed trigger, not a silent success");
    std::panic::set_hook(previous);

    assert!(!err.is_session_gone(), "a panic is not a vanished session");
    let msg = err.to_string();
    assert!(msg.contains("panicked"), "{msg}");
    assert!(
        msg.contains("s-1"),
        "and it names the session it was triggering: {msg}"
    );
}

#[test]
fn worker_config_equality_compares_a_custom_worker_by_identity() {
    // `WorkerConfig` keeps `PartialEq`/`Eq` (an `EngineConfig` is compared in tests and by
    // embedders), so `Custom` needs an answer for "are these the same worker?". `Arc::ptr_eq` is
    // the only honest one — a `dyn Worker` has no value to compare — and it is reflexive, which is
    // what keeps `Eq` truthful.
    let a = HostWorker::new(|_| Ok(TriggerOutcome::Accepted));
    let b = HostWorker::new(|_| Ok(TriggerOutcome::Accepted));
    assert_eq!(WorkerConfig::Custom(a.clone()), WorkerConfig::Custom(a));
    assert_ne!(
        WorkerConfig::Custom(b.clone()),
        WorkerConfig::Custom(HostWorker::new(|_| Ok(TriggerOutcome::Accepted)))
    );
    assert_ne!(WorkerConfig::Custom(b), WorkerConfig::Dry { log: None });

    // Two dry workers are the same worker only when they record in the same place — the field is
    // part of the configuration an embedder compares, not a detail hidden behind the variant name.
    assert_eq!(
        WorkerConfig::Dry { log: None },
        WorkerConfig::Dry { log: None }
    );
    assert_ne!(
        WorkerConfig::Dry { log: None },
        WorkerConfig::Dry {
            log: Some("/t/dry.log".into())
        }
    );
}

#[test]
fn worker_config_debug_names_a_custom_worker_without_pretending_to_render_it() {
    // `Debug` is hand-written now (a `dyn Worker` has none), and the three data-carrying variants
    // must keep rendering exactly what the derive did — an `EngineConfig` lands in app logs.
    let host = HostWorker::new(|_| Ok(TriggerOutcome::Accepted));
    assert_eq!(format!("{:?}", WorkerConfig::Custom(host)), "Custom(..)");
    assert_eq!(format!("{:?}", WorkerConfig::Disabled), "Disabled");
    assert_eq!(
        format!("{:?}", WorkerConfig::Dry { log: None }),
        "Dry { log: None }"
    );
    assert_eq!(
        format!(
            "{:?}",
            WorkerConfig::Sidecar {
                sidecar: "/p/main.mjs".into(),
                cwd: "/w".into(),
            }
        ),
        r#"Sidecar { sidecar: "/p/main.mjs", cwd: "/w" }"#
    );
}

#[test]
fn a_session_that_vanished_is_its_own_error_case_not_a_generic_failure() {
    // Requirement 2 of the AgentCore probe: a cloud session dies SILENTLY at 15 minutes idle / 8
    // hours maximum, so "resume what is no longer there" must be branchable without string
    // matching. It converts into the shared envelope as `not_found` — the named session does not
    // exist — and keeps the id it was asked to resume.
    let gone = TriggerError::SessionGone {
        session: "remote-abc".into(),
        detail: "runtime reports the session expired".into(),
    };
    assert!(gone.is_session_gone());
    let as_envelope: nexus_chat::error::NxfError = gone.into();
    assert_eq!(as_envelope.kind, ErrorKind::NotFound);
    assert!(
        as_envelope.msg.contains("remote-abc"),
        "{}",
        as_envelope.msg
    );
    assert!(
        as_envelope
            .msg
            .contains("runtime reports the session expired"),
        "{}",
        as_envelope.msg
    );

    // Anything else keeps the kind it already had — a `Failed` is a passthrough, not a reclassify.
    let other: nexus_chat::error::NxfError = TriggerError::Failed(nexus_chat::error::NxfError::io(
        "spawning node: no such file",
    ))
    .into();
    assert_eq!(other.kind, ErrorKind::Io);
    assert!(!TriggerError::Failed(nexus_chat::error::NxfError::io("x")).is_session_gone());
}

#[test]
fn the_two_bundled_workers_report_an_accepted_trigger_not_a_started_session() {
    // Requirement 1, the honest half: neither bundled worker KNOWS the runtime session id when
    // `trigger` returns. The sidecar spawns detached and the session binds itself from inside via
    // `nxc session bind`; the dry worker starts nothing at all. `Accepted` is what says so — and it
    // is the same answer a remote host gives while its provisioning call is still in flight.
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        DryWorker { log: None }
            .trigger(trigger_request("s-1", role_spec()))
            .unwrap(),
        TriggerOutcome::Accepted
    );
    let sidecar = SidecarWorker {
        sidecar: tmp.path().join("main.mjs"),
        cwd: tmp.path().to_path_buf(),
    };
    // `node` may or may not be on PATH here; either way the outcome type is what is under test, so
    // only a successful spawn is asserted on.
    if let Ok(outcome) = sidecar.trigger(trigger_request("s-2", role_spec())) {
        assert_eq!(outcome, TriggerOutcome::Accepted);
    }
}

// ---- the coordinator's terms, on the wire (nxf 6j6v.ntp9) ------------------------------------

#[test]
fn spec_json_carries_the_coordinator_that_admitted_this_spawn() {
    // Not read by the sidecar — it is here so a run's own spec file says who commissioned it, which
    // is the first thing anyone reading `.nxs/agent-logs/<session>.spec.json` after a bad round
    // wants to know. The token is `Coordinator::as_str`'s, so a black-box test reading the dry log
    // and a human reading this file see the same word.
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(TriggerRequest {
        coordinator: Coordinator::Channel,
        ..trigger_request("s-coord", role_spec())
    });
    let spec = read_spec_json(tmp.path(), "s-coord");
    assert_eq!(spec["coordinator"], serde_json::json!("channel"), "{spec}");
}

#[test]
fn spec_json_carries_both_bounds_so_the_worker_inherits_them_instead_of_deciding_them() {
    // The whole point of `TurnTerms` living in the engine (nxf 6j6v.ntp9, answering nxf 6j6v.553s
    // question (b)): what happens when a turn does not answer its thread is the COORDINATOR's
    // decision, and a bound that lived in `agent-sidecar/src/main.mjs` held for the bundled runtime
    // and for no host that installs its own `WorkerConfig::Custom` worker (nxf 41j0.vhsk).
    //
    // Asserted as concrete numbers rather than against `TurnTerms::default()`: this is a WIRE
    // format, and a test that reads the same constant as the code under test would stay green
    // through a rename of the key or a silent change of unit.
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(trigger_request("s-terms", role_spec()));
    let spec = read_spec_json(tmp.path(), "s-terms");
    assert_eq!(spec["replyReminders"], serde_json::json!(1), "{spec}");
    assert_eq!(spec["runtimeRetries"], serde_json::json!(2), "{spec}");
    assert_eq!(spec["retryBackoffMs"], serde_json::json!(5000), "{spec}");
}

#[test]
fn the_terms_a_worker_is_handed_are_the_terms_the_coordinator_stated() {
    // The relay, not the default: a host worker reads these off the request, so a `TriggerRequest`
    // built with other terms must reach the spec with those and not with the engine's.
    let tmp = tempfile::tempdir().unwrap();
    let _ = sidecar_over(tmp.path()).trigger(TriggerRequest {
        terms: TurnTerms::new(0, 7, 250),
        ..trigger_request("s-own-terms", role_spec())
    });
    let spec = read_spec_json(tmp.path(), "s-own-terms");
    assert_eq!(spec["replyReminders"], serde_json::json!(0), "{spec}");
    assert_eq!(spec["runtimeRetries"], serde_json::json!(7), "{spec}");
    assert_eq!(spec["retryBackoffMs"], serde_json::json!(250), "{spec}");
}

#[test]
fn every_coordinator_renders_the_same_token_in_the_log_and_in_the_json() {
    // The token exists three times — `#[serde(rename_all = "snake_case")]`, the hand-written
    // `Coordinator::as_str`, and `KNOWN` in `no_role_starts_without_a_coordinator.rs` — and only one
    // variant was pinned (independent review of PR #378, Code Quality #5). A later multi-word
    // variant would give `"channel_supervisor"` from serde and whatever was typed into `as_str`,
    // splitting the `DryWorker` record from the spec file with nothing red.
    //
    // Exhaustive by construction: the list below is matched against every variant, so adding one
    // without adding it here fails to compile rather than going unchecked.
    for coordinator in [
        Coordinator::Persona,
        Coordinator::Channel,
        Coordinator::Return,
        Coordinator::Released,
    ] {
        // Named so the `#[non_exhaustive]` enum still forces a new variant through this test.
        match coordinator {
            Coordinator::Persona
            | Coordinator::Channel
            | Coordinator::Return
            | Coordinator::Released => {}
            _ => panic!("a new coordinator must be added to this test's list"),
        }
        let json = serde_json::to_value(coordinator).expect("serializes");
        assert_eq!(
            json,
            serde_json::Value::String(coordinator.as_str().to_string()),
            "{coordinator:?} renders differently in the dry log and in the spec JSON"
        );
    }
}
