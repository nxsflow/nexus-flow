//! **Who wrote this op** — the replica's signing key, and the check that an op's signature holds
//! (nxf 6j6v.pzkb, the first slice of 6j6v.6aza; the design is
//! `docs/specs/E4-auth-identity-and-signed-ops.md`).
//!
//! # The key hangs on the replica
//!
//! One Ed25519 key per replica — one `.nxs/` workspace on one machine — kept in
//! `.nxs/signing.key` beside the db, readable by its owner only. The substrate [`Store`] opens it
//! itself, so every writer of the workspace signs through the same key without anybody handing one
//! around, and a validly signed op copied into ANOTHER workspace's stream arrives there from a key
//! nobody in that workspace trusts. Why the replica and not the machine is the spec's §2.1.
//!
//! # The key id is the public key
//!
//! `ed25519:<base64url of the 32-byte public key>`. Every receiver can check every signature with
//! no directory to consult; whether it BELIEVES the key is the trust list's question
//! ([`crate::trust`]), asked only when an action is decided.
//!
//! [`Store`]: crate::store::Store

use std::io::Write;
use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::model::Op;

/// The file the replica key lives in, inside the directory that holds the workspace db.
pub const KEY_FILE: &str = "signing.key";

/// What every key id starts with — the algorithm, so a later one can sit beside it.
pub const KEY_ID_PREFIX: &str = "ed25519:";

/// The first word of the key file: its format, so a later one can be told apart.
const KEY_FILE_TAG: &str = "nxs-signing-key/1";

/// A replica's signing key. The secret never leaves this struct: no accessor returns it, `Debug`
/// prints the key id only, and `ed25519-dalek` wipes it on drop.
pub struct ReplicaKey {
    signing: SigningKey,
    key_id: String,
}

impl std::fmt::Debug for ReplicaKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplicaKey")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl ReplicaKey {
    /// A fresh key from the operating system's generator. Used for an in-memory store, which signs
    /// like any other and whose signatures nobody else can have reason to trust.
    pub fn generate() -> ReplicaKey {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("the operating system's random generator is available");
        ReplicaKey::from_seed(seed)
    }

    fn from_seed(seed: [u8; 32]) -> ReplicaKey {
        let signing = SigningKey::from_bytes(&seed);
        let key_id = key_id_of(&signing.verifying_key());
        ReplicaKey { signing, key_id }
    }

    /// This key's id — `ed25519:<base64url public key>`, the string another replica trusts.
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    /// The signature over `bytes`, base64url.
    pub fn sign(&self, bytes: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(self.signing.sign(bytes).to_bytes())
    }

    /// Sign `op` in place: its [`key_id`](Op::key_id) becomes this key's and its [`sig`](Op::sig)
    /// the signature over its [canonical bytes](Op::canonical_bytes).
    pub fn seal(&self, op: &mut Op) {
        op.key_id = Some(self.key_id.clone());
        op.sig = Some(self.sign(&op.canonical_bytes()));
    }

    /// The key in `dir`/[`KEY_FILE`], created on the first ask.
    ///
    /// **Concurrent first asks agree on ONE key**: the new key is written in full to a private
    /// temporary file and then hard-linked into place, which fails rather than replaces when the
    /// file already exists — so a loser reads the winner's complete file, never a half-written one,
    /// and never overwrites it. A key that changed under a replica would orphan every op it signed
    /// before (its own ops would stop being its own), which is also why a file that cannot be read
    /// is an error rather than a reason to mint a new one.
    ///
    /// The file is created `0600` on Unix, and a key file found readable by others is narrowed back
    /// to `0600` — it is a secret, and a copied `.nxs/` may have picked up a wider mode.
    pub fn load_or_create(dir: &Path) -> std::io::Result<ReplicaKey> {
        let path = dir.join(KEY_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                narrow_permissions(&path);
                return parse_key_file(&text).ok_or_else(|| malformed(&path));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let key = ReplicaKey::generate();
        let tmp = dir.join(format!(".{KEY_FILE}.{}", ulid::Ulid::new()));
        {
            let mut file = create_private(&tmp)?;
            let seed = URL_SAFE_NO_PAD.encode(key.signing.to_bytes());
            file.write_all(format!("{KEY_FILE_TAG} {seed}\n").as_bytes())?;
            file.sync_all()?;
        }
        let linked = std::fs::hard_link(&tmp, &path);
        let _ = std::fs::remove_file(&tmp);
        match linked {
            Ok(()) => Ok(key),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let text = std::fs::read_to_string(&path)?;
                parse_key_file(&text).ok_or_else(|| malformed(&path))
            }
            Err(e) => Err(e),
        }
    }
}

fn malformed(path: &Path) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!(
            "{} is not a signing key this nxs can read — restore it rather than delete it: a new \
             key would make this replica's own earlier ops foreign to it",
            path.display()
        ),
    )
}

fn parse_key_file(text: &str) -> Option<ReplicaKey> {
    let mut words = text.split_whitespace();
    if words.next() != Some(KEY_FILE_TAG) {
        return None;
    }
    let seed: [u8; 32] = URL_SAFE_NO_PAD
        .decode(words.next()?)
        .ok()?
        .try_into()
        .ok()?;
    if words.next().is_some() {
        return None;
    }
    Some(ReplicaKey::from_seed(seed))
}

#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(unix)]
fn narrow_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.permissions().mode() & 0o077 != 0 {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}

#[cfg(not(unix))]
fn narrow_permissions(_path: &Path) {}

fn key_id_of(key: &VerifyingKey) -> String {
    format!("{KEY_ID_PREFIX}{}", URL_SAFE_NO_PAD.encode(key.to_bytes()))
}

/// The public key a key id names, or `None` when it names none — wrong prefix, not base64url, not
/// 32 bytes, or not a point on the curve.
pub fn parse_key_id(key_id: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = URL_SAFE_NO_PAD
        .decode(key_id.strip_prefix(KEY_ID_PREFIX)?)
        .ok()?
        .try_into()
        .ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

/// Whether `sig` is a valid signature of the key `key_id` names over `bytes`. Strict verification:
/// it refuses the small-order keys and non-canonical signatures under which Ed25519 would let one
/// signature verify for more than one message or key.
pub fn verify(key_id: &str, sig: &str, bytes: &[u8]) -> bool {
    let Some(key) = parse_key_id(key_id) else {
        return false;
    };
    let Some(sig) = URL_SAFE_NO_PAD
        .decode(sig)
        .ok()
        .and_then(|b| <[u8; 64]>::try_from(b).ok())
    else {
        return false;
    };
    key.verify_strict(bytes, &Signature::from_bytes(&sig))
        .is_ok()
}

/// What a replica knows about where an op came from — the verdict stored beside every op in the
/// log (`ops.provenance`), and the half of the question "does an action follow this op?" that
/// does not change over time. The other half, whether the key is trusted, is asked at the moment of
/// the action ([`crate::trust`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    /// This replica appended it — signed with its own key, or written here before signing existed.
    Own,
    /// Received, and its signature checks out against the key its `key_id` names.
    Verified,
    /// Received with a signature that does not check out, or with half a signature pair — altered
    /// on the way, or never signed by the key it names.
    Invalid,
    /// Received without any signature — an older client, or a relay that dropped it.
    Unsigned,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::Own => "own",
            Provenance::Verified => "verified",
            Provenance::Invalid => "invalid",
            Provenance::Unsigned => "unsigned",
        }
    }

    pub fn parse(s: &str) -> Option<Provenance> {
        match s {
            "own" => Some(Provenance::Own),
            "verified" => Some(Provenance::Verified),
            "invalid" => Some(Provenance::Invalid),
            "unsigned" => Some(Provenance::Unsigned),
            _ => None,
        }
    }

    /// The verdict on a RECEIVED op, from the op alone. Never [`Own`](Provenance::Own): that one is
    /// given only by the append that wrote the op here, never inferred from anything an op carries.
    pub fn of_received(op: &Op) -> Provenance {
        match (&op.key_id, &op.sig) {
            (None, None) => Provenance::Unsigned,
            (Some(key_id), Some(sig)) if verify(key_id, sig, &op.canonical_bytes()) => {
                Provenance::Verified
            }
            _ => Provenance::Invalid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op() -> Op {
        Op {
            op_id: "01HFAKE0000000000000000001".into(),
            lamport: 7,
            site: 42,
            domain: "task".into(),
            target_kind: "item".into(),
            target_id: "ab12.7x3k".into(),
            field: "title".into(),
            op_type: "set".into(),
            value: Some("hello".into()),
            author: "alice".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        }
    }

    #[test]
    fn a_sealed_op_verifies_and_every_change_to_it_does_not() {
        let key = ReplicaKey::generate();
        let mut sealed = op();
        key.seal(&mut sealed);
        assert_eq!(sealed.key_id.as_deref(), Some(key.key_id()));
        assert_eq!(Provenance::of_received(&sealed), Provenance::Verified);

        type Tamper = (&'static str, fn(&mut Op));
        let tampered: [Tamper; 11] = [
            ("op_id", |o| o.op_id.push('x')),
            ("lamport", |o| o.lamport += 1),
            ("site", |o| o.site += 1),
            ("domain", |o| o.domain = "fact".into()),
            ("target_kind", |o| o.target_kind.push('x')),
            ("target_id", |o| o.target_id.push('x')),
            ("field", |o| o.field.push('x')),
            ("op_type", |o| o.op_type.push('x')),
            ("value", |o| o.value = Some("hellO".into())),
            ("author", |o| o.author = "mallory".into()),
            ("wall_clock", |o| o.wall_clock.push('x')),
        ];
        for (what, change) in tampered {
            let mut altered = sealed.clone();
            change(&mut altered);
            assert_eq!(
                Provenance::of_received(&altered),
                Provenance::Invalid,
                "a changed {what} no longer verifies"
            );
        }
    }

    #[test]
    fn an_op_without_its_signature_is_unsigned_and_half_a_pair_is_invalid() {
        let key = ReplicaKey::generate();
        let mut sealed = op();
        key.seal(&mut sealed);

        let mut stripped = sealed.clone();
        stripped.key_id = None;
        stripped.sig = None;
        assert_eq!(Provenance::of_received(&stripped), Provenance::Unsigned);

        let mut no_sig = sealed.clone();
        no_sig.sig = None;
        assert_eq!(Provenance::of_received(&no_sig), Provenance::Invalid);
        let mut no_key = sealed;
        no_key.key_id = None;
        assert_eq!(Provenance::of_received(&no_key), Provenance::Invalid);
    }

    #[test]
    fn a_signature_does_not_verify_under_another_key() {
        let (mine, theirs) = (ReplicaKey::generate(), ReplicaKey::generate());
        let mut sealed = op();
        mine.seal(&mut sealed);
        sealed.key_id = Some(theirs.key_id().to_string());
        assert_eq!(Provenance::of_received(&sealed), Provenance::Invalid);
    }

    #[test]
    fn garbage_in_the_signature_pair_is_invalid_not_a_panic() {
        for (key_id, sig) in [
            ("", ""),
            ("ed25519:", "x"),
            ("rsa:AAAA", "AAAA"),
            ("ed25519:not base64!", "AAAA"),
            ("ed25519:AAAA", "AAAA"),
        ] {
            let mut o = op();
            o.key_id = Some(key_id.into());
            o.sig = Some(sig.into());
            assert_eq!(Provenance::of_received(&o), Provenance::Invalid, "{key_id}");
        }
        let key = ReplicaKey::generate();
        let mut o = op();
        o.key_id = Some(key.key_id().into());
        o.sig = Some(URL_SAFE_NO_PAD.encode([0u8; 64]));
        assert_eq!(Provenance::of_received(&o), Provenance::Invalid);
    }

    #[test]
    fn a_key_id_is_the_prefixed_public_key_and_parses_back() {
        let key = ReplicaKey::generate();
        assert!(key.key_id().starts_with(KEY_ID_PREFIX));
        assert_eq!(key.key_id().len(), KEY_ID_PREFIX.len() + 43);
        assert!(parse_key_id(key.key_id()).is_some());
        assert!(parse_key_id("ed25519:short").is_none());
        assert!(parse_key_id(&key.key_id()[KEY_ID_PREFIX.len()..]).is_none());
        assert!(
            !format!("{key:?}").contains(&URL_SAFE_NO_PAD.encode(key.signing.to_bytes())),
            "Debug never prints the secret"
        );
    }

    #[test]
    fn the_key_file_is_created_once_private_and_read_back_as_the_same_key() {
        let dir = tempfile::tempdir().unwrap();
        let first = ReplicaKey::load_or_create(dir.path()).unwrap();
        let again = ReplicaKey::load_or_create(dir.path()).unwrap();
        assert_eq!(first.key_id(), again.key_id(), "durable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "owner-only");
        }
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            leftovers,
            [std::ffi::OsString::from(KEY_FILE)],
            "no temp file left"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_key_file_readable_by_others_is_narrowed_back_to_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        ReplicaKey::load_or_create(dir.path()).unwrap();
        let path = dir.path().join(KEY_FILE);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        ReplicaKey::load_or_create(dir.path()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn concurrent_first_asks_agree_on_one_key() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        ReplicaKey::load_or_create(dir.path())
                            .unwrap()
                            .key_id()
                            .to_string()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(ids.windows(2).all(|w| w[0] == w[1]), "{ids:?}");
    }

    #[test]
    fn a_key_file_that_cannot_be_read_is_an_error_not_a_new_key() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(KEY_FILE), "not a key\n").unwrap();
        let err = ReplicaKey::load_or_create(dir.path()).expect_err("refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(KEY_FILE)).unwrap(),
            "not a key\n",
            "and left as it was"
        );
    }
}
