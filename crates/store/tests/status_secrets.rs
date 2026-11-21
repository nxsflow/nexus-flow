//! The secret patterns of a status note (RFC 0007, "Secrets and personal data"): a text that holds
//! a known secret is refused with `secret_in_note`, naming the field and the pattern, never the
//! match. Each pattern has a positive case and a near miss that must pass.

use nexus_flow_store::status::secrets::{check, SecretInNote, SecretPattern};

/// The pattern `check` refuses `text` with, or `None` when it passes.
fn found(text: &str) -> Option<SecretPattern> {
    check("body", text).err().map(|e| e.pattern)
}

const RSA_KEY: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
MIIEowIBAAKCAQEA3Tz2mr7SZiAMfQyuvBjM9OiZ1BjP5CE/Wm/Rr500P\n\
-----END RSA PRIVATE KEY-----";

#[test]
fn a_text_without_a_secret_passes() {
    assert_eq!(check("headline", "Shipped ka.42 and merged #160"), Ok(()));
    assert_eq!(check("body", ""), Ok(()));
}

#[test]
fn a_private_key_block_is_refused() {
    assert_eq!(found(RSA_KEY), Some(SecretPattern::PrivateKey));
    for label in ["", "OPENSSH ", "EC ", "ENCRYPTED ", "DSA "] {
        let text = format!("see below\n-----BEGIN {label}PRIVATE KEY-----\nAAAA\n");
        assert_eq!(found(&text), Some(SecretPattern::PrivateKey), "{text}");
    }
    assert_eq!(
        found("-----BEGIN PGP PRIVATE KEY BLOCK-----"),
        Some(SecretPattern::PrivateKey)
    );
}

#[test]
fn a_public_key_or_a_certificate_is_not_a_private_key() {
    assert_eq!(found("-----BEGIN PUBLIC KEY-----\nMFkw\n-----END PUBLIC KEY-----"), None);
    assert_eq!(found("-----BEGIN CERTIFICATE-----\nMIIB\n"), None);
    assert_eq!(found("rotated the BEGIN PRIVATE KEY block in the vault"), None);
}

#[test]
fn the_refusal_names_the_field_and_the_pattern() {
    let err = check("items[2].text", RSA_KEY).unwrap_err();
    assert_eq!(
        err,
        SecretInNote {
            field: "items[2].text".to_owned(),
            pattern: SecretPattern::PrivateKey,
        }
    );
    assert_eq!(err.code(), "secret_in_note");
    assert_eq!(err.pattern.name(), "private_key");
}

#[test]
fn the_refusal_holds_no_part_of_the_secret() {
    let err = check("body", RSA_KEY).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("body"), "{message}");
    assert!(message.contains("private_key"), "{message}");
    for shown in [message, format!("{err:?}")] {
        assert!(!shown.contains("BEGIN"), "{shown}");
        assert!(!shown.contains("MIIE"), "{shown}");
    }
}

#[test]
fn a_github_token_is_refused() {
    for prefix in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        let text = format!("token={prefix}16C7e42F292c6912E7710c838347Ae178B4a");
        assert_eq!(found(&text), Some(SecretPattern::GitHubToken), "{text}");
    }
    let fine_grained =
        "github_pat_11ABCDEFG0123456789abc_0123456789abcdefghijABCDEFGHIJ0123456789abcdefghijKLMNO";
    assert_eq!(found(fine_grained), Some(SecretPattern::GitHubToken));
}

#[test]
fn a_github_token_prefix_alone_is_not_a_token() {
    assert_eq!(found("classic tokens start with ghp_ and fine-grained ones with github_pat_"), None);
    assert_eq!(found("ghp_short1234"), None);
    assert_eq!(found("ghx_16C7e42F292c6912E7710c838347Ae178B4a"), None);
}

#[test]
fn an_aws_access_key_id_is_refused() {
    assert_eq!(found("AKIAIOSFODNN7EXAMPLE"), Some(SecretPattern::AwsAccessKeyId));
    assert_eq!(
        found("export AWS_ACCESS_KEY_ID=ASIAY34FZKBOKMUTVV7A;"),
        Some(SecretPattern::AwsAccessKeyId)
    );
}

#[test]
fn a_near_aws_access_key_id_is_not_one() {
    // One character short, lower case, another prefix, or part of a longer word.
    assert_eq!(found("AKIAIOSFODNN7EXAMPL"), None);
    assert_eq!(found("akiaiosfodnn7example"), None);
    assert_eq!(found("ABIAIOSFODNN7EXAMPLE"), None);
    assert_eq!(found("XAKIAIOSFODNN7EXAMPLE"), None);
    assert_eq!(found("AKIAIOSFODNN7EXAMPLEX"), None);
}

#[test]
fn a_slack_token_is_refused() {
    for kind in ['a', 'b', 'p', 'r', 's'] {
        let text = format!("SLACK_TOKEN=xox{kind}-2948312470-3051276984-tjYwvVq3RXbcXUyJ4hZ8");
        assert_eq!(found(&text), Some(SecretPattern::SlackToken), "{text}");
    }
}

#[test]
fn a_near_slack_token_is_not_one() {
    // Another kind letter, no dash, or the prefix alone.
    assert_eq!(found("xoxc-2948312470-3051276984-tjYwvVq3RXbcXUyJ4hZ8"), None);
    assert_eq!(found("xoxb2948312470-3051276984-tjYwvVq3RXbcXUyJ4hZ8"), None);
    assert_eq!(found("bot tokens start with xoxb- in Slack"), None);
}

const JWT: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIn0.\
SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";

#[test]
fn a_json_web_token_is_refused() {
    assert_eq!(found(JWT), Some(SecretPattern::JsonWebToken));
    assert_eq!(
        found(&format!("Authorization: Bearer {JWT}")),
        Some(SecretPattern::JsonWebToken)
    );
}

#[test]
fn a_near_json_web_token_is_not_one() {
    // Two parts only, an empty part, or a first part that does not start `eyJ`.
    assert_eq!(found("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0"), None);
    assert_eq!(found("eyJhbGciOiJIUzI1NiJ9..SflKxwRJSMeKKF2QT4fw"), None);
    assert_eq!(found("abJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.SflKxwRJSMeKKF2QT4fw"), None);
    assert_eq!(found("keyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.SflKxwRJSMeKKF2QT4fw"), None);
}

#[test]
fn a_linear_api_key_is_refused() {
    assert_eq!(
        found("LINEAR_API_KEY=lin_api_Xk3J9vQm2PzT8rLw4NcY7bHd1FsGa6ZeUo0RiKqE"),
        Some(SecretPattern::LinearApiKey)
    );
}

#[test]
fn a_near_linear_api_key_is_not_one() {
    // The prefix alone, a short body, or another prefix.
    assert_eq!(found("Linear's personal keys start with lin_api_"), None);
    assert_eq!(found("lin_api_Xk3J9vQm"), None);
    assert_eq!(found("lin_oauth_Xk3J9vQm2PzT8rLw4NcY7bHd1FsGa6ZeUo0RiKqE"), None);
}
