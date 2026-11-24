//! v2's `nxs` on the library seam (RFC 0001, "Reading and writing", "Public surfaces this RFC
//! brings"): [`run`] parses a command line, runs it through the seam and returns what the process
//! prints and its exit code, so that a test runs `nxs` in its own process with the options it
//! names. `main.rs` calls it with the environment's options.
//!
//! Output is JSON by default and Markdown with `--md` (`output`); an error is v1's error object
//! (`error`).

use std::ffi::OsString;
use std::path::Path;

use clap::{Parser, Subcommand};
use nexus_flow_seam::{DataDirError, Options};

mod block_on;
mod cmd;
mod error;
mod output;

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
    /// Create the repository space and nexus-flow.toml, or name the space the file holds.
    Init,
    /// Add, remove and list the members of the repository's spaces, with their role records.
    #[command(subcommand)]
    Member(cmd::member::MemberCommand),
    /// Sync this device's database with its relay, report its sync state, or set its relay.
    Sync(cmd::sync::SyncArgs),
    /// Write the repository's GENESIS from the template, only where none exists.
    #[command(subcommand)]
    Genesis(cmd::genesis::GenesisCommand),
}

/// Runs `nxs` with `args`, the program's name first, in the folder `dir`. `options` gives the
/// data directory and the device's keys; it is called only by a command that opens the database.
pub fn run<I, T>(
    args: I,
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
) -> Outcome
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(e) => return parse_error(e, &args),
    };
    let format = if cli.md { Format::Md } else { Format::Json };
    let result = match cli.command {
        Command::Init => cmd::init::run(dir, options).map(|r| format.render(&r)),
        Command::Member(command) => {
            cmd::member::run(dir, options, command).map(|r| format.render(&r))
        }
        Command::Sync(args) => cmd::sync::run(dir, options, args).map(|r| format.render(&r)),
        Command::Genesis(command) => return cmd::genesis::run(dir, command, format),
    };
    match result {
        Ok(stdout) => Outcome {
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
            code: 0,
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
    let md = args.iter().skip(1).any(|a| a == "--md");
    let format = if md { Format::Md } else { Format::Json };
    CliError::usage(&e).outcome(format)
}
