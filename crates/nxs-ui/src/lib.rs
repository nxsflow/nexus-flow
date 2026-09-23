//! nxs-ui — the shared, brand-conform CLI presentation layer every nxs init entrypoint uses
//! (`nxs init`, `nxf init`, `nxm init`), built ONCE here so styling + the chooser are not rebuilt
//! 2–3× across the modules (aye.25). The umbrella design requires that `nxs` own the visible
//! presentation; the modules are only driven (a silent/quiet mode), so this layer lives outside
//! `crates/cli` and outside the substrate (`nxs-foundation` stays free of terminal deps).
//!
//! Brand: **manufakt.io "The Forge"** — an Ember-Red accent (`#DC2626`) on a graphite ramp. THE
//! guardrail is the [`style`] TTY-gate: splendor renders only on a real terminal that is not
//! `--json`; agents, `--json`, pipes, CI, and the trycmd harness always get byte-stable plain
//! ASCII with zero escape codes.
//!
//! - [`style`]: the [`Theme`](style::Theme) (TTY/color-depth detection) + semantic helpers.
//! - [`width`]: terminal-width logic (min/max, content cap) + a collapsing box renderer.
//! - [`chooser`]: the reusable arrow-key [`Choice`](chooser::Choice) widget (inquire-backed).

pub mod chooser;
pub mod style;
pub mod width;

pub use chooser::{choose, choose_many, Choice, ChooseError};
pub use style::{ColorLevel, Theme};
