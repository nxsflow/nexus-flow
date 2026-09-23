//! Keeping a relay URL's credentials out of every message (nxf 6j6v.q3kk).
//!
//! A relay behind a gateway takes its key as userinfo in the URL —
//! `https://<user>:<password>@relay.example` — because that is the one place the `ureq` transport
//! has for credentials. [`engine::HttpTransport`](crate::engine) therefore takes the userinfo OFF
//! the URL when it is built and sends it as an `Authorization` header, so no URL it hands `ureq`, and
//! no error `ureq` formats from one, can carry it. This module is the other half: the one rule for
//! turning a text that may quote such a URL into one that does not, for every place that holds the
//! configured URL itself — `nxs sync machines` naming the relay it asked, `nxs sync bind` and
//! `nxs sync endpoint` echoing it, a configuration file's parse error quoting it — and for an
//! embedding host that logs a relay URL of its own.
//!
//! Always compiled, and dependency-free, so a host that links only the wire types can use it too.

/// What a userinfo is replaced with. Three characters that cannot be mistaken for a real user name,
/// and short enough that the host beside them stays the thing a reader sees.
pub const MASK: &str = "***";

/// `text` with the userinfo of every `scheme://userinfo@host…` in it replaced by [`MASK`].
///
/// **The WHOLE userinfo goes, user included.** A relay key is often the pair — manufakt.io's
/// staging relay takes a generated user as well as a generated password — so keeping the user
/// "because it is diagnostic" would publish half the secret. Scheme, host, port and path stay: a
/// masked message that no longer says which relay failed gets switched off.
///
/// A userinfo is only recognised inside an AUTHORITY — between `://` and the first `/`, `?`, `#`,
/// whitespace or quote — so an `@` in a path, a query or the sentence around the URL is left alone.
/// Within the authority the LAST `@` ends the userinfo, as a URL parser reads it: a password with a
/// raw `@` in it is masked whole rather than split.
pub fn redact_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme_at) = rest.find("://") {
        let after_scheme = scheme_at + 3;
        let authority_end = rest[after_scheme..]
            .find(|c: char| {
                matches!(c, '/' | '?' | '#' | '"' | '\'' | '<' | '>') || c.is_whitespace()
            })
            .map_or(rest.len(), |i| after_scheme + i);
        match rest[after_scheme..authority_end].rfind('@') {
            Some(at) => {
                out.push_str(&rest[..after_scheme]);
                out.push_str(MASK);
                rest = &rest[after_scheme + at..];
            }
            None => {
                out.push_str(&rest[..authority_end]);
                rest = &rest[authority_end..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_userinfo_goes_and_host_port_and_path_stay() {
        for (text, want) in [
            (
                "https://relay-user:s3cret@api.staging.example.io/streams/s/register: Dns Failed",
                "https://***@api.staging.example.io/streams/s/register: Dns Failed",
            ),
            (
                "http://token@127.0.0.1:8787/streams/s/ops",
                "http://***@127.0.0.1:8787/streams/s/ops",
            ),
            // A raw `@` in the password: the LAST one ends the userinfo, as a URL parser reads it.
            (
                "http://u:p@ss@relay.example/x",
                "http://***@relay.example/x",
            ),
            // No path at all, then the end of the text.
            ("http://u:p@relay.example", "http://***@relay.example"),
            // Two URLs in one message, both masked.
            (
                "from http://a:b@one.example to https://c:d@two.example:9/y",
                "from http://***@one.example to https://***@two.example:9/y",
            ),
            // Quoted, as a Debug rendering would print it.
            (
                "\"https://u:pw@relay.example\"",
                "\"https://***@relay.example\"",
            ),
        ] {
            assert_eq!(redact_userinfo(text), want, "{text}");
        }
    }

    /// A redaction that mangles the ordinary case gets turned off.
    #[test]
    fn a_text_without_credentials_is_left_exactly_as_it_was() {
        for text in [
            "",
            "sync transport: http://127.0.0.1:8787/streams/s/ops: Connection Failed",
            "mail me at someone@example.com",
            "https://relay.example/streams/a@b/ops",
            "https://relay.example?who=a@b",
            "https://relay.example#frag@x",
            "not a url :// at all @ here",
        ] {
            assert_eq!(redact_userinfo(text), text, "{text}");
        }
    }
}
