//! Minisign verification of release artifacts (nexus-flow-85y.5).
//!
//! The guarantee is the Tauri-updater one carried over to the CLI (spec §5.3, §7.2): **no
//! installation without a valid signature.** Every release tarball ships a detached
//! `.minisig`; `nxf` verifies it against a minisign public key **compiled into the binary**,
//! so an attacker who controls the download/CDN cannot serve a tampered build — they would
//! also need the private key, which never leaves CI + the operator's offline backup.
//!
//! The public key is injected at build time via `NXF_MINISIGN_PUBKEY` (the release workflow,
//! 85y.13). It is deliberately NOT a committed file: `.gitignore` blocks `*.pub`/`*.key` as a
//! belt-and-suspenders against ever committing signing material, and a build-time var keeps
//! even the (public, harmless) key out of the tree. A build with no key embedded **fails
//! closed**: `verify` returns `NoEmbeddedKey` rather than silently accepting — "cannot prove
//! validity" must never read as "valid".
//!
//! `verify_with_key` takes the key as a parameter and is therefore pure and unit-testable
//! against an ephemeral keypair (see the tests); `verify` is the thin wrapper that uses the
//! compiled-in key. The download + sha256 + atomic-replace flow that calls this lives in
//! `nxs self-update` (85y.18, nexus-flow-gel) and `install.sh` (85y.13); this module owns
//! verification only.
//!
//! ## Operator action (one-time, manual — see docs/specs/release-management.md §12.4)
//!
//! The real keypair does not exist yet. The operator must, in order:
//!
//! - mint the keypair: `minisign -G -p nxf.pub -s nxf.key` (or `rsign generate`);
//! - store the secret key + its password as GitHub Secrets `NXF_MINISIGN_PRIVATE_KEY` and `NXF_MINISIGN_KEY_PASSWORD` — set the password via `printf '%s'` with NO trailing newline (the documented reference foot-gun) — plus an offline backup (losing it = no future update path, since installed binaries trust only the baked key);
//! - expose the public key's base64 line to the release build as `NXF_MINISIGN_PUBKEY`.
//!
//! Until then this build verifies nothing (fail-closed) — which is the correct default.

use minisign_verify::{PublicKey, Signature};

/// The minisign public key text compiled into this build, or `None` when `NXF_MINISIGN_PUBKEY`
/// was unset at build time. Read through [`embedded_public_key`] (which also treats an
/// empty/whitespace value as absent).
const EMBEDDED_PUBLIC_KEY: Option<&str> = option_env!("NXF_MINISIGN_PUBKEY");

/// Why a verification did not succeed. Every variant is a refusal — there is no "maybe".
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VerifyError {
    /// The binary was built without a public key ⇒ it cannot verify anything (fail-closed).
    /// User-facing wording (eprg): name the likely cause (an unsigned / locally-built binary) and
    /// the concrete fix (install an official signed release). The maintainer key-provisioning
    /// runbook stays in this module's doc header (release-management.md §12.4), not on the line the
    /// end user sees.
    #[error(
        "this build is unsigned — it has no embedded minisign public key, so it cannot verify \
         release signatures. It is a locally-built or development build; install an official \
         signed release to verify or self-update: curl -fsSL https://nxsflow.com/nxs/install.sh | sh"
    )]
    NoEmbeddedKey,
    /// The embedded/supplied public key is not a valid minisign key.
    #[error("minisign public key is malformed: {0}")]
    MalformedPublicKey(String),
    /// The supplied `.minisig` is not a valid minisign signature.
    #[error("minisign signature is malformed: {0}")]
    MalformedSignature(String),
    /// The signature is well-formed but does not match the artifact under the trusted key.
    #[error("signature does not match the artifact under the trusted key")]
    BadSignature,
}

/// Normalize the build-time key: an empty or whitespace-only value is treated as "no key"
/// (a CI misconfiguration that set the var to "" must fail closed, not parse-error obscurely).
pub(crate) fn embedded_public_key() -> Option<&'static str> {
    match EMBEDDED_PUBLIC_KEY {
        Some(k) if !k.trim().is_empty() => Some(k),
        _ => None,
    }
}

/// Parse a minisign public key from either a full `minisign.pub` (comment line + key line) or
/// a bare base64 key line — the operator may wire either form into `NXF_MINISIGN_PUBKEY`.
fn parse_public_key(public_key: &str) -> Result<PublicKey, VerifyError> {
    let s = public_key.trim();
    // `decode` wants the two-line .pub form; `from_base64` wants the bare key line. Try the
    // richer form first, fall back to the bare line.
    PublicKey::decode(s)
        .or_else(|_| PublicKey::from_base64(s))
        .map_err(|e| VerifyError::MalformedPublicKey(e.to_string()))
}

/// Verify `message` against the detached minisign signature `signature_minisig` (the full
/// `.minisig` text) under an explicit `public_key`. Pure — the key is a parameter, so the
/// trust decision is testable with an ephemeral keypair. `allow_legacy = false`: only modern
/// prehashed signatures are accepted, which is exactly what the release pipeline emits.
pub fn verify_with_key(
    public_key: &str,
    message: &[u8],
    signature_minisig: &str,
) -> Result<(), VerifyError> {
    let pk = parse_public_key(public_key)?;
    let sig = Signature::decode(signature_minisig)
        .map_err(|e| VerifyError::MalformedSignature(e.to_string()))?;
    pk.verify(message, &sig, false)
        .map_err(|_| VerifyError::BadSignature)
}

/// Verify `message` against `signature_minisig` using the key compiled into this binary.
/// Fails closed with [`VerifyError::NoEmbeddedKey`] when the build carries no key.
pub fn verify(message: &[u8], signature_minisig: &str) -> Result<(), VerifyError> {
    let key = embedded_public_key().ok_or(VerifyError::NoEmbeddedKey)?;
    verify_with_key(key, message, signature_minisig)
}

/// `nxf verify-signature --file <path> --signature <path.minisig>` (hidden): verify a local
/// file against a detached minisign signature using the compiled-in key. This is the
/// runtime-verifiable surface of the embedded key; `install.sh` and `nxs self-update` (85y.18)
/// reuse the same `verify` primitive.
pub fn verify_command(json: bool, file: &str, signature_path: &str) -> crate::error::Result<()> {
    use crate::error::NxfError;
    let message = std::fs::read(file).map_err(|e| NxfError::io(format!("reading {file}: {e}")))?;
    let sig = std::fs::read_to_string(signature_path)
        .map_err(|e| NxfError::io(format!("reading {signature_path}: {e}")))?;

    verify(&message, &sig).map_err(to_nxf_error)?;
    if json {
        println!("{}", serde_json::json!({ "verified": true, "file": file }));
    } else {
        println!("verified {file}");
    }
    Ok(())
}

/// Map a verification failure onto the shared error envelope. A malformed `.minisig` is bad
/// *input* (validation); everything else is a trust failure (verification).
fn to_nxf_error(e: VerifyError) -> crate::error::NxfError {
    use crate::error::NxfError;
    match e {
        VerifyError::MalformedSignature(_) => NxfError::validation(e.to_string()),
        _ => NxfError::verification(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minisign::{sign, KeyPair};
    use std::io::Cursor;

    /// Mint an ephemeral keypair and sign `message`, returning `(public_key_base64, minisig)`.
    /// Uses the full `minisign` crate (a TEST-only dev-dependency) so the verifier is exercised
    /// against a real signature without ever touching the production key.
    fn sign_with_fresh_key(message: &[u8]) -> (String, String) {
        let KeyPair { pk, sk } = KeyPair::generate_unencrypted_keypair().unwrap();
        let sig_box = sign(
            None,
            &sk,
            Cursor::new(message),
            Some("trusted"),
            Some("untrusted"),
        )
        .unwrap();
        (pk.to_base64(), sig_box.into_string())
    }

    #[test]
    fn verifies_a_genuine_signature() {
        let msg = b"nxf_0.2.0_linux-x86_64.tar.gz contents";
        let (pk, sig) = sign_with_fresh_key(msg);
        assert_eq!(verify_with_key(&pk, msg, &sig), Ok(()));
    }

    #[test]
    fn accepts_the_full_pub_file_form_as_well_as_the_bare_key_line() {
        let msg = b"payload";
        let (pk_b64, sig) = sign_with_fresh_key(msg);
        // Bare base64 line (above) and the full two-line minisign.pub form must both verify.
        let full_pub = format!("untrusted comment: minisign public key\n{pk_b64}");
        assert_eq!(verify_with_key(&full_pub, msg, &sig), Ok(()));
    }

    #[test]
    fn rejects_a_tampered_artifact() {
        let (pk, sig) = sign_with_fresh_key(b"the original bytes");
        assert_eq!(
            verify_with_key(&pk, b"the original bytes (tampered)", &sig),
            Err(VerifyError::BadSignature),
        );
    }

    #[test]
    fn rejects_a_signature_from_a_different_key() {
        let msg = b"payload";
        let (_pk_a, sig_a) = sign_with_fresh_key(msg);
        let (pk_b, _sig_b) = sign_with_fresh_key(msg);
        // A valid signature, but not made by the key we trust ⇒ refuse.
        assert_eq!(
            verify_with_key(&pk_b, msg, &sig_a),
            Err(VerifyError::BadSignature)
        );
    }

    #[test]
    fn rejects_a_malformed_public_key() {
        let (_pk, sig) = sign_with_fresh_key(b"x");
        match verify_with_key("not-a-key", b"x", &sig) {
            Err(VerifyError::MalformedPublicKey(_)) => {}
            other => panic!("expected MalformedPublicKey, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_malformed_signature() {
        let (pk, _sig) = sign_with_fresh_key(b"x");
        match verify_with_key(&pk, b"x", "this is not a minisig") {
            Err(VerifyError::MalformedSignature(_)) => {}
            other => panic!("expected MalformedSignature, got {other:?}"),
        }
    }

    #[test]
    fn embedded_key_normalization_treats_empty_as_absent() {
        // The pure normalization rule (empty/whitespace ⇒ None) that `embedded_public_key`
        // applies to the compiled-in value.
        let norm = |k: Option<&str>| matches!(k, Some(s) if !s.trim().is_empty());
        assert!(!norm(None));
        assert!(!norm(Some("")));
        assert!(!norm(Some("   \n")));
        assert!(norm(Some("RWQ...")));
    }

    #[test]
    fn verify_fails_closed_without_an_embedded_key() {
        // The test binary is built without NXF_MINISIGN_PUBKEY, so the embedded key is absent
        // and `verify` must refuse rather than accept (the core fail-closed guarantee).
        assert_eq!(embedded_public_key(), None);
        let (_pk, sig) = sign_with_fresh_key(b"x");
        assert_eq!(verify(b"x", &sig), Err(VerifyError::NoEmbeddedKey));

        // eprg: the message the user actually sees must be actionable — it names the cause and
        // the fix, and never points at a repo-internal document.
        let msg = VerifyError::NoEmbeddedKey.to_string();
        assert!(
            !msg.contains("release-management") && !msg.contains(".md"),
            "no repo-internal path in the user-facing message: {msg}"
        );
        assert!(msg.contains("unsigned"), "names the likely cause: {msg}");
        assert!(
            msg.contains("install.sh"),
            "points to the official install: {msg}"
        );
    }

    #[test]
    fn to_nxf_error_pins_the_error_kind_classification() {
        use crate::error::ErrorKind;
        // A malformed `.minisig` is bad *input* ⇒ validation; every other failure is a trust
        // decision ⇒ verification. This mapping is the wired CLI contract and must not drift.
        assert_eq!(
            to_nxf_error(VerifyError::MalformedSignature("x".into())).kind,
            ErrorKind::Validation,
        );
        assert_eq!(
            to_nxf_error(VerifyError::NoEmbeddedKey).kind,
            ErrorKind::Verification
        );
        assert_eq!(
            to_nxf_error(VerifyError::BadSignature).kind,
            ErrorKind::Verification
        );
        assert_eq!(
            to_nxf_error(VerifyError::MalformedPublicKey("x".into())).kind,
            ErrorKind::Verification,
        );
    }

    #[test]
    fn verify_command_reports_io_for_missing_inputs() {
        use crate::error::ErrorKind;
        let dir = tempfile::tempdir().unwrap();
        let missing_file = dir.path().join("nope.bin");
        let missing_sig = dir.path().join("nope.minisig");
        // No artifact ⇒ io.
        let e = verify_command(
            true,
            missing_file.to_str().unwrap(),
            missing_sig.to_str().unwrap(),
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Io);

        // Artifact present, signature missing ⇒ io (the second read fails).
        let art = dir.path().join("art.bin");
        std::fs::write(&art, b"bytes").unwrap();
        let e =
            verify_command(true, art.to_str().unwrap(), missing_sig.to_str().unwrap()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Io);
    }

    #[test]
    fn verify_command_fails_closed_on_the_wired_surface() {
        use crate::error::ErrorKind;
        // The actual `nxf verify-signature` path: read a real artifact + a real signature, then
        // verify. The test binary has no embedded key, so the wired command must surface a
        // `verification` error (NoEmbeddedKey) rather than accept — proving the CLI surface,
        // not just the library, is fail-closed. (The success path needs a compiled-in key and
        // is covered by the live end-to-end check in the PR.)
        let dir = tempfile::tempdir().unwrap();
        let msg = b"nxf release artifact";
        let (_pk, sig) = sign_with_fresh_key(msg);
        let art = dir.path().join("art.bin");
        let sig_path = dir.path().join("art.bin.minisig");
        std::fs::write(&art, msg).unwrap();
        std::fs::write(&sig_path, &sig).unwrap();

        let e =
            verify_command(true, art.to_str().unwrap(), sig_path.to_str().unwrap()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Verification);
    }
}
