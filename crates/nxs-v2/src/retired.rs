//! The retired v1 entry points (RFC 0008, "The installed base"). v1 installs `nxf`, `nxm` and
//! `nxc` as symlinks to `nxs`, so an install of v2 leaves them pointing at v2's `nxs`. Started
//! under one of those names, `nxs` runs no command: it says that the name is retired in v2 and
//! names the `nxs` command that takes its place, as the error `retired_entry_point`.
//!
//! The counterparts follow the RFCs that replace each v1 module: `nxf`'s tickets are
//! `nxs ticket` (RFC 0002), `nxm`'s memories are `nxs decision` (RFC 0003), and `nxc`'s chat
//! lapses, with the factory taking its place, `nxs factory` (RFC 0006).

use std::ffi::OsStr;
use std::path::Path;

use crate::error::{CliError, Kind};

/// The retired names, with what `nxs` says of each.
const RETIRED: &[(&str, &str)] = &[
    ("nxf", "nxf is retired in v2; its counterpart is nxs ticket"),
    (
        "nxm",
        "nxm is retired in v2; its counterpart is nxs decision",
    ),
    (
        "nxc",
        "nxc is retired in v2, and v1's chat with it; its counterpart is nxs factory",
    ),
];

/// The refusal for a process started as `argv0`, or `None` when `argv0` is no retired name.
/// Only the file name counts, so a symlink started by its full path is refused as well.
pub fn check(argv0: &OsStr) -> Option<CliError> {
    let name = Path::new(argv0).file_name()?.to_str()?;
    let name = name
        .strip_suffix(std::env::consts::EXE_SUFFIX)
        .unwrap_or(name);
    RETIRED
        .iter()
        .find(|(retired, _)| *retired == name)
        .map(|(_, msg)| CliError {
            kind: Kind::RetiredEntryPoint,
            msg: (*msg).to_owned(),
        })
}
