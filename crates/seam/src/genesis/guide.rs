//! The genesis guide, `factory/genesis-guide.md` (RFC 0004, "The guide"): nexus-flow's text,
//! installed under RFC 0006's rules for shipped defaults. The repository's file is compared with
//! every version `templates/genesis/shipped.toml` lists, after CRLF becomes LF:
//!
//! - `written`: nothing is at the path; the current guide is written with an exclusive create.
//! - `current`: the file is the current guide; nothing is done.
//! - `older_default`: the file is an earlier shipped guide; nothing is done without `update`.
//! - `updated`: the same, with `update`; the current guide replaces it.
//! - `team`: anything else, a symbolic link or a folder at the path included; never touched.
//! - `retired`: the file is present and its path is retired; never touched.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use super::{FileReport, FileState, GenesisError};
use crate::install::shipped::{Shipped, State};

/// The current guide, embedded at build time.
pub const GUIDE: &str = include_str!("../../../../templates/genesis/genesis-guide.md");

/// Every version of the guide nexus-flow has shipped, in RFC 0006's `shipped.toml` format.
pub const SHIPPED: &str = include_str!("../../../../templates/genesis/shipped.toml");

/// The guide's path, relative to the repository's root.
pub const GUIDE_PATH: &str = "factory/genesis-guide.md";

/// The guide's path in `shipped.toml`, relative to the repository's `factory/`.
const SHIPPED_PATH: &str = "genesis-guide.md";

/// Installs the guide `current` in the repository at `root` by the versions `shipped` lists, and
/// reports its state; `None` when the path is retired and nothing is there. `update` replaces an
/// earlier shipped guide. Nothing but a regular file is ever read or replaced: a symbolic link,
/// dangling or not, or a folder at the path is the team's, and nothing is followed.
pub fn install_guide(
    root: &Path,
    shipped: &Shipped,
    current: &str,
    update: bool,
) -> Result<Option<FileReport>, GenesisError> {
    let path = root.join(GUIDE_PATH);
    let io = |source| GenesisError::Io {
        path: path.clone(),
        source,
    };
    let report = |state| {
        Some(FileReport {
            path: GUIDE_PATH.to_owned(),
            state: FileState::Shipped(state),
            found: None,
        })
    };
    let file = match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => Some(fs::read(&path).map_err(io)?),
        Ok(_) if shipped.is_retired(SHIPPED_PATH) => return Ok(report(State::Retired)),
        Ok(_) => return Ok(report(State::Team)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(io(e)),
    };
    let Some(state) = shipped.state(SHIPPED_PATH, file.as_deref(), update) else {
        return Ok(None);
    };
    match state {
        State::Written => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|source| GenesisError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut created) => created.write_all(current.as_bytes()).map_err(io)?,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    return install_guide(root, shipped, current, false)
                }
                Err(e) => return Err(io(e)),
            }
        }
        State::Updated => fs::write(&path, current).map_err(io)?,
        State::Current | State::OlderDefault | State::Team | State::Retired => {}
    }
    Ok(report(state))
}
