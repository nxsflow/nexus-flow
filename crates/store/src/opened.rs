//! The device member's database, open on the spaces a caller reads (RFC 0001, "Reading and
//! writing", "The library seam"): what `Engine::open` and `Engine::open_space` hold.
//!
//! A repository opens on the spaces its `nexus-flow.toml` names, and has a checkout: the root
//! and the base branch "on main" is checked against. A space opened by id has a checkout only
//! when the caller names one, and without it every "on main" value is `null` ("Derived state").
//!
//! The database is the device member's, or a running session's when the caller names one
//! (RFC 0001, "Reading and writing": the member a command runs as); an open opened as the device
//! reopens as a session with [`Opened::as_session`].
//!
//! A named space the database does not hold yet, such as on a device added but not yet synced,
//! is open as not held: no read touches the network, so a read answers from what the database
//! holds, which is nothing of that space.

use std::fmt;
use std::path::{Path, PathBuf};

use nocentraldb::{Db, ErrorCode, Space};

use crate::config::Config;
use crate::data_dir::DataDir;
use crate::id::{SessionId, SpaceId};
use crate::member_select::RunAs;
use crate::open::{open_in, open_member, DeviceKeys, InitError};
use crate::repo_root::RepoRoot;

/// The local git a caller reads "on main" from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The repository's root, the folder that holds `nexus-flow.toml`.
    pub path: PathBuf,
    /// The team's base branch, from `[git] base`.
    pub base: String,
}

/// A member's database, open on some spaces.
pub struct Opened {
    /// Kept open for as long as the spaces are: sync runs only while a database is open
    /// (RFC 0001, "Sync").
    _db: Db,
    session: Option<SessionId>,
    spaces: Vec<(SpaceId, Option<Space>)>,
    checkout: Option<Checkout>,
}

impl Opened {
    /// The session whose member's database is open; `None` for the device's member.
    pub fn session(&self) -> Option<&SessionId> {
        self.session.as_ref()
    }

    /// The same spaces and checkout, opened on the running session `session`'s database in
    /// `data` instead. A session that is not running fails, and nothing is created.
    pub async fn as_session(self, data: &DataDir, session: SessionId) -> Result<Opened, InitError> {
        let ids = self.spaces.into_iter().map(|(id, _)| id).collect();
        let checkout = self.checkout;
        drop(self._db);
        let db = open_member(data, RunAs::Session(session.clone())).await?;
        open(db, Some(session), ids, checkout).await
    }

    /// Whether the space `id` is held: `None` when it is not one of the open spaces, `Some(false)`
    /// when it is but the database holds none of it yet.
    pub fn held(&self, id: &SpaceId) -> Option<bool> {
        self.spaces
            .iter()
            .find(|(open, _)| open == id)
            .map(|(_, space)| space.is_some())
    }

    /// The checkout "on main" is read from; `None` when there is none, so "on main" is `null`.
    pub fn checkout(&self) -> Option<&Checkout> {
        self.checkout.as_ref()
    }
}

impl fmt::Debug for Opened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opened")
            .field("session", &self.session)
            .field("spaces", &self.spaces)
            .field("checkout", &self.checkout)
            .finish_non_exhaustive()
    }
}

/// Opens the device member's database for the repository at or above `dir`, on the spaces its
/// `nexus-flow.toml` names. Outside a repository the error says to run `nxs init`, and nothing
/// is created.
pub async fn repository(dir: &Path, data: &DataDir, keys: DeviceKeys) -> Result<Opened, InitError> {
    repository_as(dir, data, RunAs::Device(keys)).await
}

/// [`repository`], on the database of the member `run_as` names.
pub async fn repository_as(dir: &Path, data: &DataDir, run_as: RunAs) -> Result<Opened, InitError> {
    let root = RepoRoot::find(dir)?;
    let config = Config::read(&root)?;
    let mut ids = vec![parse(&config.repository, &root)?];
    if let Some(coordination) = &config.coordination {
        ids.push(parse(coordination, &root)?);
    }
    let session = session_of(&run_as);
    let db = open_in(&root, data, run_as).await?;
    let checkout = Checkout {
        path: root.path().to_path_buf(),
        base: config.base,
    };
    open(db, session, ids, Some(checkout)).await
}

/// Opens the device member's database on the space `id`. With `git`, the checkout is the
/// repository at or above it, whose `nexus-flow.toml` gives the base branch; without it there
/// is none, and "on main" is `null`.
pub async fn space(
    id: SpaceId,
    git: Option<&Path>,
    data: &DataDir,
    keys: DeviceKeys,
) -> Result<Opened, InitError> {
    space_as(id, git, data, RunAs::Device(keys)).await
}

/// [`space`], on the database of the member `run_as` names.
pub async fn space_as(
    id: SpaceId,
    git: Option<&Path>,
    data: &DataDir,
    run_as: RunAs,
) -> Result<Opened, InitError> {
    let session = session_of(&run_as);
    let (db, checkout) = match git {
        Some(dir) => {
            let root = RepoRoot::find(dir)?;
            let config = Config::read(&root)?;
            let db = open_in(&root, data, run_as).await?;
            let checkout = Checkout {
                path: root.path().to_path_buf(),
                base: config.base,
            };
            (db, Some(checkout))
        }
        None => (open_member(data, run_as).await?, None),
    };
    open(db, session, vec![id], checkout).await
}

fn session_of(run_as: &RunAs) -> Option<SessionId> {
    match run_as {
        RunAs::Device(_) => None,
        RunAs::Session(session) => Some(session.clone()),
    }
}

async fn open(
    db: Db,
    session: Option<SessionId>,
    ids: Vec<SpaceId>,
    checkout: Option<Checkout>,
) -> Result<Opened, InitError> {
    let mut spaces = Vec::with_capacity(ids.len());
    for id in ids {
        let space = match db.spaces().get(id.0).await {
            Ok(space) => Some(space),
            Err(e) if e.code() == ErrorCode::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        spaces.push((id, space));
    }
    Ok(Opened {
        _db: db,
        session,
        spaces,
        checkout,
    })
}

fn parse(text: &str, root: &RepoRoot) -> Result<SpaceId, InitError> {
    text.parse().map_err(|error| InitError::SpaceId {
        file: root.config_file(),
        error,
    })
}
