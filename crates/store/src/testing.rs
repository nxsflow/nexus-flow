//! What the tests of the crates above the store need from NoCentralDB and nexus-flow cannot do
//! yet, behind the feature `test-support`: the invitations of members that later tickets start
//! (a paired pm, a role session, another person's device), and a device that is no longer an
//! admin of its space. Only the store may name the SDK (NEX-89), so these live here.
//!
//! Every database here lives in a folder the caller names, with its keys in a file there: a test
//! machine has no key store.

use std::path::Path;

use nocentraldb::{Db, OpenOptions, PairKind, Role};

use crate::data_dir::DataDir;
use crate::id::SpaceId;
use crate::model;
use crate::open::{open_db, DeviceKeys};

/// Opens a new member's database in `folder`, with its keys in a file there.
fn options(folder: &Path) -> OpenOptions {
    OpenOptions::new(model::LOCK)
        .path(folder.join("db"))
        .key_file(folder.join("member.keys"))
}

/// Pairs a new agent, in `folder`, into the account of the device whose database is in `data`,
/// and returns the agent's invitation, which that account certified: the invitation pm hands the
/// device for `nxs member add --role pm` (RFC 0001, "Starting pm").
pub async fn paired_agent(data: &DataDir, keys: DeviceKeys, folder: &Path) -> String {
    let device = open_db(data, keys, None)
        .await
        .expect("the device's database");
    let agent = Db::open(options(folder).pairing().agent())
        .await
        .expect("the agent's database");
    let paired = device
        .account()
        .pair(PairKind::Agent, &agent.invitation().await)
        .await
        .expect("the device pairs the agent");
    agent
        .account()
        .complete_pairing(&paired.answer)
        .await
        .expect("the agent completes its pairing");
    agent.invitation().await.to_string()
}

/// The invitation of a sub-agent in `folder`, without a certificate: a role session's, which its
/// parent agent signs when it adds it (RFC 0001, "Starting a role session").
pub async fn sub_agent(folder: &Path) -> String {
    let db = Db::open(options(folder).sub_agent())
        .await
        .expect("the sub-agent's database");
    db.invitation().await.to_string()
}

/// The invitation of a device in `folder` with an account of its own, which the device in a
/// test's data directory has never seen: another person's device.
pub async fn other_device(folder: &Path) -> String {
    let db = Db::open(options(folder))
        .await
        .expect("the other device's database");
    db.invitation().await.to_string()
}

/// Creates a space that the device whose database is in `data` is the only admin of, such as a
/// coordination space, and returns its id.
pub async fn create_space(data: &DataDir, keys: DeviceKeys) -> SpaceId {
    let device = open_db(data, keys, None)
        .await
        .expect("the device's database");
    let space = device.spaces().create().await.expect("a new space");
    SpaceId(space.id())
}

/// Lowers the device's own role in `space` to writer: it no longer adds or removes members. The
/// SDK refuses to demote a space's last admin, so the agent of `admin`, an invitation of the
/// device's own account, is made an admin first.
pub async fn demote_device(data: &DataDir, keys: DeviceKeys, space: SpaceId, admin: &str) {
    let device = open_db(data, keys, None)
        .await
        .expect("the device's database");
    let space = device.spaces().get(space.0).await.expect("the space");
    let admin = admin.parse().expect("an invitation");
    space
        .members()
        .add(
            &admin,
            nocentraldb::AddMember {
                role: Role::Admin,
                fingerprint: None,
            },
        )
        .await
        .expect("the second admin is added");
    let me = space
        .members()
        .list()
        .await
        .expect("the members")
        .into_iter()
        .find(|m| m.is_self)
        .expect("the device is a member")
        .id;
    space
        .members()
        .set_role(me, Role::Writer)
        .await
        .expect("the device lowers its own role");
}
