//! Kleene three-valued truth, shared by the two gates that have to decide an expression they
//! cannot always resolve.
//!
//! `cargo xtask runners check` evaluates a workflow job's `if:` to answer "can a pull request
//! reach this job?"; `cargo xtask macos-tests check` evaluates a `cfg` predicate to answer "does
//! an ubuntu job compile this test?". Both meet expressions they cannot decide — an `if:` naming
//! a context this repo does not model, a `feature = "x"` whose state is unknown here — and both
//! need the SAME answer to that: `Unknown`, which never lets the gate off.
//!
//! It lived twice, once per module, until the independent review of PR #423 pointed out that
//! `macos_tests` already reuses `runners::is_github_hosted_label` and so had no reason to keep a
//! second copy of this. Two copies of a truth table is two places to get De Morgan wrong in, and
//! only one of them would have a failing test.
//!
//! Strong Kleene, and the two absorbing cases are the whole point: `False AND Unknown` is `False`
//! (an expression that cannot be true whatever the unknown turns out to be), `True OR Unknown` is
//! `True`. Everything else touching `Unknown` stays `Unknown`.

/// Kleene truth. `Unknown` is what an expression the caller cannot decide evaluates to, and it
/// never lets a gate off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Truth {
    True,
    False,
    Unknown,
}

impl Truth {
    pub fn not(self) -> Truth {
        match self {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            Truth::Unknown => Truth::Unknown,
        }
    }

    pub fn and(self, other: Truth) -> Truth {
        match (self, other) {
            (Truth::False, _) | (_, Truth::False) => Truth::False,
            (Truth::True, Truth::True) => Truth::True,
            _ => Truth::Unknown,
        }
    }

    pub fn or(self, other: Truth) -> Truth {
        match (self, other) {
            (Truth::True, _) | (_, Truth::True) => Truth::True,
            (Truth::False, Truth::False) => Truth::False,
            _ => Truth::Unknown,
        }
    }

    /// `true` for a plain `bool`, so a caller that HAS decided a predicate does not spell out a
    /// two-armed match every time.
    pub fn of(known: bool) -> Truth {
        if known {
            Truth::True
        } else {
            Truth::False
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Truth;
    use super::Truth::{False, True, Unknown};

    /// The two absorbing cases — the ones a two-valued reading gets wrong, and the ones both
    /// gates depend on: an expression that cannot hold whatever the unknown part turns out to be
    /// is decided, not unknown.
    #[test]
    fn an_unknown_is_absorbed_when_the_other_side_already_decides_it() {
        assert_eq!(False.and(Unknown), False);
        assert_eq!(Unknown.and(False), False);
        assert_eq!(True.or(Unknown), True);
        assert_eq!(Unknown.or(True), True);
    }

    /// …and stays unknown everywhere else, which is what keeps both gates fail-closed.
    #[test]
    fn an_unknown_survives_everywhere_it_still_matters() {
        assert_eq!(True.and(Unknown), Unknown);
        assert_eq!(False.or(Unknown), Unknown);
        assert_eq!(Unknown.not(), Unknown);
        assert_eq!(Unknown.and(Unknown), Unknown);
        assert_eq!(Unknown.or(Unknown), Unknown);
    }

    #[test]
    fn the_two_valued_cases_are_ordinary_boolean_logic() {
        assert_eq!(True.not(), False);
        assert_eq!(False.not(), True);
        assert_eq!(True.and(True), True);
        assert_eq!(False.or(False), False);
        assert_eq!(Truth::of(true), True);
        assert_eq!(Truth::of(false), False);
    }
}
