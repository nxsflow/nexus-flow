//! v2's `nxs` on the library seam (RFC 0001, "Reading and writing", "Public surfaces this RFC
//! brings"): [`run`] parses a command line, runs it through the seam and returns what the process
//! prints and its exit code, so that a test runs `nxs` in its own process with the options it
//! names. `main.rs` calls it with the environment's options.
//!
//! Output is JSON by default and Markdown with `--md` (`output`); an error is v1's error object
//! (`error`). A command that succeeds with a warning, such as `nxs decision add` writing a handle
//! taken already, prints `warning: <text>` on stderr, in both formats.

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;

use clap::{Parser, Subcommand};
use nexus_flow_seam::{DataDirError, Options};

mod block_on;
mod cmd;
mod error;
mod output;
mod retired;
mod v1_notice;

use error::CliError;
use output::Format;

/// What a run of `nxs` prints, and its exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub code: i32,
}

#[derive(Parser)]
#[command(name = "nxs", version, about = "nexus-flow v2")]
struct Cli {
    /// Print Markdown for a person instead of JSON.
    #[arg(long, global = true)]
    md: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a decision with its topics, tickets and the decisions it supersedes.
    #[command(subcommand)]
    Decision(cmd::decision::DecisionCommand),
    /// Create the repository space and nexus-flow.toml, or name the space the file holds.
    Init,
    /// Add, remove and list the members of the repository's spaces, with their role records.
    #[command(subcommand)]
    Member(cmd::member::MemberCommand),
    /// List the repositories of the coordination space.
    #[command(subcommand)]
    Repository(cmd::repository::RepositoryCommand),
    /// Create, join or leave the repository's coordination space.
    #[command(subcommand)]
    Space(cmd::space::SpaceCommand),
    /// Sync this device's database with its relay, report its sync state, or set its relay.
    Sync(cmd::sync::SyncArgs),
    /// Install the factory's shipped defaults under factory/.
    #[command(subcommand)]
    Factory(cmd::factory::FactoryCommand),
    /// Read the merge gate's rules from the base commit.
    MergeGate(cmd::merge_gate::MergeGateArgs),
    /// Create a ticket, or show one.
    #[command(subcommand)]
    Ticket(cmd::ticket::TicketCommand),
    /// Write the repository's GENESIS from the template, only where none exists.
    #[command(subcommand)]
    Genesis(cmd::genesis::GenesisCommand),
}

/// Runs `nxs` with `args`, the program's name first, in the folder `dir`. `options` gives the
/// data directory and the device's keys; it is called only by a command that opens the database.
/// A command that reads stdin, such as `nxs ticket create -`, reads the process's.
pub fn run<I, T>(
    args: I,
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
) -> Outcome
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    run_with_stdin(args, dir, options, &mut std::io::stdin().lock())
}

/// [`run`], with `stdin` as what a command reads from stdin.
pub fn run_with_stdin<I, T>(
    args: I,
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    stdin: &mut dyn Read,
) -> Outcome
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if let Some(e) = args.first().and_then(|argv0| retired::check(argv0)) {
        return e.outcome(md_requested(&args));
    }
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) => return parse_error(e, &args),
    };
    let format = if cli.md { Format::Md } else { Format::Json };
    let mut outcome = dispatch(cli.command, dir, options, format, stdin);
    if let Some(notice) = v1_notice::check(dir) {
        outcome.stderr.extend_from_slice(notice.as_bytes());
    }
    outcome
}

/// Runs a parsed command and returns what it prints, before any notice is added.
fn dispatch(
    command: Command,
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    format: Format,
    stdin: &mut dyn Read,
) -> Outcome {
    let result = match command {
        Command::Ticket(command) => return cmd::ticket::run(dir, options, command, format, stdin),
        Command::Decision(command) => return cmd::decision::run(dir, options, command, format),
        Command::Init => return cmd::init::run(dir, options, format),
        Command::Member(command) => {
            cmd::member::run(dir, options, command).map(|r| (format.render(&r), 0))
        }
        Command::Sync(args) => cmd::sync::run(dir, options, args).map(|r| (format.render(&r), 0)),
        Command::MergeGate(args) => {
            cmd::merge_gate::run(dir, args).map(|r| (format.render(&r), r.code()))
        }
        Command::Genesis(command) => return cmd::genesis::run(dir, command, format),
        Command::Factory(command) => {
            cmd::factory::run(dir, options, command).map(|r| (format.render(&r), 0))
        }
        Command::Repository(command) => {
            cmd::repository::run(dir, options, command).map(|r| (format.render(&r), 0))
        }
        Command::Space(command) => {
            cmd::space::run(dir, options, command).map(|r| (format.render(&r), 0))
        }
    };
    match result {
        Ok((stdout, code)) => Outcome {
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
            code,
        },
        Err(e) => e.outcome(format),
    }
}

/// Help and the version are printed as clap writes them; any other parse error is a usage
/// error, printed as every other error is.
fn parse_error(e: clap::Error, args: &[OsString]) -> Outcome {
    use clap::error::ErrorKind;
    if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
        return Outcome {
            stdout: e.render().to_string().into_bytes(),
            stderr: Vec::new(),
            code: 0,
        };
    }
    CliError::usage(&e).outcome(md_requested(args))
}

/// The format of an error printed before the command line is parsed: Markdown when any argument
/// is `--md`, JSON otherwise.
fn md_requested(args: &[OsString]) -> Format {
    if args.iter().skip(1).any(|a| a == "--md") {
        Format::Md
    } else {
        Format::Json
    }
}
