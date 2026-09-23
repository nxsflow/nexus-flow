//! dqj — the single, deterministic free-text short-id rewrite engine (spec §8).
//!
//! When a replica is reassigned a new prefix (bab/§6), its locally-minted ids change
//! `{old}.suffix → {new}.suffix`. The structural remap (T-remap) rewrites ids in the op
//! log and views; THIS engine rewrites the same ids where they appear as **references in
//! free text** (an item's body, a closing comment, …). It is the one shared impl the
//! remap pass calls — there is no second rewrite path.
//!
//! Determinism contract: only exact short-id tokens that are keys of the remap map are
//! replaced, matched on non-alphanumeric boundaries so a longer token that merely
//! *contains* an id (`aaaa.00012`, `xaaaa.0001`) is never touched. The map's values carry
//! the new prefix, so a rewritten id is never itself a key → the pass is idempotent.

use std::collections::BTreeMap;

/// A character that can be part of a short-id token (`prefix.suffix` is all ASCII
/// alphanumerics around a `.`). The `.` is deliberately NOT included so a sentence-ending
/// period (`see aaaa.0001.`) is a clean boundary, not part of the token.
fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// Rewrite every free-text occurrence of a remapped short-id in `text`. `remap` maps an
/// old short-id to its new form. Occurrences are matched only at non-id-char boundaries;
/// keys are tried longest-first so the match is unambiguous. Deterministic and idempotent.
pub fn rewrite_refs(text: &str, remap: &BTreeMap<String, String>) -> String {
    if remap.is_empty() {
        return text.to_string();
    }
    let mut keys: Vec<&str> = remap.keys().map(String::as_str).collect();
    // Longest key first (then lexicographic) → deterministic longest-match at a position.
    keys.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));

    let mut out = String::with_capacity(text.len());
    let mut idx = 0;
    let n = text.len();
    while idx < n {
        // A reference must start at a token boundary: the char just before is not an id char.
        let before_is_id = text[..idx].chars().next_back().is_some_and(is_id_char);
        let mut matched = false;
        if !before_is_id {
            for key in &keys {
                if text[idx..].starts_with(key) {
                    // ...and end at a boundary too: the char just after is not an id char.
                    let after_is_id = text[idx + key.len()..]
                        .chars()
                        .next()
                        .is_some_and(is_id_char);
                    if !after_is_id {
                        out.push_str(&remap[*key]);
                        idx += key.len();
                        matched = true;
                        break;
                    }
                }
            }
        }
        if !matched {
            let ch = text[idx..]
                .chars()
                .next()
                .expect("idx < n is a char boundary");
            out.push(ch);
            idx += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remap(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn rewrites_a_bare_reference() {
        let m = remap(&[("aaaa.0001", "kp3z.0001")]);
        assert_eq!(
            rewrite_refs("see aaaa.0001 please", &m),
            "see kp3z.0001 please"
        );
    }

    #[test]
    fn respects_token_boundaries_punctuation_and_eol() {
        let m = remap(&[("aaaa.0001", "kp3z.0001")]);
        // Parens, trailing period, start- and end-of-string are all clean boundaries.
        assert_eq!(rewrite_refs("(aaaa.0001).", &m), "(kp3z.0001).");
        assert_eq!(rewrite_refs("aaaa.0001", &m), "kp3z.0001");
        assert_eq!(rewrite_refs("ref: aaaa.0001\n", &m), "ref: kp3z.0001\n");
    }

    #[test]
    fn never_rewrites_a_substring_or_a_longer_token() {
        let m = remap(&[("aaaa.0001", "kp3z.0001")]);
        // Trailing alphanumerics or a leading id char mean it is NOT this id.
        assert_eq!(rewrite_refs("aaaa.00012", &m), "aaaa.00012");
        assert_eq!(rewrite_refs("xaaaa.0001", &m), "xaaaa.0001");
    }

    #[test]
    fn leaves_unmapped_and_foreign_ids_untouched() {
        let m = remap(&[("aaaa.0001", "kp3z.0001")]);
        // A foreign replica's id (different prefix) is not in the map → untouched.
        assert_eq!(
            rewrite_refs("links aaaa.0001 to bbbb.0002", &m),
            "links kp3z.0001 to bbbb.0002"
        );
    }

    #[test]
    fn rewrites_multiple_distinct_references() {
        let m = remap(&[("aaaa.0001", "kp3z.0001"), ("aaaa.0002", "kp3z.0002")]);
        assert_eq!(
            rewrite_refs("aaaa.0001 blocks aaaa.0002", &m),
            "kp3z.0001 blocks kp3z.0002"
        );
    }

    #[test]
    fn is_idempotent() {
        // The new prefix is never a key, so a second pass changes nothing — crucial for a
        // remap that may re-run after a crash.
        let m = remap(&[("aaaa.0001", "kp3z.0001")]);
        let once = rewrite_refs("aaaa.0001 and aaaa.0001", &m);
        assert_eq!(rewrite_refs(&once, &m), once);
    }

    #[test]
    fn empty_remap_is_identity_even_with_unicode() {
        let m: BTreeMap<String, String> = BTreeMap::new();
        assert_eq!(
            rewrite_refs("nothing aaaa.0001 — 日本語", &m),
            "nothing aaaa.0001 — 日本語"
        );
    }
}
