//! The reducer cases of §12.1, without processes. Each asserts the decision list of every input it
//! cares about, never only the end state (memory a-test-name-is-a-claim).

#[path = "../script.rs"]
pub mod script;

/// Under this binary every case is a `#[test]` of its own name.
macro_rules! case_test {
    ($name:ident) => {
        #[test]
        fn $name() {
            super::$name(&mut crate::script::Script::coding());
        }
    };
}

include!("group.rs");

mod happy;
