//! Deterministic content-hash auto-keys (spec §2.2) — the bd-parity growth lever.
//!
//! When `remember` is given no `--key`, the key is derived from the body so that a **byte-identical**
//! body dedups to one entity (in-place update) while a **reworded** body mints a new entity (and so
//! accumulates) — exactly bd's documented behavior (research §2/§4b). The hash is a pure function of
//! the body, so unlike flow's random note op-ids, auto-keys are golden-test-stable without wildcards.

use crate::model::AUTO_KEY_PREFIX;
use sha2::{Digest, Sha256};

/// How many hex chars of the sha256 digest the auto-key carries. 16 hex = 64 bits — ample to keep
/// distinct memories from colliding in a hand-curated store, short enough to stay readable.
const AUTO_KEY_HEX_LEN: usize = 16;

/// Derive a deterministic auto-key for `body`: `f-` + the first [`AUTO_KEY_HEX_LEN`] hex chars of
/// `sha256(body)`. Pure function of the body (spec §2.2).
pub fn auto_key(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    let mut key = String::with_capacity(AUTO_KEY_PREFIX.len() + AUTO_KEY_HEX_LEN);
    key.push_str(AUTO_KEY_PREFIX);
    for byte in digest.iter().take(AUTO_KEY_HEX_LEN / 2) {
        key.push_str(&format!("{byte:02x}"));
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_key_is_deterministic_and_f_prefixed() {
        let a = auto_key("auth uses JWT not sessions");
        let b = auto_key("auth uses JWT not sessions");
        assert_eq!(a, b, "same body → same key (pure function)");
        assert!(
            a.starts_with("f-"),
            "auto-keys carry the fact-domain prefix: {a}"
        );
    }

    #[test]
    fn auto_key_is_the_prefix_plus_sixteen_hex_chars() {
        // Pin the truncation length directly (not just dedup-vs-diverge): `f-` + 16 hex of sha256.
        let k = auto_key("anything");
        let hex = k.strip_prefix("f-").expect("f- prefix");
        assert_eq!(hex.len(), 16, "16 hex chars of the digest: {k}");
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()), "all hex: {k}");
    }

    #[test]
    fn byte_identical_bodies_dedup_reworded_bodies_diverge() {
        // bd parity (research §2/§4b): byte-equal repeats dedup to one key; a reworded variant
        // gets a NEW key (and so accumulates) — the documented growth behavior.
        assert_eq!(auto_key("x"), auto_key("x"));
        assert_ne!(
            auto_key("auth uses JWT"),
            auto_key("auth uses JWT (HS256)"),
            "a reworded body must mint a distinct key"
        );
    }
}
