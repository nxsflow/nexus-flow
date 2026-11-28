//! The merge gate on the seam (RFC 0010, "Public surfaces this RFC brings"):
//! [`Engine::merge_gate_rules`] and [`Engine::merge_gate_decide`], the counterparts of
//! `nxs merge-gate rules` and `nxs merge-gate decide`, and the types of their reports.
//!
//! Both read only the local git, from the repository's root as `Engine::open` finds it, and the
//! inputs they are given, and open no database, so they run on a machine without a key store, such
//! as the gate's workflow.

use std::path::Path;

use nexus_flow_merge_gate::decide::{decide, DecideError, Inputs};
use nexus_flow_merge_gate::git::{read_top_file, resolve_commit};
use nexus_flow_store::config::Config;
use nexus_flow_store::repo_root::RepoRoot;
use nexus_flow_store::space::InitError;

pub use nexus_flow_merge_gate::decide::{DecideReport, InputError, Reason, Status};
pub use nexus_flow_merge_gate::gate_toml::{Effect, RuleRef};
pub use nexus_flow_merge_gate::rules::{
    GitError, Rule, RulesError, RulesReport, Settings, GATE_TOML_PATH, GENESIS_PATH,
};

use crate::engine::{Engine, Error};
use crate::install::codeowners::{Codeowners, LOCATIONS};

impl Engine {
    /// Every rule the gate judges, read from the commit `base` names, or, without `base`, from the
    /// branch `[git] base` names in `nexus-flow.toml`, for the repository at or above
    /// `repository`. Rules that cannot be read are a report, not an error; a base that names no
    /// commit is an error.
    pub fn merge_gate_rules(repository: &Path, base: Option<&str>) -> Result<RulesReport, Error> {
        let root = RepoRoot::find(repository).map_err(InitError::Root)?;
        let base = match base {
            Some(base) => base.to_string(),
            None => Config::read(&root).map_err(InitError::Config)?.base,
        };
        Ok(nexus_flow_merge_gate::rules::rules_at(root.path(), &base)?)
    }

    /// The gate's decision on the pull request from `base` to `head`, for the repository at or
    /// above `repository`: from the rules at `base`, the verdict found in `comments` or given as
    /// `verdict`, the paths the pull request changes, the CODEOWNERS file at `base`, and `reviews`.
    /// Every decision is a report, `owner` included; it is an error only when it cannot decide:
    /// `base` or `head` names no commit, both `comments` and `verdict` are given, or `reviews` is
    /// not the forge's list of reviews.
    pub fn merge_gate_decide(
        repository: &Path,
        base: &str,
        head: &str,
        comments: Option<&[u8]>,
        verdict: Option<&[u8]>,
        reviews: Option<&[u8]>,
    ) -> Result<DecideReport, Error> {
        let root = RepoRoot::find(repository).map_err(InitError::Root)?;
        let root = root.path();
        let codeowners = codeowners_at(root, &resolve_commit(root, base)?)?;
        let inputs = Inputs {
            comments,
            verdict,
            reviews,
        };
        decide(root, base, head, inputs, |path| {
            codeowners.as_ref().is_some_and(|c| c.covers(path))
        })
        .map_err(|e| match e {
            DecideError::Git(e) => Error::Git(e),
            DecideError::Input(e) => Error::MergeGateInput(e),
        })
    }
}

/// The first CODEOWNERS file at `commit`, in the places and order GitHub looks for it, from the
/// git repository's top; `None` when there is none.
fn codeowners_at(root: &Path, commit: &str) -> Result<Option<Codeowners>, GitError> {
    for location in LOCATIONS {
        if let Some(bytes) = read_top_file(root, commit, location)? {
            return Ok(Some(Codeowners::parse(&String::from_utf8_lossy(&bytes))));
        }
    }
    Ok(None)
}
