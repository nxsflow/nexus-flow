//! `DataDir::from_env` resolves the data directory from the process's own environment, by the
//! rule of the platform the binary was built for (RFC 0001, "The local data on disk"). The test
//! sets a variable of the process, so it lives alone in its own test binary.

use std::path::PathBuf;

use nexus_flow_store::data_dir::{DataDir, DataDirError};

#[test]
fn the_process_environment_resolves_the_data_directory() {
    let dir = std::env::temp_dir().join(format!("nxs-from-env-{}", std::process::id()));
    std::env::set_var("NXS_DATA_DIR", &dir);
    assert_eq!(DataDir::from_env().unwrap().root(), dir.as_path());

    std::env::set_var("NXS_DATA_DIR", "relative/data");
    assert_eq!(
        DataDir::from_env().unwrap_err(),
        DataDirError::Relative {
            variable: "NXS_DATA_DIR",
            path: PathBuf::from("relative/data"),
        }
    );
    std::env::remove_var("NXS_DATA_DIR");
}
