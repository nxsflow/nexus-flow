//! Log ids, contract **log-id v1** (6j6v.01wt, C1-C10): an identifying value — a stream id, an op
//! id, a replica uuid, a machine id — never reaches a log line or a report in clear. It appears as
//! a keyed, truncated HMAC that only a holder of the current key can recompute, and once the key
//! is destroyed (C7, at the latest 30 days after its first use) nobody can.
//!
//! The same id under the same key gives the same token in every process that implements the
//! contract: this relay, and foundations' engine, authorizer and board-fold. The C9 vectors in the
//! tests below are the proof both sides reproduce.
//!
//! This module is 01wt's unit 1, built by 6j6v.f6yb because the relay purge ships first and
//! prints stream ids only this way. `nxf-relay log-hash` (01wt unit 5, C8) is its command line.

use std::fmt;
use std::io::Write as _;
use std::sync::OnceLock;

/// The environment variable that carries the key (C1).
pub const KEY_ENV: &str = "NXS_LOG_HMAC_KEY";

/// The kinds of id, each its own domain separator and log field name (C3). The table is closed:
/// an identifying value of any other kind is not logged at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A stream id, as the relay receives it in `/streams/<id>/…`.
    Sid,
    /// An op's `op_id`.
    Op,
    /// A replica uuid, as sent to register.
    Rid,
    /// A machine id, as sent in a presence announcement.
    Mid,
    /// An account subject (`sub`), reserved for the authorizer.
    Uid,
}

impl Kind {
    /// The kind's ASCII name: the field name and the domain separator of C2.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Sid => "sid",
            Kind::Op => "op",
            Kind::Rid => "rid",
            Kind::Mid => "mid",
            Kind::Uid => "uid",
        }
    }

    /// The kind with this exact name, or `None`.
    pub fn parse(name: &str) -> Option<Kind> {
        [Kind::Sid, Kind::Op, Kind::Rid, Kind::Mid, Kind::Uid]
            .into_iter()
            .find(|k| k.name() == name)
    }
}

/// Why a key value was refused (C1). Its `Display` names the rule and NEVER the value: a message
/// that echoed a malformed key would put (most of) a secret into a log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// No `:` between the kid and the key.
    MissingSeparator,
    /// More than one `:`.
    SecondSeparator,
    /// The part before the `:` is empty.
    EmptyKid,
    /// The kid is longer than 16 characters.
    KidTooLong,
    /// The kid has a character outside `[a-z0-9]`.
    KidAlphabet,
    /// The key is not exactly 43 characters.
    KeyLength,
    /// The key carries `=` padding.
    Padding,
    /// The key has a character outside the base64url alphabet `[A-Za-z0-9_-]`.
    KeyAlphabet,
    /// The 43 characters do not decode to 32 bytes (non-zero trailing bits).
    KeyDecode,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = match self {
            KeyError::MissingSeparator => "expected <kid>:<key>, found no ':'",
            KeyError::SecondSeparator => "expected exactly one ':'",
            KeyError::EmptyKid => "the kid before ':' is empty",
            KeyError::KidTooLong => "the kid is longer than 16 characters",
            KeyError::KidAlphabet => "the kid may only use [a-z0-9]",
            KeyError::KeyLength => {
                "the key must be exactly 43 base64url characters (32 bytes, no padding)"
            }
            KeyError::Padding => "the key must not carry '=' padding",
            KeyError::KeyAlphabet => "the key must be base64url ([A-Za-z0-9_-])",
            KeyError::KeyDecode => "the key does not decode to exactly 32 bytes",
        };
        write!(f, "{KEY_ENV} is malformed: {rule}")
    }
}

impl std::error::Error for KeyError {}

/// A parsed key: its id and the 32 key bytes. `Debug` shows the kid only.
#[derive(Clone, PartialEq, Eq)]
pub struct LogKey {
    kid: String,
    key: [u8; 32],
}

impl fmt::Debug for LogKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LogKey")
            .field("kid", &self.kid)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl LogKey {
    /// Parse `<kid>:<key>` (C1). Anything else is refused with the rule it broke, never with the
    /// value.
    pub fn parse(value: &str) -> Result<LogKey, KeyError> {
        use base64::Engine as _;

        let mut parts = value.split(':');
        let (kid, key) = match (parts.next(), parts.next(), parts.next()) {
            (Some(kid), Some(key), None) => (kid, key),
            (_, None, _) => return Err(KeyError::MissingSeparator),
            _ => return Err(KeyError::SecondSeparator),
        };
        if kid.is_empty() {
            return Err(KeyError::EmptyKid);
        }
        if kid.len() > 16 {
            return Err(KeyError::KidTooLong);
        }
        if !kid
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return Err(KeyError::KidAlphabet);
        }
        if key.contains('=') {
            return Err(KeyError::Padding);
        }
        if key.len() != 43 {
            return Err(KeyError::KeyLength);
        }
        if !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(KeyError::KeyAlphabet);
        }
        // The no-pad engine refuses non-zero trailing bits, so 43 characters decode to exactly
        // one 32-byte key or to nothing.
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(key)
            .map_err(|_| KeyError::KeyDecode)?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| KeyError::KeyDecode)?;
        Ok(LogKey {
            kid: kid.to_string(),
            key,
        })
    }

    /// The key id, as it appears inside every token.
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The token of C5, `<kind>=h1:<kid>:<hex24>`: C2's HMAC, truncated to 12 bytes (C4).
    pub fn token(&self, kind: Kind, id: &str) -> String {
        format!("{}={}", kind.name(), self.tag(kind, id))
    }

    /// The token's value without the `<kind>=` field name, `h1:<kid>:<hex24>` — what a JSON
    /// document carries under the key `<kind>` (C5).
    pub fn tag(&self, kind: Kind, id: &str) -> String {
        use hmac::{Hmac, Mac};
        use std::fmt::Write as _;

        // C2: HMAC-SHA256(key, "nxs-log-id/v1/" || kind || 0x00 || id), over the id's exact bytes.
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&self.key)
            .expect("HMAC takes a key of any length");
        mac.update(b"nxs-log-id/v1/");
        mac.update(kind.name().as_bytes());
        mac.update(&[0]);
        mac.update(id.as_bytes());
        let tag = mac.finalize().into_bytes();
        // C4: the first 12 bytes as 24 lowercase hex characters.
        let mut hex = String::with_capacity(24);
        for byte in &tag[..12] {
            let _ = write!(hex, "{byte:02x}");
        }
        format!("h1:{}:{hex}", self.kid)
    }
}

/// The process-wide key, set once by [`init`].
static KEY: OnceLock<Option<LogKey>> = OnceLock::new();

/// Set the process-wide key from the variable's value (`None` when unset). The first call decides;
/// a later call changes nothing and answers with what the first one set. A malformed value is
/// refused and leaves nothing set, so the caller can refuse to start (C1).
pub fn init(value: Option<&str>) -> Result<Option<&'static LogKey>, KeyError> {
    let parsed = value.map(LogKey::parse).transpose()?;
    Ok(KEY.get_or_init(|| parsed).as_ref())
}

/// The process-wide key, or `None` when [`init`] was not called or set none.
pub fn key() -> Option<&'static LogKey> {
    KEY.get().and_then(Option::as_ref)
}

/// One id, to be written into a log line as its C5 token — or as `<kind>=-` when no key is set
/// (C6). Infallible: nothing in here can make a log line fail to write.
#[derive(Clone, Copy)]
pub struct LogId<'a> {
    kind: Kind,
    id: &'a str,
    key: Option<&'a LogKey>,
}

impl fmt::Display for LogId<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.key {
            Some(key) => f.write_str(&key.token(self.kind, self.id)),
            None => write!(f, "{}=-", self.kind.name()),
        }
    }
}

impl LogId<'_> {
    /// The id under an explicit key rather than the process-wide one — for a caller that holds
    /// its own (the purge reads the variable itself, so a test can run it without global state).
    pub fn with_key<'a>(kind: Kind, id: &'a str, key: Option<&'a LogKey>) -> LogId<'a> {
        LogId { kind, id, key }
    }
}

/// A stream id for a log line.
pub fn sid(id: &str) -> LogId<'_> {
    LogId::with_key(Kind::Sid, id, key())
}

/// An op id for a log line.
pub fn op(id: &str) -> LogId<'_> {
    LogId::with_key(Kind::Op, id, key())
}

/// A replica uuid for a log line.
pub fn rid(id: &str) -> LogId<'_> {
    LogId::with_key(Kind::Rid, id, key())
}

/// A machine id for a log line.
pub fn mid(id: &str) -> LogId<'_> {
    LogId::with_key(Kind::Mid, id, key())
}

/// `nxf-relay log-hash <kind> <id>` (C8): print the token of `id` under the key in
/// [`KEY_ENV`], exactly as a log line carries it. `key_value` is the variable's value (`None` when
/// unset); the caller reads it, so this stays testable without process-global state.
///
/// Exit 0 with the token on `out`; exit 2 with a message on `err` and nothing on `out` when the
/// key is unset or malformed (the message never echoes it), the kind is not one of C3's, or the
/// arguments are not exactly a kind and an id.
pub fn log_hash_command(
    args: &[String],
    key_value: Option<&str>,
    out: &mut dyn std::io::Write,
    err: &mut dyn std::io::Write,
) -> u8 {
    const USAGE: &str = "usage: nxf-relay log-hash <sid|op|rid|mid|uid> <id>";
    let [kind, id] = args else {
        let _ = writeln!(err, "nxf-relay log-hash: {USAGE}");
        return 2;
    };
    let Some(kind) = Kind::parse(kind) else {
        let _ = writeln!(
            err,
            "nxf-relay log-hash: unknown kind; the kinds are sid, op, rid, mid and uid"
        );
        return 2;
    };
    let key = match key_value.map(LogKey::parse) {
        Some(Ok(key)) => key,
        Some(Err(e)) => {
            let _ = writeln!(err, "nxf-relay log-hash: {e}");
            return 2;
        }
        None => {
            let _ = writeln!(
                err,
                "nxf-relay log-hash: {KEY_ENV} is not set; there is no key to hash under"
            );
            return 2;
        }
    };
    match writeln!(out, "{}", key.token(kind, id)) {
        Ok(()) => 0,
        Err(_) => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C9's key: the bytes 0x00..0x1f.
    const C9_KEY: &str = "t1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";

    fn c9() -> LogKey {
        LogKey::parse(C9_KEY).expect("the C9 key parses")
    }

    #[test]
    fn the_c9_key_decodes_to_the_bytes_zero_to_thirty_one() {
        let key = c9();
        assert_eq!(key.kid(), "t1");
        let expected: Vec<u8> = (0u8..32).collect();
        assert_eq!(key.key.to_vec(), expected);
    }

    #[test]
    fn every_c9_vector_is_reproduced() {
        let key = c9();
        for (kind, id, token) in [
            (Kind::Sid, "6j6v", "sid=h1:t1:d5bfa71653806fdbcfb1052f"),
            (
                Kind::Op,
                "01J0DEMOOP00000000000000A",
                "op=h1:t1:ab7c22dbcefc68c0fc2f34d4",
            ),
            (
                Kind::Sid,
                "01J0DEMOOP00000000000000A",
                "sid=h1:t1:332d1e2cee79a5b45589a4e6",
            ),
            (
                Kind::Rid,
                "3f1c2a9e-0000-4000-8000-000000000001",
                "rid=h1:t1:ca8c97c1c46d0dc5c92131fc",
            ),
            (Kind::Mid, "m-laptop-1", "mid=h1:t1:232916555290733ba960135f"),
        ] {
            assert_eq!(key.token(kind, id), token, "{kind:?} {id}");
        }
    }

    #[test]
    fn the_tag_is_the_token_without_its_field_name() {
        let key = c9();
        assert_eq!(key.tag(Kind::Sid, "6j6v"), "h1:t1:d5bfa71653806fdbcfb1052f");
    }

    #[test]
    fn the_id_is_hashed_byte_for_byte_with_no_normalisation() {
        let key = c9();
        let plain = key.tag(Kind::Sid, "6j6v");
        for variant in ["6J6V", " 6j6v", "6j6v ", "6j6v\n"] {
            assert_ne!(key.tag(Kind::Sid, variant), plain, "{variant:?}");
        }
    }

    /// Each C1 refusal, with the rule it breaks. A valid 43-character key is the base the bad
    /// values are made from, so each case breaks exactly one rule.
    fn refusals() -> Vec<(String, KeyError)> {
        let good = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
        assert_eq!(good.len(), 43);
        vec![
            (format!("t1:{}", &good[..42]), KeyError::KeyLength),
            (format!("t1:{good}A"), KeyError::KeyLength),
            (format!("t1:{}=", &good[..42]), KeyError::Padding),
            (format!("t1:{}+", &good[..42]), KeyError::KeyAlphabet),
            (format!("t1:{}/", &good[..42]), KeyError::KeyAlphabet),
            (format!(":{good}"), KeyError::EmptyKid),
            (format!("T1:{good}"), KeyError::KidAlphabet),
            (format!("{}:{good}", "a".repeat(17)), KeyError::KidTooLong),
            (format!("t1:{good}:x"), KeyError::SecondSeparator),
            (format!("t1:x:{good}"), KeyError::SecondSeparator),
            (good.to_string(), KeyError::MissingSeparator),
            (String::new(), KeyError::MissingSeparator),
            (format!("t1:{good}\n"), KeyError::KeyLength),
            (format!(" t1:{good}"), KeyError::KidAlphabet),
            // The last character of 43 carries 2 bits of padding that must be zero.
            (format!("t1:{}9", &good[..42]), KeyError::KeyDecode),
        ]
    }

    #[test]
    fn every_c1_refusal_names_its_rule() {
        for (value, rule) in refusals() {
            assert_eq!(LogKey::parse(&value), Err(rule.clone()), "{value:?}");
        }
    }

    #[test]
    fn a_refusal_never_echoes_the_value() {
        for (value, _) in refusals() {
            let message = LogKey::parse(&value).expect_err("refused").to_string();
            assert!(message.starts_with("NXS_LOG_HMAC_KEY is malformed"), "{message}");
            // Every fragment of the value long enough to identify it stays out of the message.
            for start in 0..value.len().saturating_sub(7) {
                if let Some(piece) = value.get(start..start + 8) {
                    assert!(!message.contains(piece), "{message:?} echoes {piece:?}");
                }
            }
        }
    }

    #[test]
    fn a_sixteen_character_kid_of_digits_and_lowercase_is_accepted() {
        let good = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8";
        let key = LogKey::parse(&format!("20261101abcdef99:{good}")).expect("accepted");
        assert_eq!(key.kid(), "20261101abcdef99");
    }

    #[test]
    fn debug_never_shows_the_key_bytes() {
        let shown = format!("{:?}", c9());
        assert!(shown.contains("t1") && shown.contains("redacted"), "{shown}");
        assert!(!shown.contains("[0, 1, 2"), "{shown}");
    }

    #[test]
    fn without_a_key_an_id_is_written_as_its_kind_and_a_dash() {
        assert_eq!(LogId::with_key(Kind::Sid, "6j6v", None).to_string(), "sid=-");
        assert_eq!(LogId::with_key(Kind::Mid, "m-laptop-1", None).to_string(), "mid=-");
    }

    #[test]
    fn with_a_key_an_id_is_written_as_its_token() {
        let key = c9();
        assert_eq!(
            LogId::with_key(Kind::Sid, "6j6v", Some(&key)).to_string(),
            "sid=h1:t1:d5bfa71653806fdbcfb1052f"
        );
    }

    #[test]
    fn kinds_parse_by_their_exact_names_only() {
        for kind in [Kind::Sid, Kind::Op, Kind::Rid, Kind::Mid, Kind::Uid] {
            assert_eq!(Kind::parse(kind.name()), Some(kind));
        }
        for unknown in ["SID", "stream", "", "sid "] {
            assert_eq!(Kind::parse(unknown), None, "{unknown:?}");
        }
    }

    fn run_log_hash(args: &[&str], key: Option<&str>) -> (u8, String, String) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = log_hash_command(&args, key, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).expect("utf-8"),
            String::from_utf8(err).expect("utf-8"),
        )
    }

    #[test]
    fn log_hash_prints_the_c5_token_and_exits_zero() {
        let (code, out, err) = run_log_hash(&["sid", "6j6v"], Some(C9_KEY));
        assert_eq!(code, 0, "{err}");
        assert_eq!(out, "sid=h1:t1:d5bfa71653806fdbcfb1052f\n");
        let (code, out, _) = run_log_hash(&["mid", "m-laptop-1"], Some(C9_KEY));
        assert_eq!((code, out.as_str()), (0, "mid=h1:t1:232916555290733ba960135f\n"));
    }

    #[test]
    fn log_hash_without_a_key_exits_two_and_prints_nothing_on_stdout() {
        let (code, out, err) = run_log_hash(&["sid", "6j6v"], None);
        assert_eq!(code, 2);
        assert!(out.is_empty(), "{out}");
        assert!(err.contains("NXS_LOG_HMAC_KEY"), "{err}");
    }

    #[test]
    fn log_hash_with_a_malformed_key_exits_two_without_echoing_it() {
        let bad = "t1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh";
        let (code, out, err) = run_log_hash(&["sid", "6j6v"], Some(bad));
        assert_eq!(code, 2);
        assert!(out.is_empty(), "{out}");
        assert!(!err.contains("AAECAwQFBgcICQoLDA0O"), "{err}");
    }

    #[test]
    fn log_hash_refuses_an_unknown_kind_and_a_wrong_argument_count() {
        for args in [
            &["stream", "6j6v"][..],
            &["sid"][..],
            &[][..],
            &["sid", "6j6v", "extra"][..],
        ] {
            let (code, out, _) = run_log_hash(args, Some(C9_KEY));
            assert_eq!(code, 2, "{args:?}");
            assert!(out.is_empty(), "{args:?}: {out}");
        }
    }

    #[test]
    fn log_hash_never_prints_the_id() {
        let (_, out, err) = run_log_hash(&["sid", "seed-stream-7q3x"], Some(C9_KEY));
        assert!(!out.contains("seed-stream-7q3x") && !err.contains("seed-stream-7q3x"));
        let (_, out, err) = run_log_hash(&["stream", "seed-stream-7q3x"], Some(C9_KEY));
        assert!(!out.contains("seed-stream-7q3x") && !err.contains("seed-stream-7q3x"));
    }
}
