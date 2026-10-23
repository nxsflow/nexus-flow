//! `nxf-fold-purge`: the command line over [`super::purge_stream`] (design §3, §5).
//!
//! Non-interactive: the operation as arguments, the configuration from the environment only, with
//! no defaults, no config file, no prompt, no port. Whatever happens — a usage error, a missing
//! variable, a refused id, a table that is not a board-fold table, no deny table — one JSON report
//! goes to stdout and the exit code comes from [`super::exit_code`].
//!
//! The environment: `NXF_FOLD_DDB_TABLE`, `NXF_FOLD_DDB_REGION` and `NXS_DENY_TABLE` required;
//! `NXF_FOLD_DDB_ENDPOINT` optional; `NXS_LOG_HMAC_KEY` optional (log-id v1; malformed is 78 and
//! never echoed); credentials from the AWS SDK's standard chain. Every variable is read before the
//! first request, so a missing one costs no request.

use std::io::Write;
use std::time::{Duration, SystemTime};

use aws_sdk_dynamodb::Client;

use super::{
    exit_code, purge_stream_with, refusal, sid_value, validate_purge_id, LogKey, PurgeError,
    PurgeMode, PurgeOptions, PurgeOutcome, PurgeSeams, Sha256Hex, KEY_ENV,
};
use crate::ddb::{check_fold_table, DynamoDbDenyList, DynamoDbTable};
use crate::deny::DENY_TABLE_ENV;

/// One parsed invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    pub stream: String,
    pub mode: PurgeMode,
    pub expect_identity: Option<Sha256Hex>,
}

const USAGE: &str = "usage: nxf-fold-purge <stream-id> --dry-run [--expect-empty] \
                     [--expect-identity <hex>] | nxf-fold-purge <stream-id> --yes --plan <hex> \
                     [--expect-identity <hex>]";

/// Parse the arguments. Exactly one of the three invocations, or a usage error (64) that reads
/// nothing. `--keys` is not accepted (design round 2b): the fold's sort keys carry user text, and no
/// per-key output is printed in any form. `--` ends the flags, for an id that starts with `-`.
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
                    Err(usage("a flag given twice"))
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
                _ => return Err(usage("unknown flag")),
            }
        } else if stream.is_some() {
            return Err(usage("exactly one stream id per call"));
        } else {
            stream = Some(arg.clone());
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

/// The purge as a command: parse, read the environment through `env`, check both tables, run,
/// print the report on `out` and progress on `err`. Returns the exit code.
pub fn run(
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    out: &mut dyn Write,
    err: &mut (dyn Write + Send),
) -> u8 {
    let started = SystemTime::now();
    let usage_mode = PurgeMode::DryRun {
        expect_empty: false,
    };
    let invocation = match parse_args(args) {
        Ok(i) => i,
        Err(e) => {
            let mut outcome = refusal(usage_mode, "-", e, started, None);
            outcome.report.mode = "usage";
            return finish(outcome, out, err);
        }
    };
    let mode = invocation.mode;
    // C1: a malformed key is a configuration error; the message never echoes the value.
    let log_key = match env(KEY_ENV).map(|v| LogKey::parse(&v)).transpose() {
        Ok(key) => key,
        Err(e) => {
            let e = PurgeError::Config(e.to_string());
            return finish(refusal(mode, "-", e, started, None), out, err);
        }
    };
    let sid = sid_value(log_key.as_ref(), &invocation.stream);
    if let Err(e) = validate_purge_id(&invocation.stream) {
        return finish(refusal(mode, &sid, e, started, None), out, err);
    }
    let config = match Config::read(env) {
        Ok(c) => c,
        Err(e) => return finish(refusal(mode, &sid, e, started, None), out, err),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => {
            let e = PurgeError::Table("the async runtime could not start".into());
            return finish(refusal(mode, &sid, e, started, None), out, err);
        }
    };
    #[cfg(feature = "test-seams")]
    let seams = PurgeSeams::from_env();
    #[cfg(not(feature = "test-seams"))]
    let seams = PurgeSeams::shipped();
    let opts = PurgeOptions {
        expect_identity: invocation.expect_identity,
        log_key,
    };
    let outcome = runtime.block_on(async {
        let client = client_at(&config, config.endpoint.clone()).await;
        let identity = match check_fold_table(&client, &config.table).await {
            Ok(identity) => identity,
            Err(e) => return refusal(mode, &sid, e, started, None),
        };
        // The pin before the deny table (design §4 step 1): a mismatch is IdentityMismatch, even
        // where the deny table is wrong too.
        if opts
            .expect_identity
            .is_some_and(|pin| pin != identity.digest())
        {
            let e = PurgeError::IdentityMismatch;
            return refusal(mode, &sid, e, started, Some(&identity));
        }
        let deny = match DynamoDbDenyList::open(client.clone(), Some(&config.deny_table)).await {
            Ok(deny) => deny,
            Err(e) => return refusal(mode, &sid, e.into(), started, Some(&identity)),
        };
        let table = DynamoDbTable::new(client, &config.table, &invocation.stream);
        #[cfg(feature = "test-seams")]
        let inject_into = match env("NXF_FOLD_TEST_PURGE_INJECT_ENDPOINT").filter(|v| !v.is_empty())
        {
            Some(endpoint) => DynamoDbTable::new(
                client_at(&config, Some(endpoint)).await,
                &config.table,
                &invocation.stream,
            ),
            None => table.clone(),
        };
        #[cfg(not(feature = "test-seams"))]
        let inject_into = table.clone();
        let clock = SystemTime::now;
        let mut progress = |line: &str| {
            let _ = writeln!(err, "{line}");
        };
        purge_stream_with(
            &table,
            &inject_into,
            &identity,
            &deny,
            mode,
            &opts,
            &seams,
            &clock,
            &mut progress,
        )
        .await
    });
    finish(outcome, out, err)
}

/// What the environment names.
struct Config {
    table: String,
    region: String,
    deny_table: String,
    endpoint: Option<String>,
}

impl Config {
    /// Every variable, before any request. No variable has a default.
    fn read(env: &dyn Fn(&str) -> Option<String>) -> Result<Config, PurgeError> {
        let required = |name: &str| match env(name) {
            Some(v) if !v.is_empty() => Ok(v),
            _ => Err(PurgeError::Config(format!(
                "{name} is not set; the purge reads its tables from the environment and has no \
                 defaults"
            ))),
        };
        let table = required("NXF_FOLD_DDB_TABLE")?;
        let region = required("NXF_FOLD_DDB_REGION")?;
        let deny_table = match env(DENY_TABLE_ENV) {
            Some(v) if !v.is_empty() => v,
            _ => {
                return Err(PurgeError::NotADenyTable(
                    crate::ddb::DenyTableError::Unset.to_string(),
                ))
            }
        };
        Ok(Config {
            table,
            region,
            deny_table,
            endpoint: env("NXF_FOLD_DDB_ENDPOINT").filter(|v| !v.is_empty()),
        })
    }
}

/// A client for the region, at `endpoint` when one is given; the standard credential chain, and the
/// relay's request timeouts (5 s an attempt, 15 s an operation), so a black-holed endpoint ends the
/// run with exit 1 instead of hanging it.
async fn client_at(config: &Config, endpoint: Option<String>) -> Client {
    let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_sdk_dynamodb::config::Region::new(config.region.clone()))
        .timeout_config(
            aws_sdk_dynamodb::config::timeout::TimeoutConfig::builder()
                .operation_attempt_timeout(Duration::from_secs(5))
                .operation_timeout(Duration::from_secs(15))
                .build(),
        );
    if let Some(endpoint) = endpoint {
        loader = loader.endpoint_url(endpoint);
    }
    Client::new(&loader.load().await)
}

fn finish(outcome: PurgeOutcome, out: &mut dyn Write, err: &mut (dyn Write + Send)) -> u8 {
    let code = exit_code(&outcome);
    if let Some(e) = &outcome.error {
        if outcome.report.store.is_none() || outcome.report.plan_digest.is_none() {
            let _ = writeln!(
                err,
                "nxf-fold-purge: sid={} {} (exit {code})",
                outcome.report.sid,
                e.kind()
            );
        }
    }
    let text = serde_json::to_string_pretty(&outcome.report).expect("the report serializes");
    let _ = writeln!(out, "{text}");
    code
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
        assert_eq!(dry.stream, "n-h-1");
        assert_eq!(
            dry.mode,
            PurgeMode::DryRun {
                expect_empty: false
            }
        );
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
            assert_eq!(
                parse(args).unwrap().expect_identity,
                Sha256Hex::parse(PIN),
                "{args:?}"
            );
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
        usage(&["n-h-1", "n-h-2", "--dry-run"]);
        usage(&["n-h-1", "--dry-run", "--verbose"]);
    }

    #[test]
    fn a_double_dash_ends_the_flags() {
        assert_eq!(
            parse(&["--dry-run", "--", "-odd-id"]).unwrap().stream,
            "-odd-id"
        );
        usage(&["--dry-run", "--", "a", "b"]);
    }
}
