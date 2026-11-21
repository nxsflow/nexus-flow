//! The known secret patterns a status note is refused for (RFC 0007, "Secrets and personal
//! data"). The scan is pure: it reads one field's text and says which pattern it matched, and the
//! refusal names the field and the pattern, never the match, so no part of a secret reaches an
//! error message, a log or a terminal.

use std::fmt;

/// A known secret pattern. The list is nexus-flow's, ships with `nxs`, and grows with releases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretPattern {
    /// A private key block: `-----BEGIN … PRIVATE KEY-----`.
    PrivateKey,
}

impl SecretPattern {
    /// Every pattern, in the order a text is scanned for them.
    pub const ALL: [SecretPattern; 1] = [SecretPattern::PrivateKey];

    /// The pattern's name, as the refusal shows it.
    pub fn name(self) -> &'static str {
        match self {
            SecretPattern::PrivateKey => "private_key",
        }
    }

    fn matches(self, text: &str) -> bool {
        match self {
            SecretPattern::PrivateKey => private_key(text),
        }
    }
}

impl fmt::Display for SecretPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A field of a note holds a secret by a known pattern (`secret_in_note`). It carries the field
/// and the pattern, and nothing of the text that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretInNote {
    /// The field, as the caller named it: `headline`, `body`, or a line's `text` or `label`.
    pub field: String,
    /// The pattern the field matched.
    pub pattern: SecretPattern,
}

impl SecretInNote {
    /// The error's code.
    pub fn code(&self) -> &'static str {
        "secret_in_note"
    }
}

impl fmt::Display for SecretInNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: the field `{}` holds a secret by the pattern `{}`",
            self.code(),
            self.field,
            self.pattern
        )
    }
}

impl std::error::Error for SecretInNote {}

/// Refuses `text`, the content of the note's `field`, when it matches a known secret pattern; the
/// first pattern in [`SecretPattern::ALL`] that matches is named.
pub fn check(field: &str, text: &str) -> Result<(), SecretInNote> {
    match SecretPattern::ALL.into_iter().find(|p| p.matches(text)) {
        Some(pattern) => Err(SecretInNote {
            field: field.to_owned(),
            pattern,
        }),
        None => Ok(()),
    }
}

/// `-----BEGIN `, a label of upper-case letters, digits and spaces that holds `PRIVATE KEY`, then
/// `-----`: every PEM private key (`RSA`, `EC`, `OPENSSH`, `ENCRYPTED`, none) and a PGP private
/// key block.
fn private_key(text: &str) -> bool {
    const BEGIN: &str = "-----BEGIN ";
    text.match_indices(BEGIN).any(|(at, _)| {
        let rest = &text[at + BEGIN.len()..];
        let label_len = rest
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b' ')
            .count();
        rest[label_len..].starts_with("-----") && rest[..label_len].contains("PRIVATE KEY")
    })
}
