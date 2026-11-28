//! What the tests of the crates above the store need from NoCentralDB and nexus-flow cannot do
//! yet, behind the feature `test-support`: the invitations of members that later tickets start
//! (a paired pm, a role session, another person's device), and a device that is no longer an
//! admin of its space, and a space whose every ticket tag is held. Only the store may name the SDK
//! (NEX-89), so these live here.
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

/// Writes one role record per ticket tag in the repository space of `repo`, each naming the
/// device's own member, so that every tag is held and `nxs member add` finds none free.
pub async fn hold_every_tag(repo: &Path, data: &DataDir, keys: DeviceKeys) {
    hold_every_tag_but(repo, data, keys, None).await;
}

/// Writes one role record per ticket tag but `but` in the repository space of `repo`, each naming
/// the device's own member, so that every other tag is held.
pub async fn hold_every_tag_but(repo: &Path, data: &DataDir, keys: DeviceKeys, but: Option<&str>) {
    let opened = crate::opened::repository(repo, data, keys)
        .await
        .expect("the repository's spaces");
    let (id, space) = &opened.spaces()[0];
    let space = space.as_ref().expect("the repository space is held");
    let me = space
        .members()
        .list()
        .await
        .expect("the members")
        .into_iter()
        .find(|m| m.is_self)
        .expect("the device is a member")
        .id
        .to_string();
    opened
        .write(id, |w| {
            for tag in crate::members::tags::all().filter(|t| Some(t.as_str()) != but) {
                w.create(
                    "spaceMember",
                    [
                        ("member", nocentraldb::Value::from(me.as_str())),
                        ("tag", nocentraldb::Value::from(tag.as_str())),
                    ],
                )?;
            }
            Ok(())
        })
        .await
        .expect("the records are written");
}

/// Writes a second ticket with the key `key` into the repository space of the checkout at
/// `repo`, as two members holding one tag leave it (RFC 0002, "Two members with one tag"), and
/// returns its key as `nxs` shows it, with `~` and the first digits of its object id.
pub async fn ticket_with_key(repo: &Path, data: &DataDir, keys: DeviceKeys, key: &str) -> String {
    use nocentraldb::Value;

    let opened = crate::opened::repository(repo, data, keys)
        .await
        .expect("the repository opens");
    let (space, _) = &opened.spaces()[0];
    let mut twin = None;
    opened
        .write(space, |w| {
            let values = [
                ("key", Value::from(key)),
                ("type", Value::from("chore")),
                ("title", Value::from("The twin")),
                ("description", Value::from("A key two tickets hold")),
                ("status", Value::from("open")),
            ];
            twin = Some(w.create(crate::tickets::keys::TICKETS, values)?);
            Ok(())
        })
        .await
        .expect("the twin is written");
    let twin = twin.expect("the twin is created");
    crate::tickets::show(&opened, &twin.to_string())
        .await
        .expect("the twin reads")
        .key
}

/// Makes the device of the checkout at `repo` take over the tag `zz` after a member that is no
/// longer one held it: a record of that removed member carries `zz`, and the device is retagged
/// to it, so it mints only once its copy has synced (RFC 0002, "Ticket ids").
pub async fn take_over_freed_tag(repo: &Path, data: &DataDir, keys: DeviceKeys) {
    use nocentraldb::Value;

    const GONE: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    let opened = crate::opened::repository(repo, data, keys)
        .await
        .expect("the repository opens");
    let (space, _) = &opened.spaces()[0];
    opened
        .write(space, |w| {
            let values = [
                ("member", Value::from(GONE)),
                ("role", Value::from("coder")),
                ("tag", Value::from("zz")),
            ];
            w.create(crate::members::record::COLLECTION, values)
                .map(drop)
        })
        .await
        .expect("the removed member's record is written");
    let me = crate::members::list(&opened)
        .await
        .expect("the members list")
        .remove(0)
        .member;
    crate::members::retag(&opened, &me, "zz")
        .await
        .expect("the device takes zz over");
}

/// Clears the tag of the device's own record in the repository space of the checkout at
/// `repo`, so it has no tag to mint a key with.
pub async fn clear_device_tag(repo: &Path, data: &DataDir, keys: DeviceKeys) {
    use nocentraldb::Value;

    let opened = crate::opened::repository(repo, data, keys)
        .await
        .expect("the repository opens");
    let me = crate::members::list(&opened)
        .await
        .expect("the members list")
        .remove(0)
        .member;
    let (space, _) = &opened.spaces()[0];
    let record = opened
        .find(
            space,
            crate::members::record::COLLECTION,
            "byMember",
            vec![Value::from(me)],
        )
        .await
        .expect("the device's record is found")
        .remove(0)
        .id();
    opened
        .write(space, |w| {
            w.update(
                crate::members::record::COLLECTION,
                record,
                [("tag", Value::clear())],
            )
        })
        .await
        .expect("the tag is cleared");
}
