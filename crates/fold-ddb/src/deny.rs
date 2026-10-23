//! The shared deny list of deleted accounts (6j6v.dbmd round 4b): board-fold's deletion fence, and
//! the fence `nxf-fold-purge` checks before it deletes or confirms anything.
//!
//! One DynamoDB table in the hosted block, keyed by the account's `<h>` — the first 32 lowercase
//! hex characters of SHA-256(`<issuer>:<sub>`). kwv2 writes one item per deleted account in step 0
//! of the account sequence; the ticket authorizer reads it for every request; board-fold reads it
//! before EVERY chunk it folds, without a cache, through [`admission`]. The table's shape, its item
//! and the rules around it are the crate doc's "Deny table" section.
//!
//! **What a writer of the board-fold table does with the answer** (consumer contract v2.1, owner
//! decision 2026-10-23 on the open choice of design review round 4b): it writes a chunk of a stream
//! only on [`Admission::Admitted`]. [`Admission::Denied`], [`Admission::Unscoped`] and an `Err`
//! write nothing — the chunk errs, is redriven and lands in the DLQ. So a stack that runs board-fold
//! with the deny table folds scoped ids only (`u-<h>`, `n-<h>-…`), and an id the list cannot name
//! can never hold data the account deletion would not find. [`Admission::writes`] is that rule.
//!
//! Which `<h>` an id belongs to is [`account_of`], pinned by the vector file
//! `contract/deny-account-vectors.json` that foundations run against the authorizer too.

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Mutex;

/// The variable that names the deny table, for board-fold and for `nxf-fold-purge`.
pub const DENY_TABLE_ENV: &str = "NXS_DENY_TABLE";

/// The deny table's only key attribute, a string: the account's `<h>`.
pub const DENY_KEY: &str = "h";

/// An account's `<h>`: exactly 32 lowercase hex characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountHash(String);

impl AccountHash {
    /// `s` when it is exactly 32 lowercase hex characters, else `None` — no normalisation.
    pub fn parse(s: &str) -> Option<AccountHash> {
        let hex = s.len() == 32
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        hex.then(|| AccountHash(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The account `stream` belongs to: `u-<h>` exactly, or any id beginning `n-<h>-` (the trailing
/// hyphen included, as the authorizer reads it). Anything else is unscoped: `None`.
pub fn account_of(stream: &str) -> Option<AccountHash> {
    if let Some(h) = stream.strip_prefix("u-") {
        return AccountHash::parse(h);
    }
    let rest = stream.strip_prefix("n-")?;
    // `n-<h>-…`: 32 characters of `<h>`, then the hyphen; whatever follows is the account's.
    let (h, tail) = (rest.get(..32)?, rest.get(32..)?);
    if !tail.starts_with('-') {
        return None;
    }
    AccountHash::parse(h)
}

/// Where a deny list is kept, for a report: the table's name and its `TableArn` (consumer contract
/// v2.1: a runner compares the ARN with the one the authorizer reports, never the name alone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyTable {
    pub name: String,
    pub table_arn: Option<String>,
}

/// A reader of the deny list.
pub trait DenyList: Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// The table this list reads, when it is one; `None` for a list in memory.
    fn table(&self) -> Option<&DenyTable> {
        None
    }

    /// Whether `account` is on the list, read fresh: no implementation caches, neither a hit nor a
    /// miss.
    fn is_denied(
        &self,
        account: &AccountHash,
    ) -> impl Future<Output = Result<bool, Self::Error>> + Send;
}

/// What the deny list says about writing a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    /// The stream's account is not on the list.
    Admitted,
    /// The stream's account is on the list: nothing may be written for it.
    Denied,
    /// The id carries no `<h>`, so no deny item can name it. A board-fold with the deny table
    /// writes nothing for it either (owner decision 2026-10-23).
    Unscoped,
}

impl Admission {
    /// Whether a chunk of the stream may be written: only when it is [`Admission::Admitted`].
    pub fn writes(self) -> bool {
        self == Admission::Admitted
    }

    /// The report's `fence` value: `not_denied`, `denied` or `unscoped`.
    pub fn fence(self) -> &'static str {
        match self {
            Admission::Admitted => "not_denied",
            Admission::Denied => "denied",
            Admission::Unscoped => "unscoped",
        }
    }
}

/// Call before EVERY chunk that may write `stream`, and never cache the answer. Anything but
/// [`Admission::Admitted`] — and an `Err` — writes nothing for it.
pub async fn admission<D: DenyList>(deny: &D, stream: &str) -> Result<Admission, D::Error> {
    let Some(account) = account_of(stream) else {
        return Ok(Admission::Unscoped);
    };
    Ok(if deny.is_denied(&account).await? {
        Admission::Denied
    } else {
        Admission::Admitted
    })
}

/// The deny list in memory, for tests and an embedder's own tests.
#[derive(Debug, Default)]
pub struct MemDenyList {
    denied: Mutex<BTreeSet<String>>,
}

impl MemDenyList {
    pub fn new() -> MemDenyList {
        MemDenyList::default()
    }

    /// Put `account` on the list.
    pub fn deny(&self, account: &AccountHash) {
        self.denied
            .lock()
            .unwrap()
            .insert(account.as_str().to_string());
    }
}

impl DenyList for MemDenyList {
    type Error = std::convert::Infallible;

    async fn is_denied(&self, account: &AccountHash) -> Result<bool, Self::Error> {
        Ok(self.denied.lock().unwrap().contains(account.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mem::ready;

    const H: &str = "00ff00ff00ff00ff00ff00ff00ff00ff";

    fn h() -> AccountHash {
        AccountHash::parse(H).expect("32 lowercase hex")
    }

    #[test]
    fn account_of_maps_the_scoped_forms_to_their_account() {
        for id in [
            format!("u-{H}"),
            format!("n-{H}-1"),
            format!("n-{H}-10"),
            format!("n-{H}-1-x"),
        ] {
            assert_eq!(account_of(&id), Some(h()), "{id}");
        }
    }

    #[test]
    fn account_of_refuses_every_other_form() {
        let upper = H.to_uppercase();
        for id in [
            format!("u-{H}-1"),
            format!("u-{H}x"),
            format!("n-{H}"),
            format!("n-{}-1", &H[..31]),
            format!("n-{H}0-1"),
            format!("n-{upper}-1"),
            format!("x-{H}-1"),
            "6j6v".to_string(),
            String::new(),
        ] {
            assert_eq!(account_of(&id), None, "{id}");
        }
    }

    #[test]
    fn account_of_reproduces_the_shared_vector_file() {
        let doc: serde_json::Value =
            serde_json::from_str(include_str!("../contract/deny-account-vectors.json"))
                .expect("the vector file is JSON");
        assert_eq!(doc["contract"], "deny-account-of");
        assert_eq!(doc["version"], 1);
        let cases = doc["cases"].as_array().expect("cases");
        assert!(cases.len() >= 20, "the file lost cases: {}", cases.len());
        for case in cases {
            let id = case["id"].as_str().expect("id");
            let expected = case["account"].as_str();
            assert_eq!(
                account_of(id).as_ref().map(AccountHash::as_str),
                expected,
                "{id:?}: {}",
                case["why"]
            );
        }
    }

    #[test]
    fn an_account_hash_is_exactly_thirty_two_lowercase_hex() {
        assert_eq!(h().as_str(), H);
        for bad in [
            &H[..31],
            &format!("{H}0")[..],
            &H.to_uppercase()[..],
            "",
            "g".repeat(32).as_str(),
        ] {
            assert_eq!(AccountHash::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn admission_answers_denied_admitted_and_unscoped() {
        let list = MemDenyList::new();
        list.deny(&h());
        let other = "11ff00ff00ff00ff00ff00ff00ff00ff";
        assert_eq!(
            ready(admission(&list, &format!("n-{H}-1"))).unwrap(),
            Admission::Denied
        );
        assert_eq!(
            ready(admission(&list, &format!("u-{H}"))).unwrap(),
            Admission::Denied
        );
        assert_eq!(
            ready(admission(&list, &format!("n-{other}-1"))).unwrap(),
            Admission::Admitted
        );
        assert_eq!(
            ready(admission(&list, "6j6v")).unwrap(),
            Admission::Unscoped
        );
    }

    #[test]
    fn only_admitted_writes() {
        assert!(Admission::Admitted.writes());
        assert!(!Admission::Denied.writes());
        assert!(!Admission::Unscoped.writes());
        assert_eq!(Admission::Admitted.fence(), "not_denied");
        assert_eq!(Admission::Denied.fence(), "denied");
        assert_eq!(Admission::Unscoped.fence(), "unscoped");
    }

    #[derive(Debug)]
    struct Down;
    impl std::fmt::Display for Down {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("the deny table is unreachable")
        }
    }
    impl std::error::Error for Down {}

    struct Failing;
    impl DenyList for Failing {
        type Error = Down;
        async fn is_denied(&self, _: &AccountHash) -> Result<bool, Down> {
            Err(Down)
        }
    }

    #[test]
    fn a_read_that_errs_is_an_error_never_admitted() {
        assert!(ready(admission(&Failing, &format!("n-{H}-1"))).is_err());
    }

    /// An unscoped id is never looked up: the list cannot name it, and a lookup that errs must not
    /// turn it into anything but `Unscoped`.
    #[test]
    fn an_unscoped_id_is_not_looked_up() {
        assert_eq!(
            ready(admission(&Failing, "6j6v")).unwrap(),
            Admission::Unscoped
        );
    }
}
