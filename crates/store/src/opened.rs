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
//! A space's kind comes from its marker (`crate::space_marker`), which the store reads for every
//! open space it holds, so a space opened by id alone has its kind too. A space without a
//! marker, created before the marker existed, takes its kind from `nexus-flow.toml`: the space it
//! names as `repository` or `coordination`, in the repository opened or the checkout named. A
//! space of neither, such as an unmarked one opened by id without a checkout, is of no known
//! kind, and every write checks [`Opened::check_write`] first, so it takes no write of a
//! collection restricted to some kinds.
//!
//! A named space the database does not hold yet, such as on a device added but not yet synced,
//! is open as not held: no read touches the network, so a read answers from what the database
//! holds, which is nothing of that space.

use std::fmt;
use std::path::{Path, PathBuf};

use nocentraldb::{Db, ErrorCode, Space};

use crate::config::Config;
use crate::data_dir::DataDir;
use crate::device::RelayUrl;
use crate::id::{SessionId, SpaceId};
use crate::member_select::RunAs;
use crate::open::{open_in, open_member, DeviceKeys, InitError, Network};
use crate::repo_root::RepoRoot;
use crate::space_kind::{self, SpaceKind, WrongSpace};
use crate::space_marker::{self, Marked};

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
    db: Db,
    /// The data directory it lives in, which holds the device's settings.
    data: DataDir,
    session: Option<SessionId>,
    /// The relay it syncs through; `None` in island mode or when opened off the network.
    relay: Option<RelayUrl>,
    spaces: Vec<(SpaceId, Option<Space>)>,
    /// The kinds `nexus-flow.toml` names, for the spaces it names.
    kinds: Vec<(SpaceId, SpaceKind)>,
    /// What the markers of each held space say of its kind.
    marked: Vec<(SpaceId, Marked)>,
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
        let kinds = self.kinds;
        let checkout = self.checkout;
        drop(self.db);
        let (db, relay) =
            open_member(data, RunAs::Session(session.clone()), Network::Relay).await?;
        let member = Member {
            db,
            data,
            session: Some(session),
            relay,
        };
        open(member, ids, kinds, checkout).await
    }

    /// Whether the space `id` is held: `None` when it is not one of the open spaces, `Some(false)`
    /// when it is but the database holds none of it yet.
    pub fn held(&self, id: &SpaceId) -> Option<bool> {
        self.spaces
            .iter()
            .find(|(open, _)| open == id)
            .map(|(_, space)| space.is_some())
    }

    /// The kind of the open space `id`: the one its marker names, or, for a space without a
    /// marker, the one `nexus-flow.toml` names; `None` when neither names it, or its markers
    /// disagree.
    pub fn kind(&self, id: &SpaceId) -> Option<SpaceKind> {
        self.held(id)?;
        match self.marked.iter().find(|(held, _)| held == id) {
            Some(&(_, Marked::Kind(kind))) => return Some(kind),
            Some((_, Marked::Conflict)) => return None,
            Some((_, Marked::None)) | None => {}
        }
        self.kinds
            .iter()
            .find(|(named, _)| named == id)
            .map(|&(_, kind)| kind)
    }

    /// Whether `collection` may be written into the open space `id` (RFC 0001, "The model"):
    /// every write checks this first.
    pub fn check_write(&self, id: &SpaceId, collection: &str) -> Result<(), WrongSpace> {
        space_kind::check(collection, self.kind(id))
    }

    /// The checkout "on main" is read from; `None` when there is none, so "on main" is `null`.
    pub fn checkout(&self) -> Option<&Checkout> {
        self.checkout.as_ref()
    }

    /// The open database, for sync (`crate::sync`).
    pub(crate) fn db(&self) -> &Db {
        &self.db
    }

    /// The data directory the database lives in.
    pub(crate) fn data(&self) -> &DataDir {
        &self.data
    }

    /// The relay the database syncs through; `None` when it syncs with none.
    pub(crate) fn relay(&self) -> Option<&RelayUrl> {
        self.relay.as_ref()
    }

    /// The open spaces, each with what the database holds of it.
    pub(crate) fn spaces(&self) -> &[(SpaceId, Option<Space>)] {
        &self.spaces
    }

    /// Opens the space `id` beside the others as the repository's coordination space, as
    /// `nexus-flow.toml` will name it once a create or a join has written it; a space already
    /// open stays as it is.
    pub(crate) async fn open_coordination(&mut self, id: SpaceId) -> Result<(), InitError> {
        if self.held(&id).is_none() {
            let space = get(&self.db, id).await?;
            self.spaces.push((id, space));
            self.kinds.push((id, SpaceKind::Coordination));
            self.mark().await?;
        }
        Ok(())
    }

    /// Closes the open space `id`, as a leave does once `nexus-flow.toml` no longer names it.
    pub(crate) fn close(&mut self, id: &SpaceId) {
        self.spaces.retain(|(open, _)| open != id);
        self.kinds.retain(|(named, _)| named != id);
        self.marked.retain(|(held, _)| held != id);
    }

    /// Looks again for the open spaces the database did not hold, which sync may have brought.
    pub(crate) async fn refresh(&mut self) -> Result<(), InitError> {
        for (id, space) in &mut self.spaces {
            if space.is_none() {
                *space = get(&self.db, *id).await?;
            }
        }
        self.mark().await
    }
}

impl Opened {
    /// Reads the markers of every held space.
    async fn mark(&mut self) -> Result<(), InitError> {
        let mut marked = Vec::with_capacity(self.spaces.len());
        for (id, space) in &self.spaces {
            if space.is_some() {
                marked.push((*id, space_marker::read(self, id).await?));
            }
        }
        self.marked = marked;
        Ok(())
    }
}

impl fmt::Debug for Opened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opened")
            .field("session", &self.session)
            .field("relay", &self.relay)
            .field("spaces", &self.spaces)
            .field("kinds", &self.kinds)
            .field("marked", &self.marked)
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
    repository_on(dir, data, run_as, Network::Relay).await
}

/// [`repository_as`], on the network `network` allows.
pub(crate) async fn repository_on(
    dir: &Path,
    data: &DataDir,
    run_as: RunAs,
    network: Network,
) -> Result<Opened, InitError> {
    let root = RepoRoot::find(dir)?;
    let config = Config::read(&root)?;
    let kinds = named_kinds(&config, &root)?;
    let ids = kinds.iter().map(|&(id, _)| id).collect();
    let session = session_of(&run_as);
    let (db, relay) = open_in(&root, data, run_as, network).await?;
    let checkout = Checkout {
        path: root.path().to_path_buf(),
        base: config.base,
    };
    let member = Member {
        db,
        data,
        session,
        relay,
    };
    open(member, ids, kinds, Some(checkout)).await
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
    let ((db, relay), kinds, checkout) = match git {
        Some(dir) => {
            let root = RepoRoot::find(dir)?;
            let config = Config::read(&root)?;
            let kinds = named_kinds(&config, &root)?;
            let db = open_in(&root, data, run_as, Network::Relay).await?;
            let checkout = Checkout {
                path: root.path().to_path_buf(),
                base: config.base,
            };
            (db, kinds, Some(checkout))
        }
        None => (
            open_member(data, run_as, Network::Relay).await?,
            Vec::new(),
            None,
        ),
    };
    let member = Member {
        db,
        data,
        session,
        relay,
    };
    open(member, vec![id], kinds, checkout).await
}

fn session_of(run_as: &RunAs) -> Option<SessionId> {
    match run_as {
        RunAs::Device(_) => None,
        RunAs::Session(session) => Some(session.clone()),
    }
}

/// The spaces `config` names, with their kinds: the repository space first.
fn named_kinds(config: &Config, root: &RepoRoot) -> Result<Vec<(SpaceId, SpaceKind)>, InitError> {
    let mut kinds = vec![(parse(&config.repository, root)?, SpaceKind::Repository)];
    if let Some(coordination) = &config.coordination {
        kinds.push((parse(coordination, root)?, SpaceKind::Coordination));
    }
    Ok(kinds)
}

/// A member's database as it was opened.
struct Member<'a> {
    db: Db,
    data: &'a DataDir,
    session: Option<SessionId>,
    relay: Option<RelayUrl>,
}

async fn open(
    member: Member<'_>,
    ids: Vec<SpaceId>,
    kinds: Vec<(SpaceId, SpaceKind)>,
    checkout: Option<Checkout>,
) -> Result<Opened, InitError> {
    let mut spaces = Vec::with_capacity(ids.len());
    for id in ids {
        spaces.push((id, get(&member.db, id).await?));
    }
    let mut opened = Opened {
        db: member.db,
        data: member.data.clone(),
        session: member.session,
        relay: member.relay,
        spaces,
        kinds,
        marked: Vec::new(),
        checkout,
    };
    opened.mark().await?;
    Ok(opened)
}

/// The space `id`, or `None` while the database holds none of it.
async fn get(db: &Db, id: SpaceId) -> Result<Option<Space>, InitError> {
    match db.spaces().get(id.0).await {
        Ok(space) => Ok(Some(space)),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn parse(text: &str, root: &RepoRoot) -> Result<SpaceId, InitError> {
    text.parse().map_err(|error| InitError::SpaceId {
        file: root.config_file(),
        error,
    })
}
