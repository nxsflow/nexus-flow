//! `nxs sync key`, `nxs sync trust list|add|remove` and `nxs sync verify` — whose signed ops this
//! workspace believes (nxf 6j6v.pzkb; spec `docs/specs/E4-auth-identity-and-signed-ops.md` §2.3).
//!
//! Thin: every verb is one substrate call on the workspace's store — the same call an embedding
//! app makes through `nexus_flow_facade::engine::Engine` (`key_id`, `trusted_keys`, `trust_key`,
//! `distrust_key`, `untrusted_signers`, `op_provenance`) — plus its rendering. The list is local to
//! this replica and none of it is synced; there is deliberately no MCP tool for any of it, because
//! an agent's instruction must not be able to widen whom a machine believes.
//!
//! None of these needs the workspace to be bound to a stream: the key exists from the first open,
//! and a list can be prepared before the first pull.

use crate::error::{NxfError, Result};
use nexus_flow_core::store::Store;
use nxs_foundation::signing::Provenance;
use nxs_foundation::workspace::Workspace;

fn open(ws: &Workspace) -> Result<Store> {
    let path = ws.db_path_str()?;
    Store::open(&path, ws.replica.site_id)
        .map_err(|e| NxfError::io(format!("opening store at {path}: {e}")))
}

/// `nxs sync key`: this replica's key id — the string another machine adds to believe this one.
pub fn key_verb(json: bool, ws: &Workspace) -> Result<()> {
    let store = open(ws)?;
    let key_id = store.key_id();
    if json {
        println!("{}", serde_json::json!({ "ok": true, "key_id": key_id }));
    } else {
        println!("{key_id}");
        println!(
            "On the machine that should believe this one, in the same workspace — after comparing \
             this string over a channel you trust, not through the relay:\n  \
             nxs sync trust add {key_id} --name <this machine>"
        );
    }
    Ok(())
}

/// `nxs sync trust list`: the keys this workspace believes, and the keys that signed ops here
/// without being on the list.
pub fn list_verb(json: bool, ws: &Workspace) -> Result<()> {
    let store = open(ws)?;
    let trusted = store.trusted_keys();
    let untrusted = store.untrusted_signers();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "trusted": trusted,
                "untrusted_signers": untrusted,
            })
        );
        return Ok(());
    }
    println!("trusted — ops these keys sign may carry agent actions here:");
    for key in &trusted {
        let who = if key.own {
            "this replica".to_string()
        } else if key.name.is_empty() {
            "(unnamed)".to_string()
        } else {
            key.name.clone()
        };
        let added = if key.added.is_empty() {
            String::new()
        } else {
            format!(", added {}", key.added)
        };
        // The receiver's view of a relay that drops signatures (6j6v.pzkb): a machine you trust
        // whose ops arrive, but none of them signed, looks exactly like this.
        let signed = if key.own {
            String::new()
        } else if key.verified_ops == 0 {
            ", no op it signed has arrived here yet — if that machine's changes do arrive, the \
             relay is dropping signatures (upgrade it to nxs 0.58 or later)"
                .to_string()
        } else {
            format!(", {} verified op(s)", key.verified_ops)
        };
        println!("  {}  {who}{added}{signed}", key.key_id);
    }
    if !untrusted.is_empty() {
        println!(
            "signed ops here from keys NOT trusted — shown on the board, no agent action follows \
             them. The authors are what the ops claim; compare the key with its owner before \
             adding it:"
        );
        for signer in &untrusted {
            println!(
                "  {}  {} op(s), signed as {}",
                signer.key_id,
                signer.ops,
                signer.authors.join(", ")
            );
        }
    }
    Ok(())
}

/// `nxs sync trust add <key-id> [--name <name>]`.
pub fn add_verb(
    json: bool,
    ws: &Workspace,
    key_id: &str,
    name: Option<&str>,
    now: &str,
) -> Result<()> {
    let mut store = open(ws)?;
    let added = store.trust_key(key_id, name.unwrap_or(""), now)?;
    let key_id = key_id.trim();
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "key_id": key_id,
                "name": name.map(str::trim).unwrap_or(""),
                "added": added,
            })
        );
    } else if added {
        println!(
            "trusted {key_id} — the ops it signs, past ones included, may now carry agent actions \
             in this workspace"
        );
    } else {
        println!("{key_id} was already trusted");
    }
    Ok(())
}

/// `nxs sync trust remove <key-id>` — revocation, effective at once.
pub fn remove_verb(json: bool, ws: &Workspace, key_id: &str) -> Result<()> {
    let mut store = open(ws)?;
    let removed = store.distrust_key(key_id)?;
    let key_id = key_id.trim();
    if json {
        println!(
            "{}",
            serde_json::json!({ "ok": true, "key_id": key_id, "removed": removed })
        );
    } else if removed {
        println!(
            "no longer trusted: {key_id} — its ops stay on the board, and none of them carries an \
             agent action from now on"
        );
    } else {
        println!("{key_id} was not on the trust list");
    }
    Ok(())
}

/// The op a chat message was folded from — every message row carries the coordinate of its op, and
/// a coordinate names one op (6j6v.fc5p). `None` when there is no such message, and in a workspace
/// where chat was never active (no `messages` table to ask).
fn op_of_message(store: &Store, message_id: &str) -> Option<String> {
    use rusqlite::OptionalExtension;
    store
        .connection()
        .query_row(
            "SELECT o.op_id FROM messages m JOIN ops o ON o.lamport = m.lamport AND o.site = m.site
              WHERE m.message_id = ?1 LIMIT 1",
            [message_id],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()
}

/// `nxs sync verify <id>`: where one op came from, and whether an agent action may follow it. The
/// id is an op id, or a chat message id (`nxc` prints those), which stands for the op behind it.
pub fn verify_verb(json: bool, ws: &Workspace, id: &str) -> Result<()> {
    let store = open(ws)?;
    let seen = store
        .op_provenance(id)
        .or_else(|| op_of_message(&store, id).and_then(|op_id| store.op_provenance(&op_id)))
        .ok_or_else(|| {
            NxfError::not_found(format!("no op and no message {id} in this workspace's log"))
        })?;
    if json {
        let mut value = serde_json::to_value(&seen).expect("OpProvenance serializes");
        value["ok"] = serde_json::Value::Bool(true);
        println!("{value}");
        return Ok(());
    }
    let key = seen.key_id.as_deref().unwrap_or("no key");
    let verdict = match seen.provenance {
        Provenance::Own => "written by this replica".to_string(),
        Provenance::Verified => match (&seen.trusted_as, seen.trusted) {
            (Some(name), true) => format!("signed by {key}, trusted as {name}"),
            (None, true) => format!("signed by {key}, trusted"),
            _ => format!("signed by {key}, which this workspace does not trust"),
        },
        Provenance::Invalid => format!(
            "carries a signature for {key} that does NOT check out — altered on the way, or never \
             signed by that key"
        ),
        Provenance::Unsigned => "carries no signature".to_string(),
    };
    println!("op {}: {verdict}", seen.op_id);
    println!("author, as the op names it: {}", seen.author);
    if seen.acts {
        println!("an agent action may follow it");
    } else {
        println!("no agent action follows it — it is kept and shown, nothing more");
    }
    Ok(())
}
