//! A member's ticket tag (RFC 0002, "Ticket ids"): two characters, a lower-case ASCII letter and
//! then a lower-case letter or a digit, such as `ka` or `p7`; 936 in all. It starts the keys of the
//! tickets its member creates.
//!
//! [`pick`] takes one of the tags that are not taken, chosen by a seed: `nxs member add` seeds it
//! with the new member's id, which is random, so that two inviters picking offline at the same time
//! rarely give two members the same tag, as RFC 0002 counts on.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

/// The characters a tag may end with, in byte order: the digits, then the letters.
const SECOND: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// A ticket tag, such as `ka`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag([u8; 2]);

/// A text that is no tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotATag;

impl Tag {
    /// The tag as written.
    pub fn as_str(&self) -> &str {
        // Both bytes are ASCII, as `from_str` and `all` check.
        std::str::from_utf8(&self.0).expect("a tag is ASCII")
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Tag {
    type Err = NotATag;

    fn from_str(text: &str) -> Result<Self, NotATag> {
        match text.as_bytes() {
            &[first, second] if first.is_ascii_lowercase() && SECOND.contains(&second) => {
                Ok(Tag([first, second]))
            }
            _ => Err(NotATag),
        }
    }
}

/// Every tag, in byte order: `a0` first, `zz` last.
pub fn all() -> impl Iterator<Item = Tag> {
    (b'a'..=b'z').flat_map(|first| SECOND.iter().map(move |&second| Tag([first, second])))
}

/// A tag that is not in `taken`, chosen by `seed`; `None` when every tag is taken.
pub fn pick(taken: &BTreeSet<Tag>, seed: &[u8]) -> Option<Tag> {
    let free: Vec<Tag> = all().filter(|tag| !taken.contains(tag)).collect();
    if free.is_empty() {
        return None;
    }
    let mut bytes = [0u8; 8];
    for (slot, byte) in bytes.iter_mut().zip(seed) {
        *slot = *byte;
    }
    let index = u64::from_le_bytes(bytes) % free.len() as u64;
    Some(free[index as usize])
}
