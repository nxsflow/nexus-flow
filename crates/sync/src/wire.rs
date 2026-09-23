//! The versioned wire envelope. Every op travels as a [`WireOp`]; the relay treats its
//! CONTENT as **opaque** — it stores every field of the envelope, indexes only by the assigned
//! sequence, and NEVER rejects an op for an unknown `target_kind` / `op_type` /
//! `envelope_version` (the server half of forward-compat, spec §4.1 / §7).
//!
//! That opacity covers unknown *values* and — since `6j6v.5crb` — unknown *fields*. It used to
//! stop at values: the relay decomposes the envelope into named columns/attributes, so a field a
//! newer peer added was dropped in transit by an older relay, and durably (the relay reserializes
//! from its own columns). Adding a field was additive for PEERS (serde defaults it, as `domain`
//! did — aye.1.4) but silently lossy across a mixed version stand, which self-hosted relays make
//! the normal case. [`WireOp::extra`] closes that: what this build does not know rides through as
//! a catch-all, and each backend persists it in one passthrough column/attribute.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The envelope version this build emits. A forward-compat discriminator only: a peer
/// MUST preserve a higher version it does not understand rather than reject it (§7).
pub const ENVELOPE_VERSION: u16 = 1;

/// The default op domain — flow's. Threaded through serde so a payload that predates the
/// `domain` field (an older peer) deserializes as a `task` op (the only domain flow ever
/// emitted), keeping the envelope additive without an `ENVELOPE_VERSION` bump.
fn default_domain() -> String {
    "task".to_string()
}

/// True for the default (`task`) domain — used to omit `domain` from the wire for flow ops,
/// so a task op's bytes are byte-identical to the pre-aye.1.4 envelope (and the relay opacity
/// round-trip of a domain-less future op stays verbatim). Foreign domains always serialize.
fn is_task_domain(domain: &str) -> bool {
    domain == "task"
}

/// One op on the wire. The field set mirrors `core::Op` plus the [`envelope_version`]
/// discriminator. All payload fields are plain strings/ints, so an op carrying an
/// unknown `op_type` or `target_kind` deserializes and re-serializes losslessly — that
/// is what makes the relay's opacity (and the client's store-then-skip, §7) possible.
///
/// [`envelope_version`]: WireOp::envelope_version
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireOp {
    pub envelope_version: u16,
    pub op_id: String,
    pub lamport: i64,
    pub site: i64,
    /// The product/reducer domain (`task`/`fact`/`message`, spec §4.1). Carried opaquely so a
    /// foreign op survives a relay round-trip with its domain intact (aye.1.4). Defaults to
    /// `task` for an older peer's domain-less payload, and is omitted from the wire when it IS
    /// `task` so flow's envelope bytes are unchanged.
    #[serde(default = "default_domain", skip_serializing_if = "is_task_domain")]
    pub domain: String,
    pub target_kind: String,
    pub target_id: String,
    pub field: String,
    pub op_type: String,
    pub value: Option<String>,
    pub author: String,
    pub wall_clock: String,
    /// Every envelope field this build does not know, carried verbatim (`6j6v.5crb`).
    ///
    /// A newer peer adds a field; an older peer parses it into here instead of onto the floor,
    /// and re-serializes it back into the envelope unchanged. That is what lets the E4 auth slice
    /// (`6j6v.6aza`) introduce `sig` additively: a signed op pushed through an older relay comes
    /// out the far side still signed, rather than silently downgraded to unsigned on the one path
    /// the signature exists to protect.
    ///
    /// A `BTreeMap` (not a `HashMap`) so re-serialization is deterministic — the relay stores this
    /// as one JSON blob per op and must not rewrite it differently on every read.
    ///
    /// **The signature pair lives here too, on purpose** (6j6v.pzkb): [`KEY_ID_FIELD`] and
    /// [`SIG_FIELD`], read through [`key_id`](WireOp::key_id) and [`sig`](WireOp::sig). This build
    /// knows them, but they stay in the passthrough rather than becoming named fields: every relay
    /// since 5crb carries the passthrough with no change to its storage, where a named field would
    /// need a new column in each of the relay's three backends — and a relay that dropped it would
    /// be exactly the silent downgrade signing exists to prevent. Neither is signed: a signature
    /// cannot cover itself, and the key id is a lookup hint.
    ///
    /// **Not covered by [`canonical_bytes`](WireOp::canonical_bytes)**, deliberately: a signature
    /// cannot cover itself, and two builds that must agree on the signed bytes disagree on which
    /// fields are "unknown". A field graduates into the signed bytes by becoming a NAMED field
    /// appended to the canonical tail under a bumped [`CANONICAL_FORM`].
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// The envelope field carrying the key id of the key that signed the op (6j6v.pzkb) — in
/// [`WireOp::extra`], see there for why.
pub const KEY_ID_FIELD: &str = "key_id";

/// The envelope field carrying the op's signature (6j6v.pzkb) — in [`WireOp::extra`].
pub const SIG_FIELD: &str = "sig";

/// Tag of the canonical encoding produced by [`WireOp::canonical_bytes`] — the same bytes as
/// `nxs_foundation::model::Op::canonical_bytes` (a test under `engine` holds the two equal; the
/// relay does not link the foundation, so the encoding is written here a second time). Part of the
/// signed bytes, so a signature is bound to the encoding that produced it.
///
/// `nxs-op/2`: `nxs-op/1` also covered [`envelope_version`](WireOp::envelope_version), and never
/// signed anything. The envelope version is transport metadata a later build rewrites when it
/// re-wraps an op, so a signature over it would break on the first push after an upgrade.
pub const CANONICAL_FORM: &[u8] = b"nxs-op/2";

impl WireOp {
    /// The op's **canonical byte encoding** — the deterministic, unambiguous serialization an
    /// op signature is taken over (forward-compat invariant 3, `6j6v.xsf3`). Signing itself is
    /// E4's (`6j6v.6aza`); what has to exist now is an encoding that means exactly one thing.
    ///
    /// It is deliberately NOT the JSON wire form. JSON gives this envelope more than one valid
    /// encoding of the same op — `domain` is omitted for `task` and present otherwise, key order
    /// and whitespace are a serializer's choice — so "sign the bytes you received" would make a
    /// signature depend on which peer re-serialized it last. This encoding is computed from the
    /// FIELDS instead, so it is identical no matter which wire form the op arrived in.
    ///
    /// Shape: the [`CANONICAL_FORM`] tag, then every op field in declaration order, each
    /// length-prefixed (8-byte big-endian) so no value can impersonate a field boundary — two
    /// distinct ops can never encode to the same bytes. `value` carries a presence byte, keeping
    /// `None` and `Some("")` distinct, exactly as the relay backends already keep them.
    ///
    /// [`envelope_version`](WireOp::envelope_version) is NOT covered: it describes the envelope,
    /// not the op, and a build re-wraps an op in its own version when it pushes it. A later version
    /// that changes what an op MEANS changes the tag instead, so an old signature can never be read
    /// as covering the new meaning.
    ///
    /// A future additive field (`sig` itself excluded — a signature cannot cover itself) is
    /// APPENDED to the tail under a bumped tag; nothing already here ever moves. [`extra`] is
    /// deliberately NOT hashed: it holds exactly the fields a build does not understand, so two
    /// peers signing over it would disagree on the bytes the moment their versions differ.
    ///
    /// [`extra`]: WireOp::extra
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

    /// The key id this op carries in its envelope ([`KEY_ID_FIELD`]), if it carries one as text.
    pub fn key_id(&self) -> Option<&str> {
        self.extra.get(KEY_ID_FIELD).and_then(|v| v.as_str())
    }

    /// The signature this op carries in its envelope ([`SIG_FIELD`]), if it carries one as text.
    pub fn sig(&self) -> Option<&str> {
        self.extra.get(SIG_FIELD).and_then(|v| v.as_str())
    }

    /// Whether the envelope carries any part of a signature pair, readable or not.
    pub fn carries_a_signature(&self) -> bool {
        self.extra.contains_key(KEY_ID_FIELD) || self.extra.contains_key(SIG_FIELD)
    }
}

/// Take one half of the signature pair out of the passthrough. A value that is not text is kept
/// as its JSON spelling rather than dropped: it is still a signature somebody sent, which the
/// receiver must judge `invalid`, never `unsigned`.
#[cfg(feature = "engine")]
fn take_signature_part(
    extra: &mut BTreeMap<String, serde_json::Value>,
    name: &str,
) -> Option<String> {
    extra.remove(name).map(|v| match v {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    })
}

// The core <-> wire conversions live behind `engine`: only the client (which owns a
// core store) needs them. The relay handles WireOp opaquely and never touches core.
#[cfg(feature = "engine")]
impl From<&nexus_flow_core::model::Op> for WireOp {
    fn from(op: &nexus_flow_core::model::Op) -> WireOp {
        WireOp {
            envelope_version: ENVELOPE_VERSION,
            op_id: op.op_id.clone(),
            lamport: op.lamport,
            site: op.site,
            domain: op.domain.clone(),
            target_kind: op.target_kind.clone(),
            target_id: op.target_id.clone(),
            field: op.field.clone(),
            op_type: op.op_type.clone(),
            value: op.value.clone(),
            author: op.author.clone(),
            wall_clock: op.wall_clock.clone(),
            // A locally-authored op has nothing this build does not know, by construction — only
            // its signature pair, which travels in the passthrough (see `WireOp::extra`).
            extra: [(KEY_ID_FIELD, &op.key_id), (SIG_FIELD, &op.sig)]
                .into_iter()
                .filter_map(|(name, part)| {
                    part.as_ref()
                        .map(|v| (name.to_string(), serde_json::Value::String(v.clone())))
                })
                .collect(),
        }
    }
}

#[cfg(feature = "engine")]
impl From<WireOp> for nexus_flow_core::model::Op {
    /// Drops `envelope_version` (transport-only metadata) and reconstructs the core op.
    ///
    /// The envelope carries `domain` (aye.1.4), so a foreign `fact`/`message` op keeps its
    /// domain across a relay round-trip instead of being coerced to `task`. A domain-less
    /// payload from an older peer defaults to `task` (serde), so flow stays byte-identical.
    ///
    /// The signature pair comes out of [`extra`](WireOp::extra) onto the op's own
    /// `key_id`/`sig`, where the receiving store verifies it (6j6v.pzkb) — the owner's decision of
    /// 2026-08-10: the checked signature is carried in the log, not checked at receipt and dropped.
    ///
    /// The rest of the catch-all is dropped here — the core op log has no column for a field this
    /// build does not understand, and §7's store-don't-fold is about unknown VALUES in known
    /// columns. Its job ends at the transport: it keeps an additive field intact
    /// `push -> store -> read_since -> pull`, which is what an older RELAY between two newer peers
    /// would otherwise break. This loses nothing: the engine pushes only local-origin ops, so a
    /// pulled foreign op is never re-emitted from here.
    fn from(mut w: WireOp) -> nexus_flow_core::model::Op {
        let key_id = take_signature_part(&mut w.extra, KEY_ID_FIELD);
        let sig = take_signature_part(&mut w.extra, SIG_FIELD);
        nexus_flow_core::model::Op {
            op_id: w.op_id,
            lamport: w.lamport,
            site: w.site,
            domain: w.domain,
            target_kind: w.target_kind,
            target_id: w.target_id,
            field: w.field,
            op_type: w.op_type,
            value: w.value,
            author: w.author,
            wall_clock: w.wall_clock,
            key_id,
            sig,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WireOp {
        WireOp {
            envelope_version: ENVELOPE_VERSION,
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
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn round_trip_serde_preserves_all_fields() {
        let op = sample();
        let json = serde_json::to_string(&op).unwrap();
        let back: WireOp = serde_json::from_str(&json).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn unknown_envelope_version_and_op_type_survive_round_trip() {
        // A newer peer's op: envelope_version + op_type the current build does not know.
        // The relay (and any opaque carrier) must preserve them verbatim, never drop.
        let raw = serde_json::json!({
            "envelope_version": 999,
            "op_id": "01HFUTURE0000000000000000",
            "lamport": 3,
            "site": 1,
            "target_kind": "gizmo",
            "target_id": "zz99.abcd",
            "field": "sparkle",
            "op_type": "future_set",
            "value": "v",
            "author": "bob",
            "wall_clock": ""
        });
        let op: WireOp = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(op.envelope_version, 999);
        assert_eq!(op.op_type, "future_set");
        assert_eq!(op.target_kind, "gizmo");
        // Re-serializing yields the same payload — nothing was lost.
        assert_eq!(serde_json::to_value(&op).unwrap(), raw);
    }

    #[cfg(feature = "engine")]
    #[test]
    fn op_to_wire_and_back_is_identity() {
        use nexus_flow_core::model::Op;
        let op = Op {
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
        };
        let wire = WireOp::from(&op);
        assert_eq!(wire.envelope_version, ENVELOPE_VERSION);
        assert_eq!(
            Op::from(wire),
            op,
            "wire envelope round-trips a core Op losslessly"
        );
    }

    #[cfg(feature = "engine")]
    #[test]
    fn foreign_domain_survives_a_wire_round_trip() {
        // aye.1.4: a foreign `fact` op (a future memory product's) must keep its domain across
        // a relay round-trip. Before this, `From<WireOp>` hard-coded `task`, so the §7
        // store-don't-fold forward-compat held locally but was LOST over sync.
        use nexus_flow_core::model::Op;
        let fact = Op {
            op_id: "01J0FAKEULID0000000000000".into(),
            lamport: 5,
            site: 2,
            domain: "fact".into(),
            target_kind: "fact".into(),
            target_id: "f.1".into(),
            field: "body".into(),
            op_type: "set".into(),
            value: Some("hello".into()),
            author: "u".into(),
            wall_clock: String::new(),
            key_id: None,
            sig: None,
        };
        let wire = WireOp::from(&fact);
        assert_eq!(wire.domain, "fact", "the envelope carries the op's domain");
        assert_eq!(
            Op::from(wire),
            fact,
            "domain survives the round-trip — never coerced to task, never dropped"
        );
    }

    // ---- invariant 3 (6j6v.xsf3): the envelope has a canonical encoding, and room for a sig ----

    #[test]
    fn canonical_bytes_are_deterministic_and_independent_of_the_wire_form() {
        // The property that makes a signature verifiable at the far end: a `task` op omits
        // `domain` on the wire while a `fact` op carries it, and a peer may re-serialize an op
        // any number of times on the way. None of that may change what was signed.
        let op = sample();
        assert_eq!(op.canonical_bytes(), op.canonical_bytes(), "deterministic");

        let json = serde_json::to_string(&op).unwrap();
        assert!(
            !json.contains("domain"),
            "a task op omits `domain` from the wire — the ambiguity this encoding sidesteps"
        );
        let from_omitted: WireOp = serde_json::from_str(&json).unwrap();
        let mut explicit = serde_json::from_str::<serde_json::Value>(&json).unwrap();
        explicit["domain"] = serde_json::Value::String("task".into());
        let from_explicit: WireOp = serde_json::from_value(explicit).unwrap();
        assert_eq!(
            from_omitted.canonical_bytes(),
            from_explicit.canonical_bytes(),
            "the same op encodes identically whether or not the wire spelled `domain` out"
        );
        assert_eq!(op.canonical_bytes(), from_omitted.canonical_bytes());
    }

    #[test]
    fn canonical_bytes_separate_every_distinct_op() {
        // Length-prefixing is what stops a value from impersonating a field boundary: without
        // it, `author="a"` + `wall_clock="bc"` and `author="ab"` + `wall_clock="c"` would sign
        // the same bytes, and a signature could be replayed onto an op it never covered.
        let base = sample();
        let mut shifted = base.clone();
        shifted.author = "alic".into();
        shifted.wall_clock = "e".into();
        let mut joined = base.clone();
        joined.author = "alice".into();
        joined.wall_clock = String::new();
        assert_ne!(shifted.canonical_bytes(), joined.canonical_bytes());

        // And every field is actually covered — a change anywhere changes the bytes.
        let mut seen = std::collections::HashSet::new();
        seen.insert(base.canonical_bytes());
        let variants = {
            let mut v = Vec::new();
            for mutate in [
                |o: &mut WireOp| o.op_id.push('x'),
                |o: &mut WireOp| o.lamport += 1,
                |o: &mut WireOp| o.site += 1,
                |o: &mut WireOp| o.domain = "fact".into(),
                |o: &mut WireOp| o.target_kind.push('x'),
                |o: &mut WireOp| o.target_id.push('x'),
                |o: &mut WireOp| o.field.push('x'),
                |o: &mut WireOp| o.op_type.push('x'),
                |o: &mut WireOp| o.value = None,
                |o: &mut WireOp| o.value = Some(String::new()),
                |o: &mut WireOp| o.author.push('x'),
                |o: &mut WireOp| o.wall_clock.push('x'),
            ] {
                let mut o = base.clone();
                mutate(&mut o);
                v.push(o);
            }
            v
        };
        for v in variants {
            assert!(
                seen.insert(v.canonical_bytes()),
                "every field is covered and distinct: {v:?}"
            );
        }
    }

    /// The envelope version describes the envelope, not the op, and a build re-wraps an op in its
    /// own version when it pushes it — so it is deliberately not signed (`nxs-op/2`). An op signed
    /// before an upgrade and pushed after it must still verify.
    #[test]
    fn canonical_bytes_do_not_cover_the_envelope_version() {
        let base = sample();
        let mut rewrapped = base.clone();
        rewrapped.envelope_version = ENVELOPE_VERSION + 1;
        assert_eq!(base.canonical_bytes(), rewrapped.canonical_bytes());
        assert!(base.canonical_bytes().starts_with(b"nxs-op/2"));
    }

    #[test]
    fn canonical_bytes_keep_an_absent_value_apart_from_an_empty_one() {
        // `None` and `Some("")` are different ops, and every relay backend already stores them
        // apart (DynamoDB omits the attribute entirely for `None`). The signature must agree.
        let mut absent = sample();
        absent.value = None;
        let mut empty = sample();
        empty.value = Some(String::new());
        assert_ne!(absent.canonical_bytes(), empty.canonical_bytes());
    }

    #[test]
    fn an_unknown_envelope_field_survives_the_envelope_verbatim() {
        // 6j6v.5crb: the envelope is opaque to unknown VALUES (the test above) *and* to unknown
        // FIELDS. Before the catch-all, serde dropped a field this build does not know, and the
        // drop was durable across the relay — so an additive `sig` from a newer client would be
        // stripped by an older relay and the receiving client would see an unsigned op: a silent
        // downgrade on exactly the path signing exists to protect (6j6v.6aza, point 3).
        let mut raw = serde_json::to_value(sample()).unwrap();
        raw["sig"] = serde_json::Value::String("ed25519:deadbeef".into());
        let parsed: WireOp = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(
            parsed.extra.get("sig").and_then(|v| v.as_str()),
            Some("ed25519:deadbeef"),
            "an unknown field lands in the catch-all instead of the floor"
        );
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            raw,
            "and re-serializes back into the envelope, byte-for-byte the same payload"
        );
    }

    #[test]
    fn the_catch_all_holds_only_fields_this_build_does_not_know() {
        // The catch-all must not shadow a known field: `domain` is spelled out on the wire for a
        // foreign domain, and it has to land on `WireOp::domain` — not in `extra` (where the
        // relay would store it twice and `canonical_bytes` would miss it).
        let mut op = sample();
        op.domain = "fact".into();
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains("\"domain\":\"fact\""));
        let back: WireOp = serde_json::from_str(&json).unwrap();
        assert_eq!(back.domain, "fact");
        assert!(
            back.extra.is_empty(),
            "a field this build DOES know never lands in the catch-all"
        );
        assert_eq!(back, op);
    }

    #[test]
    fn canonical_bytes_do_not_cover_the_catch_all() {
        // Deliberate, and the reason it is pinned: `sig` itself arrives through `extra` until
        // 6j6v.6aza promotes it to a named field, and a signature cannot cover itself. A future
        // field is folded into the signed bytes by APPENDING it to the tail under a bumped
        // CANONICAL_FORM tag (see `canonical_bytes`) — never by hashing the catch-all, whose key
        // set differs between two builds that must agree on the same bytes.
        let base = sample();
        let mut with_sig = base.clone();
        with_sig
            .extra
            .insert("sig".into(), serde_json::Value::String("ed25519:x".into()));
        assert_eq!(
            base.canonical_bytes(),
            with_sig.canonical_bytes(),
            "the catch-all is carried, not signed"
        );
    }

    #[test]
    fn the_signature_pair_is_read_from_the_passthrough_only_when_it_is_text() {
        let mut op = sample();
        assert!(!op.carries_a_signature());
        op.extra.insert(
            KEY_ID_FIELD.into(),
            serde_json::Value::String("ed25519:k".into()),
        );
        op.extra.insert(SIG_FIELD.into(), serde_json::json!(7));
        assert_eq!(op.key_id(), Some("ed25519:k"));
        assert_eq!(op.sig(), None, "not text");
        assert!(op.carries_a_signature());
    }

    /// The two spellings of one encoding — this one, for the relay, which does not link the
    /// foundation, and `Op::canonical_bytes`, which the foundation signs and verifies — must be the
    /// same bytes, or a signature made on one side fails on the other.
    #[cfg(feature = "engine")]
    #[test]
    fn the_wire_and_the_op_spell_the_same_canonical_bytes() {
        use nexus_flow_core::model::Op;
        assert_eq!(CANONICAL_FORM, nxs_foundation::model::CANONICAL_FORM);
        let mut shapes = vec![sample()];
        let mut fact = sample();
        fact.domain = "fact".into();
        fact.value = None;
        shapes.push(fact);
        let mut empty = sample();
        empty.value = Some(String::new());
        empty.lamport = -3;
        empty.wall_clock = "2026-09-22T10:00:00Z".into();
        shapes.push(empty);
        for wire in shapes {
            assert_eq!(
                wire.canonical_bytes(),
                Op::from(wire.clone()).canonical_bytes()
            );
        }
    }

    #[cfg(feature = "engine")]
    #[test]
    fn a_signed_op_keeps_its_signature_pair_across_the_wire() {
        use nexus_flow_core::model::Op;
        let mut op = Op::from(sample());
        op.key_id = Some("ed25519:key".into());
        op.sig = Some("signature".into());
        let wire = WireOp::from(&op);
        assert_eq!(
            (wire.key_id(), wire.sig()),
            (Some("ed25519:key"), Some("signature"))
        );
        let json = serde_json::to_string(&wire).unwrap();
        let back: WireOp = serde_json::from_str(&json).unwrap();
        assert_eq!(
            Op::from(back),
            op,
            "and back onto the op, nothing else in the catch-all"
        );
    }

    /// A signature pair that is not text is still a signature somebody sent — it must arrive as
    /// something the store judges `invalid`, never as the absence that reads `unsigned`.
    #[cfg(feature = "engine")]
    #[test]
    fn a_signature_that_is_not_text_arrives_as_one_not_as_none() {
        use nexus_flow_core::model::Op;
        let mut wire = sample();
        wire.extra.insert(SIG_FIELD.into(), serde_json::json!(7));
        let op = Op::from(wire);
        assert_eq!(op.sig.as_deref(), Some("7"));
        assert_eq!(op.key_id, None);
    }

    #[test]
    fn wireop_without_a_domain_field_deserializes_as_task() {
        // An older peer's payload predates the `domain` field; serde must default it to `task`
        // (flow's domain) so the envelope stays forward/backward compatible without a version
        // bump (owner directive: sync is deployed nowhere, keep it simple).
        let raw = serde_json::json!({
            "envelope_version": ENVELOPE_VERSION,
            "op_id": "01HFAKE0000000000000000001",
            "lamport": 7,
            "site": 42,
            "target_kind": "item",
            "target_id": "ab12.7x3k",
            "field": "title",
            "op_type": "set",
            "value": "hello",
            "author": "alice",
            "wall_clock": ""
        });
        let op: WireOp = serde_json::from_value(raw).unwrap();
        assert_eq!(
            op.domain, "task",
            "a domain-less op defaults to the task domain"
        );
    }
}
