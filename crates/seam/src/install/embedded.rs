//! The factory's defaults `nxs` embeds at build time from `templates/factory/` (RFC 0006, "Where
//! nexus-flow keeps them"), so that installing needs no network. A test holds this table equal
//! to the folder, so a file added there and not here fails the build's tests.

macro_rules! template {
    ($path:literal) => {
        (
            $path,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../templates/factory/",
                $path
            ))
            .as_slice(),
        )
    };
}

/// The text of `templates/factory/shipped.toml`.
pub(crate) const SHIPPED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../templates/factory/shipped.toml"
));

/// Each current default, by its path relative to `factory/`.
pub(crate) const TEXTS: &[(&str, &[u8])] = &[
    template!("CONFIG.md"),
    template!("FACTORY.md"),
    template!("roles/coder.md"),
    template!("roles/pm.md"),
    template!("status-note.md"),
];
