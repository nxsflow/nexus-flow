//! TLS for the Postgres backend — the difference between "sync to a Postgres on your own
//! LAN" and "sync to a managed Postgres, e.g. Supabase".
//!
//! The driver used to connect with `NoTls`, which every managed provider refuses outright:
//! Supabase, Neon and RDS all require TLS, so the backend was silently limited to a database
//! reachable in plaintext. This module builds the connector that lifts that limit
//! (nexus-flow-6j6v.94gm).
//!
//! Three choices worth knowing, because each has a trap behind it:
//!
//! * **`ring`, explicitly, not by default.** `rustls`'s default crypto provider is
//!   `aws-lc-rs`, whose `aws-lc-sys` needs a C toolchain and `cmake` on the build host — the
//!   exact cost that keeps the DynamoDB backend out of the released binary
//!   (nexus-flow-6j6v.7nee). Cargo.toml selects `ring`, and [`client_config`] installs it
//!   **per config** via `builder_with_provider` rather than relying on the process-default.
//!   That is not belt-and-braces: in a `--features postgres,dynamodb` build the AWS SDK
//!   unifies `aws-lc-rs` onto the same `rustls`, leaving *two* providers compiled in, and
//!   `ClientConfig::builder()` then panics at runtime with "no process-level CryptoProvider".
//!   Naming the provider makes that build work instead of blowing up on first connect.
//! * **Bundled roots, not the OS trust store.** The relay ships as a static musl binary aimed
//!   at distroless/scratch images, where `/etc/ssl/certs` may not exist. `webpki-roots` is
//!   self-contained, so verification works with no host setup.
//! * **A private CA is still reachable**, via `NXF_RELAY_PG_CA_FILE` — a PEM bundle appended
//!   to those roots. Self-hosted Postgres and providers that hand out their own CA are not
//!   locked out by the bundled floor.
//!
//! What this module deliberately does NOT do is decide *whether* to use TLS. That is
//! libpq's `sslmode` in the connection URL, parsed by the driver: the default `prefer`
//! negotiates TLS when the server offers it and falls back to plaintext when it does not
//! (so the CI Postgres service container, which has SSL off, stays green), while
//! `sslmode=require` makes TLS mandatory. Handing the driver a TLS-capable connector
//! therefore adds a capability without changing any existing deployment's behaviour.

use std::path::Path;
use std::sync::Arc;

use rustls::{ClientConfig, RootCertStore};

use crate::store::{StoreError, StoreResult};

/// Env var naming a PEM file whose certificates are trusted **in addition to** the bundled
/// public roots.
pub(crate) const CA_FILE_ENV: &str = "NXF_RELAY_PG_CA_FILE";

/// Root store: the bundled Mozilla roots, plus every certificate in `ca_file` when one is
/// given.
///
/// Split from the env lookup on purpose — env vars are process-global, and a test that sets
/// one races every other test in the binary (see the `det-ids-env-test-race` note). The
/// caller reads the variable; this function takes the path, so the behaviour is unit-testable
/// without touching global state.
fn root_store(ca_file: Option<&Path>) -> StoreResult<RootCertStore> {
    let mut roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let Some(path) = ca_file else {
        return Ok(roots);
    };

    // Every failure below names both the variable and the path. An operator who points this
    // at the wrong file is one typo away from a connection that fails much later with a
    // certificate error, and the message has to say which knob caused it.
    let pem = std::fs::read(path).map_err(|e| {
        StoreError(format!(
            "{CA_FILE_ENV}: cannot read {}: {e}",
            path.display()
        ))
    })?;
    let mut cursor = std::io::Cursor::new(pem);
    let certs = rustls_pemfile::certs(&mut cursor)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            StoreError(format!(
                "{CA_FILE_ENV}: cannot parse {} as PEM: {e}",
                path.display()
            ))
        })?;
    if certs.is_empty() {
        // A readable file with no CERTIFICATE block is the quiet version of the same
        // mistake — pointing at a private key, or at the server cert's key half. Left
        // unchecked it would produce a root store that verifies nothing extra and an
        // operator convinced they had installed their CA.
        return Err(StoreError(format!(
            "{CA_FILE_ENV}: {} contains no CERTIFICATE block",
            path.display()
        )));
    }
    for cert in certs {
        roots.add(cert).map_err(|e| {
            StoreError(format!(
                "{CA_FILE_ENV}: {} holds a certificate rustls rejected: {e}",
                path.display()
            ))
        })?;
    }
    Ok(roots)
}

/// The client config the Postgres connector runs on: `ring`, named explicitly (see the
/// module header), verifying against [`root_store`].
fn client_config(ca_file: Option<&Path>) -> StoreResult<ClientConfig> {
    let roots = root_store(ca_file)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| StoreError(format!("postgres TLS: {e}")))?;
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}

/// The connector handed to the connection pool. Reads [`CA_FILE_ENV`]; everything it decides
/// from it is tested through [`root_store`].
///
/// Also used by the Postgres parity suite for its admin connection, so the test harness and
/// the product dial identically — see the note on the module declaration in `lib.rs`.
pub fn make_tls() -> StoreResult<tokio_postgres_rustls::MakeRustlsConnect> {
    let ca_file = std::env::var(CA_FILE_ENV).ok();
    let config = client_config(ca_file.as_deref().map(Path::new))?;
    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway self-signed CA, used only to prove the file is read and trusted. It is
    /// not a secret and grants nothing: there is no private key here, and nothing in this
    /// repo ever connects to a server presenting it.
    const TEST_CA_PEM: &str = "-----BEGIN CERTIFICATE-----\n\
MIIDGzCCAgOgAwIBAgIUNF+71SBlIf8nY72NGsuSpKXdBTowDQYJKoZIhvcNAQEL\n\
BQAwHDEaMBgGA1UEAwwRbnhmLXJlbGF5IHRlc3QgQ0EwIBcNMjYwODA3MTcxOTIz\n\
WhgPMjEyNjA3MTQxNzE5MjNaMBwxGjAYBgNVBAMMEW54Zi1yZWxheSB0ZXN0IENB\n\
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEArpHUye9EjQwpr54l2UEf\n\
qpweds6gM+3XyKDDu94Jkq/CTWcJN9wiVrmWNVebRBtG1JLoha44NgfhPvkH+qiV\n\
jtucglOpWmRpo+dpu7z8Ds0c8e4fwvczpc4OdPuFR8+jL9yI2/p4Y4iGngnhtUdF\n\
2XxoFWCdFy9gbj5fGpQHw91bW24uXaqZj0z0ADDIvq0MGxr6yt6vNf2APALsRlt8\n\
a1GhARB+HGnVCG44F9gC+nrLpZNN0xYkyJdTjAFR2Tb9ZqM5+Oi7aqohpu7ZYQxh\n\
qI2ER1f3kvdyRbCXRzKbG1OIff+mIG/rSR0eJWC7gR5EU8chzovle/8EzWHhR6fQ\n\
jQIDAQABo1MwUTAdBgNVHQ4EFgQUKRazHAwPq6LfEJcTE55Tzj49PnkwHwYDVR0j\n\
BBgwFoAUKRazHAwPq6LfEJcTE55Tzj49PnkwDwYDVR0TAQH/BAUwAwEB/zANBgkq\n\
hkiG9w0BAQsFAAOCAQEAcEBGVhB7k0LcioDBAC/+ziC/OqJ7RVlh44FO3qsR4ru5\n\
pIbq81x1q0SERoMAvdWRIsBz+KcQp9s7nWYJCV68nxwTlFJZQAYPWO8ptOnOmdIr\n\
SgHpP82Z0BtWbzAvvQGvjNhydcAnkFhbFcHwBu6a/uGbickh/eU9WQT10x99Kpg4\n\
R/nsPTa3vGvon431S2XeCotKihan93yIyORjS12WsbSYlTE3YLrrpg4SMDPSrsCR\n\
NSZNYmsc998/8QY/G96fBuLyNa+4aS4qB6cV2+1bkKuqJwK9GZZBJDCa/5t60b1a\n\
qqmhPSVjLnYNOdyeyKzF3JdrQoITD2ubDi5Yk3qVtQ==\n\
-----END CERTIFICATE-----\n";

    fn write(dir: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, body).expect("write fixture");
        path
    }

    #[test]
    fn without_a_ca_file_the_bundled_public_roots_are_trusted() {
        let roots = root_store(None).expect("bundled roots");
        assert_eq!(
            roots.len(),
            webpki_roots::TLS_SERVER_ROOTS.len(),
            "the bundled Mozilla roots are the floor — a managed provider with a public CA \
             must verify with no host trust store at all"
        );
        assert!(roots.len() > 1, "an empty bundle would verify nothing");
    }

    #[test]
    fn a_custom_ca_file_is_trusted_on_top_of_the_bundled_roots() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write(&dir, "ca.pem", TEST_CA_PEM);
        let roots = root_store(Some(&path)).expect("custom CA accepted");
        assert_eq!(
            roots.len(),
            webpki_roots::TLS_SERVER_ROOTS.len() + 1,
            "a private CA must be ADDED to the public roots, not replace them — a relay \
             pointed at a self-hosted Postgres still talks to public services"
        );
    }

    #[test]
    fn a_missing_ca_file_fails_loudly_and_names_the_knob() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("absent.pem");
        let err = root_store(Some(&path)).expect_err("a missing CA file must not be ignored");
        assert!(
            err.0.contains(CA_FILE_ENV),
            "message must name the env var: {err:?}"
        );
        assert!(
            err.0.contains("absent.pem"),
            "message must name the path: {err:?}"
        );
    }

    #[test]
    fn a_ca_file_without_a_certificate_fails_loudly() {
        let dir = tempfile::tempdir().expect("tempdir");
        // The realistic mistake: pointing the variable at a key instead of a certificate.
        let path = write(
            &dir,
            "key.pem",
            "-----BEGIN PRIVATE KEY-----\nMIIB\n-----END PRIVATE KEY-----\n",
        );
        let err = root_store(Some(&path)).expect_err("a certificate-less file must not pass");
        assert!(
            err.0.contains("no CERTIFICATE block"),
            "message must say what was missing: {err:?}"
        );
    }

    #[test]
    fn the_client_config_builds_on_ring_without_a_process_default_provider() {
        // The regression this guards: `ClientConfig::builder()` panics when zero or two
        // providers are compiled in, and a `--features postgres,dynamodb` build has two.
        // Naming the provider must keep config construction infallible either way.
        client_config(None).expect("client config builds with an explicitly named provider");
    }
}
