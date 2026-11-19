use super::tools_first;
use std::path::Path;

/// A `PATH` of `entries`, in this platform's spelling.
fn path_of(entries: &[&str]) -> String {
    std::env::join_paths(entries)
        .unwrap()
        .into_string()
        .unwrap()
}

#[test]
fn the_tools_lead_the_path_once_and_the_rest_keeps_its_order() {
    assert_eq!(
        tools_first(Path::new("/tools"), Some(&path_of(&["/b", "/tools", "/c"]))),
        path_of(&["/tools", "/b", "/c"])
    );
    assert_eq!(tools_first(Path::new("/tools"), None), "/tools");
}

#[test]
fn a_directory_a_path_cannot_carry_leaves_the_path_as_it_was() {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let inherited = path_of(&["/b", "/c"]);
    assert_eq!(
        tools_first(Path::new(&format!("/a{separator}b")), Some(&inherited)),
        inherited
    );
}
