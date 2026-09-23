//! The substrate op — the unit of history, sync, and convergence (spec §4.1). Domain-agnostic:
//! every product's ops share this one shape, and the `domain` field tags which product/reducer
//! owns it. Product vocabulary (item types, statuses, edge kinds, the materialized row shapes)
//! lives in each product crate, never here.

/// The highest Lamport number an op may carry and still take part in the fold (6j6v.m19v).
///
/// A real log counts in thousands; `2^62` leaves every honest history room forever while keeping
/// the clock far from `i64::MAX`. An op past it is one no correct replica can have minted — it is
/// the shape a hostile writer at a relay that authenticates nobody plants, because the Lamport rule
/// (`clock = max(clock, op.lamport)`, then `+ 1`) would otherwise park every later local write
/// where it overflows, and let that one op win every compare it takes part in.
///
/// §7 forbids dropping a foreign op, so such an op is KEPT and made inert: it stays in the log,
/// visible to anything that reads the log as a log (history, export, an image), and nothing that
/// folds or orders by the clock takes it into account — no reducer folds it and no clock is seeded
/// from it. The rule is a pure function of the op, so every replica agrees on it and the board
/// still converges. See [`Op::lamport_in_bound`].
pub const MAX_LAMPORT: i64 = 1 << 62;

/// The highest Lamport number an op this replica RECEIVED can move its clock to (6j6v.m19v).
///
/// [`MAX_LAMPORT`] alone would only move the attack: an op at exactly the bound folds, parks every
/// clock there, and every local op after it lands above the bound and is inert — a frozen board
/// instead of a crash. Capping what a received op does to the clock `2^61` below the bound leaves
/// that many local writes of headroom, which no replica will ever use up.
///
/// The price, stated rather than hidden: an op between this ceiling and [`MAX_LAMPORT`] folds but
/// does not move the clock, so it keeps winning its one field against later writes. A writer who
/// can reach the relay can pin a field that way (it can already overwrite any field) — it can no
/// longer crash a replica, wrap its clock or freeze the board. Which ops a replica ACTS on is the
/// signature's question (6j6v.pzkb), not the clock's.
pub const CLOCK_CEILING: i64 = 1 << 61;

/// The tag in front of an op's [canonical bytes](Op::canonical_bytes) — the bytes its signature is
/// taken over. Part of what is signed, so a signature is bound to the encoding that produced it.
///
/// `nxs-op/2`, because `nxs-op/1` (6j6v.xsf3) also covered the wire's `envelope_version` and never
/// signed anything: the envelope version is transport metadata a later build rewrites when it
/// re-wraps an op, so a signature over it would break on the first push after an upgrade. An op
/// field that must be signed later is APPENDED under a new tag; nothing already here moves.
pub const CANONICAL_FORM: &[u8] = b"nxs-op/2";

/// One operation in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub op_id: String,
    pub lamport: i64,
    pub site: i64,
    /// The product/reducer domain (`task` | `fact` | `message` …) — the axis the reducer registry
    /// dispatches on (spec §4.1).
    pub domain: String,
    pub target_kind: String,
    pub target_id: String,
    pub field: String,
    pub op_type: String,
    pub value: Option<String>,
    /// Who appended this op — stamped at append and never rewritten (forward-compat invariant 1,
    /// `6j6v.xsf3`). Attribution on an append-only log cannot be added after the fact, so this is
    /// never blank on a locally-emitted op: [`Store::emit`](crate::store::Store::emit) asserts it.
    ///
    /// **It is SELF-DECLARED, not authenticated.** The assert binds ops this replica emits; an op
    /// that arrived over sync carries whatever its origin wrote, including a name belonging to
    /// someone else, and [`Store::apply`](crate::store::Store::apply) neither checks nor rewrites
    /// it (§7 forbids dropping a foreign op, and there is nothing to check against yet). So this
    /// field is an audit trail among cooperating replicas today — do not treat it as proof of
    /// origin, and do not gate an action on it. The E4 auth slice (`6j6v.6aza`) is what turns it
    /// into an authenticated identity, by signing the op and verifying before the action; the
    /// field it will authenticate has to exist on every op from the start, which is why it is
    /// here now.
    pub author: String,
    pub wall_clock: String,
    /// The key that signed this op, as `ed25519:<base64url public key>` — a lookup hint, never an
    /// authority claim by itself: authority comes from [`sig`](Op::sig) checking out against this
    /// key, and from this replica trusting it (6j6v.pzkb). `None` on an op nobody signed — every op
    /// written before signing existed, and every op from a client that does not sign.
    pub key_id: Option<String>,
    /// The Ed25519 signature over [`canonical_bytes`](Op::canonical_bytes), base64url. Carried with
    /// the op everywhere it goes — the log is a signed audit log, re-verifiable by anyone, rather
    /// than one checked once at receipt and forgotten (owner decision, 2026-08-10).
    pub sig: Option<String>,
}

impl Op {
    /// Whether this op's Lamport number is one the fold and the clock may take into account — at
    /// most [`MAX_LAMPORT`] (6j6v.m19v). Only the top is bounded: a negative number cannot move a
    /// clock that only ever advances, and it loses every compare it enters, so it harms nothing.
    pub fn lamport_in_bound(&self) -> bool {
        self.lamport <= MAX_LAMPORT
    }

    /// The op's **canonical bytes** — the one deterministic, unambiguous encoding its signature is
    /// taken over (forward-compat invariant 3, 6j6v.xsf3; signing itself, 6j6v.pzkb).
    ///
    /// Computed from the FIELDS, never from received bytes: the JSON wire form has more than one
    /// valid spelling of the same op (`domain` is omitted for `task`, key order and whitespace are a
    /// serializer's choice), and a signature over bytes-as-received would depend on which peer
    /// re-serialized the op last.
    ///
    /// Shape: [`CANONICAL_FORM`], then every field in declaration order, each length-prefixed
    /// (8-byte big-endian) so no value can impersonate a field boundary. `value` carries a presence
    /// byte, keeping `None` and `Some("")` apart as every relay backend does. The signature pair
    /// ([`key_id`](Op::key_id), [`sig`](Op::sig)) is not covered — a signature cannot cover itself,
    /// and pointing the key id at another key only makes the signature fail.
    ///
    /// `nxs_sync::wire::WireOp::canonical_bytes` computes the same bytes for an op on the wire,
    /// without this crate (the relay does not link it); a test there holds the two equal.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        fn field(out: &mut Vec<u8>, bytes: &[u8]) {
            out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            out.extend_from_slice(bytes);
        }
        let mut out = Vec::new();
        out.extend_from_slice(CANONICAL_FORM);
        field(&mut out, self.op_id.as_bytes());
        field(&mut out, &self.lamport.to_be_bytes());
        field(&mut out, &self.site.to_be_bytes());
        field(&mut out, self.domain.as_bytes());
        field(&mut out, self.target_kind.as_bytes());
        field(&mut out, self.target_id.as_bytes());
        field(&mut out, self.field.as_bytes());
        field(&mut out, self.op_type.as_bytes());
        match &self.value {
            None => out.push(0),
            Some(v) => {
                out.push(1);
                field(&mut out, v.as_bytes());
            }
        }
        field(&mut out, self.author.as_bytes());
        field(&mut out, self.wall_clock.as_bytes());
        out
    }
}

/// Resolve the identity to stamp on an op's [`Op::author`] from a precedence chain, treating a
/// **blank** candidate as absent (forward-compat invariant 1, `6j6v.xsf3`).
///
/// Every product CLI resolves its actor the same way — an explicit per-product variable
/// (`NXF_ACTOR`/`NXM_ACTOR`/`NXC_ACTOR`/`NXS_ACTOR`), else `$USER`, else the binary's own name —
/// and `std::env::var` reports a var that is SET BUT EMPTY as `Ok("")`. Taken literally that
/// authors ops with no identity at all, permanently, on a log where attribution cannot be
/// backfilled. So the rule lives here, once, rather than four times in four `actor()` helpers:
/// blank (empty or all-whitespace) is not an identity, it is an absent one.
///
/// Pure over its inputs so the precedence is unit-testable without touching process env — the same
/// shape as the MCP server's `pick_actor` hybrid (#zxj), which already applied this rule at its own
/// seam while the CLIs did not.
pub fn resolve_author(primary: Option<String>, fallback: Option<String>, default: &str) -> String {
    fn present(candidate: Option<String>) -> Option<String> {
        candidate.filter(|s| !s.trim().is_empty())
    }
    present(primary)
        .or_else(|| present(fallback))
        .unwrap_or_else(|| default.to_string())
}

/// Validate an actor an EMBEDDER supplied, returning the stored (trimmed) form — the `Result`-shaped
/// half of the same rule [`resolve_author`] applies to a precedence chain.
///
/// The two exist for two different callers. A CLI RESOLVES: it owns the fallback chain, so a blank
/// candidate simply falls through to the next tier and there is nothing to report. An embedding app
/// PASSES an actor in as an argument: there is no next tier, so a blank one is a caller mistake and
/// has to come back as a rejection. `Store::emit`'s non-blank assert is the substrate's last-resort
/// guarantee against an unattributed op reaching an append-only log — it is not an input check, and
/// a public library call must not panic on an embedder's bad argument (review finding Code Quality
/// #1, PR #311). So every write seam that accepts an `actor` validates it here first, the same way
/// `nexus_flow_facade::validate::label` rejects a blank label rather than storing one.
pub fn validate_author(actor: &str) -> crate::error::Result<&str> {
    let trimmed = actor.trim();
    if !is_attributable(trimmed) {
        return Err(crate::error::NxfError::validation(
            "an actor must not be blank — every op records who wrote it",
        ));
    }
    Ok(trimmed)
}

/// Whether `author` actually NAMES someone: the one definition of "this op is attributable",
/// shared by the substrate's append-time assert, the embedder-facing [`validate_author`], and the
/// reducers that fold an author into a view's identity column (chat's `sender`/`opener`).
///
/// Blank is the obvious case. The subtler one is an author built only from characters that RENDER
/// as nothing — zero-width spaces and joiners, bidi marks, a BOM: `str::trim` strips none of them,
/// so `"\u{200B}"` passes a plain non-empty check and then displays as an empty name in every read
/// surface (PR #314 review, Integrity & Robustness #2). It is not an impersonation — none of these
/// can collide with a real `origin/handle` — but "every op records who wrote it" has to mean a
/// name a person can read, not one that merely occupies bytes.
///
/// Deliberately a hand-written set rather than a Unicode-category dependency: `char::is_control`
/// and `char::is_whitespace` already cover C0/C1 and every space separator, which leaves the
/// format characters below as the whole remaining gap.
pub fn is_attributable(author: &str) -> bool {
    author.chars().any(|c| !is_invisible(c))
}

/// True for a character that occupies no visible width — whitespace, a control, or a format
/// character. See [`is_attributable`].
fn is_invisible(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c,
            '\u{00AD}'                 // soft hyphen
            | '\u{061C}'               // arabic letter mark
            | '\u{180E}'               // mongolian vowel separator
            | '\u{200B}'..='\u{200F}'  // zero-width space/non-joiner/joiner, LRM, RLM
            | '\u{202A}'..='\u{202E}'  // bidi embedding + override
            | '\u{2060}'..='\u{2064}'  // word joiner, invisible operators
            | '\u{2066}'..='\u{2069}'  // bidi isolates
            | '\u{FEFF}'               // zero-width no-break space (BOM)
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_author_prefers_the_primary_variable() {
        assert_eq!(
            resolve_author(Some("alice".into()), Some("bob".into()), "nxf"),
            "alice"
        );
    }

    #[test]
    fn resolve_author_falls_back_then_defaults() {
        assert_eq!(resolve_author(None, Some("bob".into()), "nxf"), "bob");
        assert_eq!(resolve_author(None, None, "nxf"), "nxf");
    }

    #[test]
    fn a_blank_candidate_is_absent_not_an_identity() {
        // `NXF_ACTOR=` is `Ok("")` from `std::env::var`, not `Err(NotPresent)`. Taking it
        // literally would author every op of that session with no identity at all — the one
        // thing an append-only log can never repair (invariant 1, 6j6v.xsf3).
        assert_eq!(
            resolve_author(Some(String::new()), Some("bob".into()), "nxf"),
            "bob"
        );
        assert_eq!(resolve_author(Some("   ".into()), None, "nxf"), "nxf");
        assert_eq!(resolve_author(None, Some(String::new()), "nxf"), "nxf");
    }

    #[test]
    fn validate_author_rejects_a_blank_and_returns_the_stored_form() {
        // The embedder-facing half: no fallback tier to fall through to, so a blank actor is a
        // rejection, not a fall-through — and never the panic the substrate's assert would raise.
        assert_eq!(validate_author("alice").unwrap(), "alice");
        assert_eq!(validate_author("  alice  ").unwrap(), "alice", "trimmed");
        for blank in ["", " ", "\t\n "] {
            let err = validate_author(blank).expect_err("a blank actor is rejected");
            assert_eq!(err.kind, crate::error::ErrorKind::Validation);
        }
    }

    #[test]
    fn resolve_author_never_returns_a_blank() {
        for (p, f) in [
            (None, None),
            (Some(String::new()), Some(String::new())),
            (Some("\t \n".into()), Some("  ".into())),
        ] {
            assert!(!resolve_author(p, f, "nxf").trim().is_empty());
        }
    }
}
