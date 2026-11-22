//! Where nexus-flow's data directory is on each platform, and `NXS_DATA_DIR` over all of them
//! (RFC 0001, "The local data on disk"). The environment is passed in, so every platform's rule is
//! tested on every host.

use std::ffi::OsString;
use std::path::PathBuf;

use nexus_flow_store::data_dir::{DataDir, DataDirError, Platform};

/// An absolute path on the host the test runs on, built from `/`-separated parts.
fn abs(parts: &str) -> PathBuf {
    let root = if cfg!(windows) { r"C:\" } else { "/" };
    parts.split('/').fold(PathBuf::from(root), |p, s| p.join(s))
}

/// The environment of one test: only the variables it names are set.
fn env<'a>(vars: &'a [(&'a str, PathBuf)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        vars.iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone().into_os_string())
    }
}

fn root(platform: Platform, vars: &[(&str, PathBuf)]) -> PathBuf {
    DataDir::resolve(platform, env(vars))
        .expect("the data directory resolves")
        .root()
        .to_path_buf()
}

#[test]
fn macos_uses_application_support_under_home() {
    assert_eq!(
        root(Platform::MacOs, &[("HOME", abs("Users/ada"))]),
        abs("Users/ada/Library/Application Support/nexus-flow")
    );
}

#[test]
fn linux_uses_xdg_data_home_when_it_is_set() {
    assert_eq!(
        root(
            Platform::Linux,
            &[
                ("HOME", abs("home/ada")),
                ("XDG_DATA_HOME", abs("data/ada"))
            ]
        ),
        abs("data/ada/nexus-flow")
    );
}

#[test]
fn linux_falls_back_to_local_share_when_xdg_data_home_is_unset() {
    assert_eq!(
        root(Platform::Linux, &[("HOME", abs("home/ada"))]),
        abs("home/ada/.local/share/nexus-flow")
    );
}

#[test]
fn linux_treats_an_empty_xdg_data_home_as_unset() {
    assert_eq!(
        root(
            Platform::Linux,
            &[("HOME", abs("home/ada")), ("XDG_DATA_HOME", PathBuf::new())]
        ),
        abs("home/ada/.local/share/nexus-flow")
    );
}

#[test]
fn linux_ignores_a_relative_xdg_data_home_as_the_xdg_spec_says() {
    assert_eq!(
        root(
            Platform::Linux,
            &[
                ("HOME", abs("home/ada")),
                ("XDG_DATA_HOME", PathBuf::from("data"))
            ]
        ),
        abs("home/ada/.local/share/nexus-flow")
    );
}

#[test]
fn windows_uses_local_app_data() {
    assert_eq!(
        root(
            Platform::Windows,
            &[("LOCALAPPDATA", abs("Users/ada/AppData/Local"))]
        ),
        abs("Users/ada/AppData/Local/nexus-flow")
    );
}

#[test]
fn nxs_data_dir_wins_over_every_platform_default() {
    let vars = [
        ("NXS_DATA_DIR", abs("srv/nxs")),
        ("HOME", abs("home/ada")),
        ("XDG_DATA_HOME", abs("data/ada")),
        ("LOCALAPPDATA", abs("Users/ada/AppData/Local")),
    ];
    for platform in [Platform::MacOs, Platform::Linux, Platform::Windows] {
        assert_eq!(root(platform, &vars), abs("srv/nxs"), "{platform:?}");
    }
}

#[test]
fn an_empty_nxs_data_dir_is_unset() {
    assert_eq!(
        root(
            Platform::Linux,
            &[("NXS_DATA_DIR", PathBuf::new()), ("HOME", abs("home/ada"))]
        ),
        abs("home/ada/.local/share/nexus-flow")
    );
}

#[test]
fn a_relative_nxs_data_dir_is_refused_so_it_cannot_land_in_the_working_tree() {
    let vars = [
        ("NXS_DATA_DIR", PathBuf::from("data")),
        ("HOME", abs("home/ada")),
    ];
    let err = DataDir::resolve(Platform::Linux, env(&vars)).unwrap_err();
    assert!(
        matches!(
            err,
            DataDirError::Relative {
                variable: "NXS_DATA_DIR",
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_platform_without_its_home_variable_is_refused() {
    for (platform, variable) in [
        (Platform::MacOs, "HOME"),
        (Platform::Linux, "HOME"),
        (Platform::Windows, "LOCALAPPDATA"),
    ] {
        let err = DataDir::resolve(platform, env(&[])).unwrap_err();
        assert!(
            matches!(err, DataDirError::Unset { variable: v } if v == variable),
            "{platform:?}: {err:?}"
        );
    }
}

#[test]
fn a_relative_home_is_refused() {
    let err =
        DataDir::resolve(Platform::Linux, env(&[("HOME", PathBuf::from("ada"))])).unwrap_err();
    assert!(
        matches!(
            err,
            DataDirError::Relative {
                variable: "HOME",
                ..
            }
        ),
        "{err:?}"
    );
}
