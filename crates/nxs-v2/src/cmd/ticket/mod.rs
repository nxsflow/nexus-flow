//! `nxs ticket` (RFC 0002, "Commands, JSON and the seam"): one noun and a verb. `create` writes a
//! ticket through `Engine::ticket_create`, `show` reads one through `Engine::ticket`, and both
//! print the ticket as one record ([`record`]).

use std::io::Read;
use std::path::Path;

use clap::Subcommand;
use nexus_flow_seam::{DataDirError, Options};

use crate::output::Format;
use crate::Outcome;

mod create;
mod record;
mod show;

/// The verbs of `nxs ticket`.
#[derive(Debug, Subcommand)]
pub enum TicketCommand {
    /// Create a ticket and mint its key; `-` reads the whole ticket as JSON from stdin.
    Create(Box<create::CreateArgs>),
    /// Show one ticket, by key or object id.
    Show(show::ShowArgs),
}

/// Runs `nxs ticket <verb>` in `dir`, with `stdin` for what `create` reads from it.
pub fn run(
    dir: &Path,
    options: impl FnOnce() -> Result<Options, DataDirError>,
    command: TicketCommand,
    format: Format,
    stdin: &mut dyn Read,
) -> Outcome {
    let printed = match command {
        TicketCommand::Create(args) => create::run(dir, options, *args, stdin).map(|c| match c {
            create::Created::Key(key) => format!("{key}\n"),
            create::Created::Ticket(record) => format.render(&*record),
        }),
        TicketCommand::Show(args) => show::run(dir, options, args).map(|r| format.render(&r)),
    };
    match printed {
        Ok(stdout) => Outcome {
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
            code: 0,
        },
        Err(e) => e.outcome(format),
    }
}
