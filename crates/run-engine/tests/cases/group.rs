// The form of a case group, for the two binaries that compile the groups: `cases!` makes each case a
// plain `pub fn`, a `case_test!` of the same name under `tests`, and an entry of `ALL`. The determinism
// binary compiles the same files with `case_test!` defined to nothing.

/// Declares one group of cases.
macro_rules! cases {
    ($( fn $name:ident($s:ident) $body:block )+) => {
        $( pub fn $name($s: &mut crate::script::Script) $body )+

        pub const ALL: &[(&str, fn(&mut crate::script::Script))] =
            &[$( (stringify!($name), $name) ),+];

        mod tests {
            $( case_test!($name); )+
        }
    };
}
