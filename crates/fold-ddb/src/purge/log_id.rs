//! A copy of `nxs_server::log_id`'s key and token (log-id v1, 6j6v.01wt C1-C5), so the board-fold
//! purge prints a stream id only as its `sid` token without this crate depending on the relay
//! (design §3, f6yb MF-13). What is copied is the contract, not a dependency: the tests below
//! reproduce the five C9 vectors and refuse exactly C1's values, and `purge::tests` holds every
//! token and every refusal equal to the relay's own, so the two copies cannot drift apart.
//!
//! Only what the purge needs is here: [`LogKey::parse`] and [`LogKey::tag`]/[`LogKey::token`].
//! The process-wide key, the log-line helpers and `nxf-relay log-hash` stay the relay's.

use std::fmt;

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
            (
                Kind::Mid,
                "m-laptop-1",
                "mid=h1:t1:232916555290733ba960135f",
            ),
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
            assert!(
                message.starts_with("NXS_LOG_HMAC_KEY is malformed"),
                "{message}"
            );
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
        assert!(
            shown.contains("t1") && shown.contains("redacted"),
            "{shown}"
        );
        assert!(!shown.contains("[0, 1, 2"), "{shown}");
    }
}
