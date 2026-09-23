//! ID strategy: short, human-readable `{prefix}.{suffix}` — four Crockford-base32
//! (lowercase) chars on each side, e.g. `ab12.7x3k`. The replica `prefix` namespaces
//! ids across replicas (see `Replica`); the random `suffix` is unique *within* a
//! replica by minting through [`ReplicaIds::mint_unique`], which retries against the
//! local store on collision. The op-log fold merges same-id creates silently, so
//! uniqueness must be secured at mint time, not relied on as a hard constraint.
//!
//! CROSS-REPLICA HAZARD (E4): cross-replica uniqueness rests entirely on the `prefix`
//! being distinct per replica. `prefix` is currently minted at random in a 32^4≈1M
//! space with NO coordination, so two replicas can collide (birthday-bounded); such a
//! collision lets same-id creates on different replicas silently alias into one item
//! when their op-logs merge. Harmless in single-replica E2, but the sync layer (E4)
//! must guarantee prefix distinctness (coordinate/register prefixes, widen the space,
//! or detect+remap on merge). Tracked in beads; do not assume prefixes are unique.

/// True iff `s` is a well-formed short-id prefix: exactly four Crockford-base32 chars
/// (lowercased → ASCII alphanumeric). Used to reject a malformed prefix arriving from an
/// UNtrusted source — a compromised relay's `Reassigned(new_prefix)` — BEFORE it is spliced
/// into ids: a prefix carrying a `.` or the edge composite separator (`\u{1f}`) would make
/// later id splitting mis-parse. The honest path (offline mint, registry-derived candidate)
/// always satisfies this.
pub fn is_valid_prefix(s: &str) -> bool {
    s.len() == 4 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// How many random suffixes to try before declaring the local space exhausted. The
/// suffix space is 32^4 ≈ 1.05M; even near a replica's expected ceiling (tens of
/// thousands of items) a candidate collides only a few percent of the time, so 64
/// tries failing would mean the space is genuinely full — a loud error, not a retry.
const MINT_ATTEMPTS: usize = 64;

/// Mints ids for one replica. Pick a distinct `prefix` per install.
pub struct ReplicaIds {
    prefix: String,
}

impl ReplicaIds {
    pub fn new(prefix: &str) -> ReplicaIds {
        ReplicaIds {
            prefix: prefix.to_string(),
        }
    }

    /// A fresh candidate id `{prefix}.{suffix}`. The suffix is the last four chars of a
    /// random ULID, lowercased — uniform over the Crockford-base32 alphabet. Not
    /// guaranteed unique on its own; go through [`mint_unique`](Self::mint_unique).
    pub fn new_id(&self) -> String {
        let ulid = ulid::Ulid::new().to_string().to_ascii_lowercase();
        let suffix = &ulid[ulid.len() - 4..];
        format!("{}.{}", self.prefix, suffix)
    }

    /// Mint an id whose suffix is free locally, retrying on collision. `is_taken`
    /// reports whether a candidate already exists in this replica's store. Returns
    /// `None` only if [`MINT_ATTEMPTS`] candidates all collided — surface that loudly
    /// rather than looping forever.
    pub fn mint_unique(&self, mut is_taken: impl FnMut(&str) -> bool) -> Option<String> {
        (0..MINT_ATTEMPTS)
            .map(|_| self.new_id())
            .find(|id| !is_taken(id))
    }

    /// The `n`-th deterministic id for this replica: `{prefix}.{n:04}`. The suffix is a
    /// zero-padded decimal — and decimal digits are a strict subset of the Crockford-base32
    /// lowercase alphabet, so the id is as well-formed as a random one (just visibly synthetic,
    /// e.g. `ab12.0001`). Used only by [`mint_sequential`]; `n` must be in `1..=9999` to keep the
    /// four-char suffix (a larger `n` would widen it and break the 4+4 shape).
    pub fn nth_id(&self, n: u32) -> String {
        debug_assert!(
            (1..=9999).contains(&n),
            "nth_id(n) needs n in 1..=9999 to keep a 4-char suffix, got {n}"
        );
        format!("{}.{:04}", self.prefix, n)
    }

    /// Deterministically mint the first locally-free id in the sequence `nth_id(1),
    /// nth_id(2), …` (nexus-flow-4oa.2). Unlike [`mint_unique`] (random suffix), this is a
    /// pure function of store state: scanning from 1 every time means the result depends only
    /// on which ids are already taken, so two separate `nxf` processes minting against the same
    /// state agree — the property a reproducible multi-step doc example needs. `None` only if
    /// the whole `1..=9999` window is taken (a loud failure, never an infinite loop). This is a
    /// test/docs determinism affordance; production minting goes through [`mint_unique`].
    ///
    /// The rescan-from-1 makes the Nth call O(N) store probes (O(N²) over a workspace's life),
    /// which is a deliberate, immaterial trade-off: this path runs only under the determinism
    /// switch, where N is a handful of seeded items — not worth a stateful counter that would
    /// have to survive across the separate processes this very property buys us.
    pub fn mint_sequential(&self, mut is_taken: impl FnMut(&str) -> bool) -> Option<String> {
        (1..=9999).map(|n| self.nth_id(n)).find(|id| !is_taken(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crockford base32, lowercased: digits + a–z minus the visually ambiguous
    /// `i l o u`. Both the prefix and the suffix draw from this alphabet.
    const CROCKFORD_LOWER: &str = "0123456789abcdefghjkmnpqrstvwxyz";

    #[test]
    fn suffix_is_four_crockford_lowercase_chars() {
        let m = ReplicaIds::new("ab12");
        for _ in 0..200 {
            let id = m.new_id();
            let (prefix, suffix) = id.split_once('.').expect("id has one '.' separator");
            assert_eq!(prefix, "ab12", "prefix is preserved: {id}");
            assert_eq!(suffix.chars().count(), 4, "suffix is 4 chars: {id}");
            assert!(
                suffix.chars().all(|c| CROCKFORD_LOWER.contains(c)),
                "suffix is crockford-base32 lowercase: {id}"
            );
        }
    }

    /// Deterministic minting (golden-docs harness, nexus-flow-4oa.2): `nth_id` is a pure,
    /// stable function of `(prefix, n)` — a zero-padded decimal suffix (digits are a subset
    /// of Crockford-base32, so the id stays well-formed). This is what lets a doc example
    /// show a fixed id like `ab12.0001` instead of a random one.
    #[test]
    fn nth_id_is_prefix_plus_zero_padded_decimal() {
        let m = ReplicaIds::new("ab12");
        assert_eq!(m.nth_id(1), "ab12.0001");
        assert_eq!(m.nth_id(42), "ab12.0042");
        assert_eq!(m.nth_id(9999), "ab12.9999");
        // Suffix is four chars drawn from the Crockford-lowercase alphabet (digits only here).
        let id = m.nth_id(7);
        let (_, suffix) = id.split_once('.').unwrap();
        assert_eq!(suffix.chars().count(), 4);
        assert!(suffix.chars().all(|c| CROCKFORD_LOWER.contains(c)));
    }

    /// `mint_sequential` walks `nth_id(1), nth_id(2), …` and returns the FIRST id free in the
    /// local store. Because it always scans from 1, it is a deterministic function of store
    /// state — across SEPARATE `nxf` processes (each a fresh start), the Nth create against an
    /// otherwise-untouched store yields the Nth id. That cross-process stability is exactly what
    /// makes a multi-step golden example (`create` → `claim <that id>`) reproducible.
    #[test]
    fn mint_sequential_returns_first_free_in_order() {
        let m = ReplicaIds::new("ab12");
        let taken = ["ab12.0001", "ab12.0002"];
        let id = m
            .mint_sequential(|c| taken.contains(&c))
            .expect("a free sequential id");
        assert_eq!(id, "ab12.0003");
    }

    #[test]
    fn mint_sequential_first_free_is_one_and_is_stable() {
        let m = ReplicaIds::new("ab12");
        // Nothing taken → first id is always `0001`, identically across calls (determinism).
        assert_eq!(m.mint_sequential(|_| false).unwrap(), "ab12.0001");
        assert_eq!(m.mint_sequential(|_| false), m.mint_sequential(|_| false));
    }

    #[test]
    fn mint_sequential_returns_none_when_space_exhausted() {
        let m = ReplicaIds::new("ab12");
        // Every candidate taken → loud None, never an infinite loop.
        assert!(m.mint_sequential(|_| true).is_none());
    }

    #[test]
    fn mint_unique_retries_past_taken_candidates() {
        let m = ReplicaIds::new("ab12");
        let mut calls = 0;
        // First two candidates are "taken"; the third is free.
        let id = m
            .mint_unique(|_| {
                calls += 1;
                calls <= 2
            })
            .expect("a free id within the attempt budget");
        assert!(id.starts_with("ab12."));
        assert_eq!(calls, 3, "kept minting until a free suffix was found");
    }

    #[test]
    fn mint_unique_returns_none_when_space_exhausted() {
        let m = ReplicaIds::new("ab12");
        // Every candidate collides: minting gives up loudly, it does not loop forever.
        assert!(m.mint_unique(|_| true).is_none());
    }

    #[test]
    fn valid_prefix_accepts_four_alphanumerics_and_rejects_unsafe_shapes() {
        for ok in ["ab12", "aaaa", "0000", "zzzz"] {
            assert!(is_valid_prefix(ok), "{ok} should be valid");
        }
        // Wrong length, empty, and — crucially — anything that would break id splitting:
        // a dot or the edge composite separator embedded in the prefix.
        for bad in ["", "abc", "abcde", "ab.2", "a b1", "ab\u{1f}2", "ab/2"] {
            assert!(!is_valid_prefix(bad), "{bad:?} should be rejected");
        }
    }
}
