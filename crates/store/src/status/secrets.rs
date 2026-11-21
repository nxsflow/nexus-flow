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
    /// A GitHub token: `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` or `github_pat_`, and its body.
    GitHubToken,
    /// An AWS access key id: `AKIA` or `ASIA` and 16 upper-case letters or digits.
    AwsAccessKeyId,
    /// A Slack token: `xoxa-`, `xoxb-`, `xoxp-`, `xoxr-` or `xoxs-`, and its body.
    SlackToken,
    /// A JSON web token: three base64url parts joined by `.`, the first starting `eyJ`.
    JsonWebToken,
}

impl SecretPattern {
    /// Every pattern, in the order a text is scanned for them.
    pub const ALL: [SecretPattern; 5] = [
        SecretPattern::PrivateKey,
        SecretPattern::GitHubToken,
        SecretPattern::AwsAccessKeyId,
        SecretPattern::SlackToken,
        SecretPattern::JsonWebToken,
    ];

    /// The pattern's name, as the refusal shows it.
    pub fn name(self) -> &'static str {
        match self {
            SecretPattern::PrivateKey => "private_key",
            SecretPattern::GitHubToken => "github_token",
            SecretPattern::AwsAccessKeyId => "aws_access_key_id",
            SecretPattern::SlackToken => "slack_token",
            SecretPattern::JsonWebToken => "json_web_token",
        }
    }

    fn matches(self, text: &str) -> bool {
        match self {
            SecretPattern::PrivateKey => private_key(text),
            SecretPattern::GitHubToken => ["ghp_", "gho_", "ghu_", "ghs_", "ghr_", "github_pat_"]
                .into_iter()
                .any(|prefix| prefixed(text, prefix, is_word)),
            SecretPattern::AwsAccessKeyId => aws_access_key_id(text),
            SecretPattern::SlackToken => ["xoxa-", "xoxb-", "xoxp-", "xoxr-", "xoxs-"]
                .into_iter()
                .any(|prefix| prefixed(text, prefix, is_word_or_dash)),
            SecretPattern::JsonWebToken => json_web_token(text),
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

/// A run of exactly 20 upper-case letters or digits, not part of a longer word, that starts with
/// `AKIA` or `ASIA`.
fn aws_access_key_id(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_key = |b: u8| b.is_ascii_uppercase() || b.is_ascii_digit();
    ["AKIA", "ASIA"].into_iter().any(|prefix| {
        text.match_indices(prefix).any(|(at, _)| {
            let end = at + 20;
            let starts_word = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
            let ends_word = bytes.get(end).is_none_or(|b| !b.is_ascii_alphanumeric());
            starts_word
                && ends_word
                && bytes.get(at..end).is_some_and(|key| key.iter().all(|b| is_key(*b)))
        })
    })
}

/// Three non-empty runs of base64url characters joined by `.`, the first starting `eyJ` (the
/// encoding of `{"`) at the start of a word.
fn json_web_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices("eyJ").any(|(at, _)| {
        if at > 0 && is_word_or_dash(bytes[at - 1]) {
            return false;
        }
        let mut rest = &bytes[at..];
        for part in 0..3 {
            let len = rest.iter().take_while(|b| is_word_or_dash(**b)).count();
            if len == 0 {
                return false;
            }
            rest = &rest[len..];
            if part < 2 {
                match rest.split_first() {
                    Some((b'.', after)) => rest = after,
                    _ => return false,
                }
            }
        }
        true
    })
}

/// The fewest token characters after a token's prefix for it to count as a token, so that a line
/// that only names a prefix, such as "classic tokens start with ghp_", is not refused. Every token
/// these prefixes start is longer.
const MIN_TOKEN_BODY: usize = 20;

/// `prefix` followed by at least [`MIN_TOKEN_BODY`] bytes that `is_body` accepts.
fn prefixed(text: &str, prefix: &str, is_body: fn(u8) -> bool) -> bool {
    text.match_indices(prefix).any(|(at, _)| {
        text[at + prefix.len()..]
            .bytes()
            .take_while(|b| is_body(*b))
            .count()
            >= MIN_TOKEN_BODY
    })
}

/// A letter, a digit or `_`.
fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A letter, a digit, `_` or `-`.
fn is_word_or_dash(b: u8) -> bool {
    is_word(b) || b == b'-'
}
