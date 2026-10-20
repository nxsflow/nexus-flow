//! `nxf-relay purge`: the command line over [`super::purge_stream`] (design §2 C, §6).
//!
//! Non-interactive: the operation as arguments, the configuration from the environment only, with
//! no defaults, no config file, no prompt. Whatever happens — a usage error, a missing variable, a
//! refused id, a store that is not a relay store — one JSON report goes to stdout and the exit
//! code comes from [`super::exit_code`].

use std::io::Write;
use std::sync::Arc;
use std::time::SystemTime;

use nxs_sync::protocol::StreamId;

use super::{
    exit_code, new_report, purge_stream_with, sid_value, validate_purge_id, ErrorReport, PurgeDeps,
    PurgeError, PurgeMode, PurgeOptions, PurgeOutcome, PurgeSeams, PurgeStore, RefusedReport,
    Sha256Hex,
};
use crate::log_id::{LogKey, KEY_ENV};

/// One parsed invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub stream: StreamId,
    pub mode: PurgeMode,
    pub expect_identity: Option<Sha256Hex>,
}

const USAGE: &str = "usage: nxf-relay purge <stream-id> --dry-run [--expect-empty] \
                     [--expect-identity <hex>] | nxf-relay purge <stream-id> --yes --plan <hex> \
                     [--expect-identity <hex>]";

/// Parse the arguments after `purge`. Exactly one of the three invocations, or a usage error
/// (64) that reads nothing. `--keys` was removed in design round 3 and is refused like any other
/// unknown flag. `--` ends the flags, for an id that starts with `-`.
pub fn parse_args(args: &[String]) -> Result<Invocation, PurgeError> {
    let usage = |why: &str| PurgeError::Usage(format!("{why}; {USAGE}"));
    let digest = |flag: &str, value: Option<&String>| -> Result<Sha256Hex, PurgeError> {
        value
            .and_then(|v| Sha256Hex::parse(v))
            .ok_or_else(|| usage(&format!("{flag} takes 64 hex characters")))
    };
    let (mut dry_run, mut yes, mut expect_empty) = (false, false, false);
    let (mut plan, mut expect_identity, mut stream) = (None, None, None);
    let mut it = args.iter();
    let mut flags_done = false;
    while let Some(arg) = it.next() {
        if !flags_done && arg.starts_with('-') {
            let once = |seen: bool| {
                if seen {
                    Err(usage(&format!("{arg} given twice")))
                } else {
                    Ok(true)
                }
            };
            match arg.as_str() {
                "--" => flags_done = true,
                "--dry-run" => dry_run = once(dry_run)?,
                "--yes" => yes = once(yes)?,
                "--expect-empty" => expect_empty = once(expect_empty)?,
                "--plan" => {
                    once(plan.is_some())?;
                    plan = Some(digest("--plan", it.next())?);
                }
                "--expect-identity" => {
                    once(expect_identity.is_some())?;
                    expect_identity = Some(digest("--expect-identity", it.next())?);
                }
                // `--keys` printed op and registry keys until design round 3; it is gone.
                _ => return Err(usage("unknown flag")),
            }
        } else if stream.is_some() {
            return Err(usage("exactly one stream id per call"));
        } else {
            stream = Some(StreamId(arg.clone()));
        }
    }
    let stream = stream.ok_or_else(|| usage("no stream id"))?;
    let mode = match (dry_run, yes, plan) {
        (true, false, None) => PurgeMode::DryRun { expect_empty },
        (false, true, Some(plan)) if !expect_empty => PurgeMode::Delete { plan },
        _ => return Err(usage("not one of the three invocations")),
    };
    Ok(Invocation {
        stream,
        mode,
        expect_identity,
    })
}

/// The purge as a command: parse, read the environment through `env`, open the store, run, print
/// the report on `out` and progress on `err`. Returns the exit code.
pub fn run(
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let started = SystemTime::now();
    let invocation = match parse_args(args) {
        Ok(i) => i,
        Err(e) => {
            return finish(
                refusal(
                    PurgeMode::DryRun {
                        expect_empty: false,
                    },
                    "-",
                    e,
                    started,
                    "usage",
                ),
                out,
                err,
            )
        }
    };
    // C1: a malformed key is a configuration error; the message never echoes the value.
    let log_key = match env(KEY_ENV).map(|v| LogKey::parse(&v)).transpose() {
        Ok(key) => key,
        Err(e) => {
            let e = PurgeError::Config(e.to_string());
            return finish(refusal(invocation.mode, "-", e, started, ""), out, err);
        }
    };
    let sid = sid_value(log_key.as_ref(), &invocation.stream);
    if let Err(e) = validate_purge_id(&invocation.stream) {
        let mut outcome = refusal(invocation.mode, &sid, e, started, "");
        if let Some(PurgeError::InvalidStreamId { rule }) = &outcome.error {
            outcome.report.refused = Some(RefusedReport { rule });
        }
        return finish(outcome, out, err);
    }
    let read_only = matches!(invocation.mode, PurgeMode::DryRun { .. });
    let store = match open_store(env, read_only) {
        Ok(store) => store,
        Err(e) => return finish(refusal(invocation.mode, &sid, e, started, ""), out, err),
    };
    #[cfg(feature = "test-seams")]
    let seams = PurgeSeams::from_env();
    #[cfg(not(feature = "test-seams"))]
    let seams = PurgeSeams::shipped();
    let deps = PurgeDeps { store };
    let opts = PurgeOptions {
        expect_identity: invocation.expect_identity,
        log_key,
    };
    let clock = SystemTime::now;
    let outcome = purge_stream_with(
        &deps,
        &invocation.stream,
        invocation.mode,
        &opts,
        &seams,
        &clock,
        &mut |line: &str| {
            let _ = writeln!(err, "{line}");
        },
    );
    finish(outcome, out, err)
}

/// An outcome for a run that stopped before the purge itself started.
fn refusal(
    mode: PurgeMode,
    sid: &str,
    error: PurgeError,
    started: SystemTime,
    mode_label: &'static str,
) -> PurgeOutcome {
    let mut report = new_report(mode, sid.to_string(), started);
    if !mode_label.is_empty() {
        report.mode = mode_label;
    }
    report.error = Some(ErrorReport {
        kind: error.kind(),
        message: error.to_string(),
    });
    PurgeOutcome {
        report,
        error: Some(error),
    }
}

fn finish(outcome: PurgeOutcome, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let code = exit_code(&outcome);
    if let Some(e) = &outcome.error {
        if outcome.report.store.is_none() {
            let _ = writeln!(err, "nxf-relay purge: {e} (exit {code})");
        }
    }
    let text = serde_json::to_string_pretty(&outcome.report).expect("the report serializes");
    let _ = writeln!(out, "{text}");
    code
}

/// The variable `name`, or a configuration error naming it.
fn required(env: &dyn Fn(&str) -> Option<String>, name: &str) -> Result<String, PurgeError> {
    match env(name) {
        Some(v) if !v.is_empty() => Ok(v),
        _ => Err(PurgeError::Config(format!(
            "{name} is not set; the purge reads its store from the environment and has no defaults"
        ))),
    }
}

/// The store the environment names, opened for this mode. No variable has a default.
fn open_store(
    env: &dyn Fn(&str) -> Option<String>,
    read_only: bool,
) -> Result<Arc<dyn PurgeStore>, PurgeError> {
    let backend = required(env, "NXF_RELAY_BACKEND")?;
    match backend.as_str() {
        "sqlite" => {
            let ops = required(env, "NXF_RELAY_DB")?;
            let registry = required(env, "NXF_RELAY_REGISTRY_DB")?;
            Ok(Arc::new(super::sqlite::SqlitePurgeStore::open(
                &ops, &registry, read_only,
            )?))
        }
        #[cfg(feature = "postgres")]
        "postgres" => {
            let url = required(env, "NXF_RELAY_PG_URL")?;
            Ok(Arc::new(super::pg::PostgresPurgeStore::connect(&url)?))
        }
        #[cfg(feature = "dynamodb")]
        "dynamodb" => {
            let ops = required(env, "NXF_RELAY_DDB_TABLE")?;
            let registry = required(env, "NXF_RELAY_DDB_REGISTRY_TABLE")?;
            let region = required(env, "NXF_RELAY_DDB_REGION")?;
            let endpoint = env("NXF_RELAY_DDB_ENDPOINT").filter(|v| !v.is_empty());
            Ok(Arc::new(super::ddb::DynamoDbPurgeStore::new(
                ops, registry, region, endpoint,
            )))
        }
        // Only reached for a backend whose arm above this build was compiled without.
        other if other == "postgres" || other == "dynamodb" => Err(PurgeError::Config(format!(
            "NXF_RELAY_BACKEND={backend} is not in this build (it needs a source build with \
             `--features {backend}`)"
        ))),
        _ => Err(PurgeError::Config(
            "NXF_RELAY_BACKEND must be one of sqlite, postgres, dynamodb".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "9f2c0000000000000000000000000000000000000000000000000000000000aa";
    const PIN: &str = "a41c0000000000000000000000000000000000000000000000000000000000bb";

    fn parse(args: &[&str]) -> Result<Invocation, PurgeError> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        parse_args(&args)
    }

    fn usage(args: &[&str]) {
        match parse(args) {
            Err(PurgeError::Usage(_)) => {}
            other => panic!("{args:?} must be a usage error, got {other:?}"),
        }
    }

    #[test]
    fn the_three_invocations_parse() {
        let dry = parse(&["n-h-1", "--dry-run"]).unwrap();
        assert_eq!(dry.stream.as_str(), "n-h-1");
        assert_eq!(
            dry.mode,
            PurgeMode::DryRun {
                expect_empty: false
            }
        );
        assert_eq!(dry.expect_identity, None);

        let confirm = parse(&["n-h-1", "--dry-run", "--expect-empty"]).unwrap();
        assert_eq!(confirm.mode, PurgeMode::DryRun { expect_empty: true });

        let yes = parse(&["n-h-1", "--yes", "--plan", DIGEST]).unwrap();
        assert_eq!(
            yes.mode,
            PurgeMode::Delete {
                plan: Sha256Hex::parse(DIGEST).unwrap()
            }
        );

        for args in [
            &["n-h-1", "--dry-run", "--expect-identity", PIN][..],
            &[
                "--expect-identity",
                PIN,
                "n-h-1",
                "--dry-run",
                "--expect-empty",
            ][..],
            &["n-h-1", "--plan", DIGEST, "--yes", "--expect-identity", PIN][..],
        ] {
            let parsed = parse(args).unwrap();
            assert_eq!(parsed.expect_identity, Sha256Hex::parse(PIN), "{args:?}");
        }
    }

    #[test]
    fn every_other_combination_is_a_usage_error() {
        usage(&[]);
        usage(&["n-h-1"]);
        usage(&["--dry-run"]);
        usage(&["n-h-1", "--dry-run", "--yes"]);
        usage(&["n-h-1", "--yes"]);
        usage(&["n-h-1", "--plan", DIGEST]);
        usage(&["n-h-1", "--dry-run", "--plan", DIGEST]);
        usage(&["n-h-1", "--yes", "--plan", DIGEST, "--expect-empty"]);
        usage(&["n-h-1", "--expect-empty"]);
        usage(&["n-h-1", "--dry-run", "--keys"]);
        usage(&["n-h-1", "--yes", "--plan", DIGEST, "--keys"]);
        usage(&["n-h-1", "--dry-run", "--dry-run"]);
        usage(&["n-h-1", "--yes", "--plan", DIGEST, "--plan", DIGEST]);
        usage(&["n-h-1", "--yes", "--plan"]);
        usage(&["n-h-1", "--yes", "--plan", "abc"]);
        usage(&["n-h-1", "--dry-run", "--expect-identity"]);
        usage(&["n-h-1", "--dry-run", "--expect-identity", "xyz"]);
        usage(&["n-h-1", "n-h-2", "--dry-run"]);
        usage(&["n-h-1", "--dry-run", "--verbose"]);
        usage(&["n-h-1", "-n"]);
    }

    #[test]
    fn a_double_dash_ends_the_flags() {
        let parsed = parse(&["--dry-run", "--", "-odd-id"]).unwrap();
        assert_eq!(parsed.stream.as_str(), "-odd-id");
        usage(&["--dry-run", "--", "a", "b"]);
    }

    fn cli(args: &[&str], vars: &[(&str, &str)]) -> (u8, serde_json::Value, String) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let vars: std::collections::HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let env = move |name: &str| vars.get(name).cloned();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&args, &env, &mut out, &mut err);
        let report = serde_json::from_slice(&out).expect("one JSON report on stdout");
        (code, report, String::from_utf8(err).unwrap())
    }

    #[test]
    fn a_usage_error_prints_a_report_and_exits_64() {
        let (code, report, _) = cli(&["n-h-1", "--keys"], &[]);
        assert_eq!(code, 64);
        assert_eq!(report["error"]["kind"], "Usage");
        assert_eq!(report["mode"], "usage");
    }

    #[test]
    fn every_required_variable_is_named_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("relay.sqlite");
        let db = db.to_str().unwrap();
        for (vars, missing) in [
            (vec![], "NXF_RELAY_BACKEND"),
            (vec![("NXF_RELAY_BACKEND", "sqlite")], "NXF_RELAY_DB"),
            (
                vec![("NXF_RELAY_BACKEND", "sqlite"), ("NXF_RELAY_DB", db)],
                "NXF_RELAY_REGISTRY_DB",
            ),
        ] {
            let (code, report, _) = cli(&["n-h-1", "--dry-run"], &vars);
            assert_eq!(code, 78, "{missing}");
            assert!(
                report["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains(missing),
                "{report}"
            );
        }
        assert!(!std::path::Path::new(db).exists(), "nothing was created");
    }

    #[test]
    fn a_malformed_key_is_78_and_never_echoed() {
        let bad = "t1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
        let (code, report, err) = cli(
            &["n-h-1", "--dry-run"],
            &[("NXS_LOG_HMAC_KEY", bad), ("NXF_RELAY_BACKEND", "sqlite")],
        );
        assert_eq!(code, 78);
        assert!(!report.to_string().contains("AAECAwQFBgcICQoL"));
        assert!(!err.contains("AAECAwQFBgcICQoL"));
    }

    #[test]
    fn a_refused_id_is_64_with_its_rule_before_the_environment_is_read() {
        let (code, report, _) = cli(&["#due", "--dry-run"], &[]);
        assert_eq!(code, 64);
        assert_eq!(report["refused"]["rule"], "leading_hash");
    }
}
