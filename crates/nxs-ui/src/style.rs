//! manufakt-Forge styling, TTY-gated (spec / aye.25). The brand is **manufakt.io "The Forge"**:
//! an **Ember-Red** accent (`#DC2626`, deep `#B91C1C`) on a graphite neutral ramp — emphasis is
//! weight/inversion PLUS the ember accent for active states / the recommended default.
//!
//! THE byte-stability guardrail (non-negotiable): splendor renders **only** when output is a real
//! terminal AND not `--json`. Agents, `--json`, pipes, CI, and the trycmd harness get byte-stable
//! plain ASCII with **zero** escape codes. A second axis honors `NO_COLOR`/`TERM=dumb`: on a TTY
//! with color suppressed we still use bold/inversion (weight, not color), never raw color codes.

use anstyle::{Ansi256Color, AnsiColor, Color, RgbColor, Style};

// ── manufakt-Forge palette (../brand/BRAND.md) ──────────────────────────────
/// Ember Red — the dominant, decisive accent (CTAs, active states, the recommended default).
pub const EMBER: RgbColor = RgbColor(0xDC, 0x26, 0x26);
/// Deep ember — pressed/active emphasis (kept for callers that want the darker tone).
pub const EMBER_DEEP: RgbColor = RgbColor(0xB9, 0x1C, 0x1C);
/// Secondary text on the graphite ramp — for muted/auxiliary lines.
pub const GRAPHITE_SECONDARY: RgbColor = RgbColor(0x6B, 0x6B, 0x6B);
/// Border tone on the graphite ramp — for box frames/dividers.
pub const BORDER: RgbColor = RgbColor(0xE5, 0xE5, 0xE0);
/// Flame — the warm end of the brand's declared Ember → Flame gradient (`#DC2626` → `#EA580C`).
/// The brand names it only as a gradient stop; the priority ramp needs it as a tone of its own.
pub const FLAME: RgbColor = RgbColor(0xEA, 0x58, 0x0C);

/// How much color the active terminal can render — the truecolor/256/16 fallback axis (point 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorLevel {
    /// No color (NO_COLOR / `TERM=dumb`): weight/inversion only.
    None,
    /// 16-color ANSI: ember → `Red`.
    Ansi16,
    /// 256-color: ember → a near Ansi256 red.
    Ansi256,
    /// 24-bit truecolor: the exact `#DC2626`.
    TrueColor,
}

/// The resolved presentation capability of the current sink. Built once via [`Theme::detect`] from
/// the TTY + env, or constructed directly in tests / by a caller that wants to force it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Whether ANY escape codes may be emitted at all — the byte-stability gate (`!json && tty`).
    /// When false, every helper returns its input verbatim (plain ASCII, no escapes).
    allow_escapes: bool,
    /// The color depth to emit when `allow_escapes` and color is not suppressed.
    color: ColorLevel,
}

impl Theme {
    /// Construct a theme directly (tests, or a caller forcing a capability).
    pub fn new(allow_escapes: bool, color: ColorLevel) -> Theme {
        Theme {
            allow_escapes,
            color,
        }
    }

    /// A guaranteed-plain theme: no escapes, ever. The shape agents/`--json`/pipes always get.
    pub fn plain() -> Theme {
        Theme::new(false, ColorLevel::None)
    }

    /// Detect the capability for a stdout sink. `json` forces plain (the machine contract is never
    /// framed). Otherwise: escapes only on a real terminal; color only if not suppressed by
    /// `NO_COLOR`/`TERM=dumb`; depth from the terminal's advertised support.
    pub fn detect(json: bool) -> Theme {
        use std::io::IsTerminal;
        Theme::detect_for(json, std::io::stdout().is_terminal())
    }

    /// [`detect`](Self::detect) with the is-terminal decision injected — the testable core.
    pub fn detect_for(json: bool, is_terminal: bool) -> Theme {
        let allow_escapes = !json && is_terminal;
        if !allow_escapes {
            return Theme::plain();
        }
        // Best-effort: turn on ANSI processing for a legacy Windows console (no-op elsewhere).
        let _ = anstyle_query::windows::enable_ansi_colors();
        let color = if anstyle_query::no_color() || term_is_dumb() {
            ColorLevel::None
        } else if anstyle_query::truecolor() {
            ColorLevel::TrueColor
        } else if anstyle_query::term_supports_ansi_color() {
            if term_is_256() {
                ColorLevel::Ansi256
            } else {
                ColorLevel::Ansi16
            }
        } else {
            ColorLevel::None
        };
        Theme::new(true, color)
    }

    /// Whether this theme emits any escapes at all (false ⇒ plain ASCII everywhere).
    pub fn is_styled(&self) -> bool {
        self.allow_escapes
    }

    /// The color depth this theme renders.
    pub fn color_level(&self) -> ColorLevel {
        self.color
    }

    /// The ember accent as the right `Color` for this theme's depth, or `None` when color is off.
    fn ember(&self) -> Option<Color> {
        match self.color {
            ColorLevel::None => None,
            // 256-color near-ember: a deep red that reads as ember without truecolor.
            ColorLevel::Ansi256 => Some(Color::Ansi256(Ansi256Color(160))),
            ColorLevel::Ansi16 => Some(Color::Ansi(AnsiColor::Red)),
            ColorLevel::TrueColor => Some(Color::Rgb(EMBER)),
        }
    }

    fn paint(&self, style: Style, s: &str) -> String {
        if !self.allow_escapes {
            return s.to_string();
        }
        format!("{}{s}{}", style.render(), style.render_reset())
    }

    /// A section heading — bold (weight is the constant; the brand never relies on color alone).
    pub fn heading(&self, s: &str) -> String {
        self.paint(Style::new().bold(), s)
    }

    /// Strong emphasis — bold.
    pub fn emphasis(&self, s: &str) -> String {
        self.paint(Style::new().bold(), s)
    }

    /// The ember accent for active states / the recommended default. With color: bold ember. With
    /// escapes but no color (`NO_COLOR` on a TTY): bold inversion — weight, never a raw color code.
    pub fn accent(&self, s: &str) -> String {
        let style = match self.ember() {
            Some(c) => Style::new().bold().fg_color(Some(c)),
            None => Style::new().bold().invert(),
        };
        self.paint(style, s)
    }

    /// A muted/auxiliary line — dimmed (and the graphite-secondary tone where color is available).
    pub fn muted(&self, s: &str) -> String {
        let mut style = Style::new().dimmed();
        if !matches!(self.color, ColorLevel::None) {
            let c = match self.color {
                ColorLevel::TrueColor => Color::Rgb(GRAPHITE_SECONDARY),
                _ => Color::Ansi(AnsiColor::BrightBlack),
            };
            style = style.fg_color(Some(c));
        }
        self.paint(style, s)
    }

    /// A priority cell, painted by the item's CANONICAL ORDINAL (0 = most urgent) — the shared
    /// brand ramp `nxf`, `nxm` and `nxc` all show the same rungs of (6j6v.8bax).
    ///
    /// The ordinal, never the label: priority NAMES are plugin vocabulary (`issue-tracker` says
    /// P0..P4, `personal-todo` says now/soon/later/…), and the canonical form every seam carries
    /// is the ordinal. A ramp that matched on the string "P1" would be wrong the moment another
    /// plugin is active. `s` is the already-localized label and is passed through untouched.
    ///
    /// ONE axis, hot to neutral, along the brand's Ember → Flame gradient and the graphite ramp
    /// below it — a rainbow would be off-brand and would not read as an ordering:
    ///
    /// | ordinal | tone | weight |
    /// | --- | --- | --- |
    /// | 0 | [`EMBER_DEEP`] | bold |
    /// | 1 | [`EMBER`] | — |
    /// | 2 | [`FLAME`] | — |
    /// | 3 | the plain foreground | — |
    /// | ≥ 4 | [`GRAPHITE_SECONDARY`] | dim |
    ///
    /// The two axes are independent, which is what keeps the ramp legible where color is
    /// suppressed (`NO_COLOR` / `TERM=dumb`): weight alone still separates the hot end from the
    /// cold one. A plugin with fewer rungs than five simply uses the hot end; one with more gets
    /// the cold end for everything past it — colder than "low" must never look more urgent again.
    pub fn priority(&self, ordinal: usize, s: &str) -> String {
        let mut style = match ordinal {
            0 => Style::new().bold(),
            n if n >= 4 => Style::new().dimmed(),
            _ => Style::new(),
        };
        if let Some(c) = self.priority_tone(ordinal) {
            style = style.fg_color(Some(c));
        }
        self.paint(style, s)
    }

    /// The ramp's tone for one ordinal at this theme's depth, or `None` where the rung is
    /// deliberately unpainted (the neutral rung 3) or color is off entirely.
    fn priority_tone(&self, ordinal: usize) -> Option<Color> {
        match self.color {
            ColorLevel::None => None,
            ColorLevel::TrueColor => match ordinal {
                0 => Some(Color::Rgb(EMBER_DEEP)),
                1 => Some(Color::Rgb(EMBER)),
                2 => Some(Color::Rgb(FLAME)),
                3 => None,
                _ => Some(Color::Rgb(GRAPHITE_SECONDARY)),
            },
            ColorLevel::Ansi256 => match ordinal {
                // 124 / 160 / 202: a dark red, the near-ember `accent` already uses, and an
                // orange — three rungs that stay apart on a 256-color terminal.
                0 => Some(Color::Ansi256(Ansi256Color(124))),
                1 => Some(Color::Ansi256(Ansi256Color(160))),
                2 => Some(Color::Ansi256(Ansi256Color(202))),
                3 => None,
                _ => Some(Color::Ansi(AnsiColor::BrightBlack)),
            },
            ColorLevel::Ansi16 => match ordinal {
                // 16 colors hold no orange, so flame becomes yellow: the only remaining warm tone
                // that is not red, which keeps the three hot rungs distinguishable. Rung 0 is red
                // like rung 1 and separates on its bold weight.
                0 | 1 => Some(Color::Ansi(AnsiColor::Red)),
                2 => Some(Color::Ansi(AnsiColor::Yellow)),
                3 => None,
                _ => Some(Color::Ansi(AnsiColor::BrightBlack)),
            },
        }
    }

    /// A box border / divider glyph string in the border tone (graphite ramp).
    pub fn box_border(&self, s: &str) -> String {
        let mut style = Style::new();
        if !matches!(self.color, ColorLevel::None) {
            let c = match self.color {
                ColorLevel::TrueColor => Color::Rgb(BORDER),
                _ => Color::Ansi(AnsiColor::BrightBlack),
            };
            style = style.fg_color(Some(c));
        }
        self.paint(style, s)
    }
}

fn term_is_dumb() -> bool {
    std::env::var("TERM").map(|t| t == "dumb").unwrap_or(false)
}

/// Heuristic 256-vs-16 split: a `*-256color` TERM advertises 256. Truecolor is decided earlier.
fn term_is_256() -> bool {
    std::env::var("TERM")
        .map(|t| t.contains("256color"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_theme_emits_zero_escapes() {
        // The load-bearing guard: a plain theme returns input verbatim — no ESC byte anywhere.
        let t = Theme::plain();
        for rendered in [
            t.heading("Forge"),
            t.emphasis("x"),
            t.accent("recommended"),
            t.muted("aux"),
            t.box_border("───"),
        ] {
            assert!(
                !rendered.contains('\u{1b}'),
                "plain theme must not emit an escape: {rendered:?}"
            );
        }
        assert_eq!(t.accent("recommended"), "recommended");
    }

    #[test]
    fn json_forces_plain_even_on_a_terminal() {
        // `--json` is the machine contract — never framed, even when stdout is a TTY.
        let t = Theme::detect_for(true, true);
        assert!(!t.is_styled());
        assert_eq!(t.accent("x"), "x");
    }

    #[test]
    fn non_terminal_forces_plain_even_without_json() {
        // A pipe/redirect (not a TTY) is byte-stable plain ASCII regardless of --json.
        let t = Theme::detect_for(false, false);
        assert!(!t.is_styled());
        assert_eq!(t.heading("x"), "x");
    }

    #[test]
    fn truecolor_accent_carries_the_exact_ember_rgb() {
        // On a truecolor terminal the accent SGR encodes #DC2626 = 220;38;38.
        let t = Theme::new(true, ColorLevel::TrueColor);
        let out = t.accent("go");
        assert!(
            out.contains('\u{1b}'),
            "styled theme emits escapes: {out:?}"
        );
        assert!(out.contains("220;38;38"), "ember rgb present: {out:?}");
    }

    #[test]
    fn no_color_on_a_tty_uses_weight_not_color() {
        // NO_COLOR on a TTY: escapes are allowed (bold/invert) but NO color SGR (no `38;2`/`38;5`).
        let t = Theme::new(true, ColorLevel::None);
        let out = t.accent("go");
        assert!(out.contains('\u{1b}'), "weight still styles: {out:?}");
        assert!(
            !out.contains("38;2") && !out.contains("38;5"),
            "no color code: {out:?}"
        );
    }

    // ---- the priority ramp (6j6v.8bax) -------------------------------------

    #[test]
    fn priority_ramp_runs_deep_ember_to_graphite_across_the_ordinals() {
        // ONE axis, hot to neutral, along the brand's declared Ember → Flame gradient: the reader
        // sees the weighting without reading the label.
        let t = Theme::new(true, ColorLevel::TrueColor);
        let p0 = t.priority(0, "P0");
        assert!(p0.contains("185;28;28"), "P0 is deep ember: {p0:?}");
        assert!(p0.contains("\u{1b}[1m"), "P0 is additionally bold: {p0:?}");
        assert!(t.priority(1, "P1").contains("220;38;38"), "P1 is ember");
        assert!(t.priority(2, "P2").contains("234;88;12"), "P2 is flame");
        assert_eq!(
            t.priority(3, "P3"),
            "P3",
            "P3 is the unstyled foreground — the ramp's neutral rung"
        );
        assert!(
            t.priority(4, "P4").contains("107;107;107"),
            "P4 is graphite secondary"
        );
    }

    #[test]
    fn priority_ramp_reads_the_ordinal_and_never_the_label() {
        // Priority LABELS are plugin vocabulary — issue-tracker says "P1", personal-todo says
        // "soon". The canonical form is the ordinal, so the same rung paints whatever text it is
        // handed; a ramp that matched on "P1" would be wrong under any other plugin.
        let t = Theme::new(true, ColorLevel::TrueColor);
        for (ord, rgb) in [(0, "185;28;28"), (1, "220;38;38"), (2, "234;88;12")] {
            let issue = t.priority(ord, "P9");
            let todo = t.priority(ord, "someday");
            assert!(
                issue.contains(rgb) && todo.contains(rgb),
                "ordinal {ord} paints both"
            );
            assert!(
                todo.contains("someday"),
                "the label passes through verbatim: {todo:?}"
            );
        }
    }

    #[test]
    fn priority_ramp_clamps_an_out_of_range_ordinal_to_the_cold_end() {
        // A plugin may declare more rungs than the ramp has tones. Anything past the last one is
        // colder still, so it renders as the cold end rather than falling back to unstyled — which
        // would make an icebox item look more urgent than a merely-low one.
        let t = Theme::new(true, ColorLevel::TrueColor);
        assert_eq!(t.priority(9, "icebox"), t.priority(4, "icebox"));
    }

    #[test]
    fn priority_ramp_emits_zero_escapes_on_a_plain_theme() {
        // The byte-stability guardrail, at the ramp: agents, `--json`, pipes, CI and trycmd see
        // the bare label for EVERY rung.
        let t = Theme::plain();
        for ord in 0..6 {
            let out = t.priority(ord, "P");
            assert_eq!(out, "P", "ordinal {ord} is verbatim: {out:?}");
        }
    }

    #[test]
    fn priority_ramp_uses_weight_not_color_when_color_is_suppressed() {
        // NO_COLOR / TERM=dumb on a TTY: the ramp keeps the weight axis (bold at the hot end, dim
        // at the cold end) and emits no color SGR at all.
        let t = Theme::new(true, ColorLevel::None);
        for ord in 0..6 {
            let out = t.priority(ord, "P");
            assert!(
                !out.contains("38;2") && !out.contains("38;5") && !out.contains("[3"),
                "ordinal {ord} carries no color: {out:?}"
            );
        }
        assert!(t.priority(0, "P").contains("\u{1b}[1m"), "hot end is bold");
        assert!(t.priority(4, "P").contains("\u{1b}[2m"), "cold end is dim");
        assert_eq!(t.priority(2, "P"), "P", "the middle has nothing to say");
    }

    #[test]
    fn priority_ramp_degrades_to_256_and_16_colors() {
        // The gradient survives as three distinguishable tones on a lesser terminal — never as a
        // truecolor sequence a 16-color terminal would print as garbage.
        let c256 = Theme::new(true, ColorLevel::Ansi256);
        assert!(
            c256.priority(0, "x").contains("38;5;124"),
            "deep ember → 124"
        );
        assert!(c256.priority(1, "x").contains("38;5;160"), "ember → 160");
        assert!(c256.priority(2, "x").contains("38;5;202"), "flame → 202");

        let c16 = Theme::new(true, ColorLevel::Ansi16);
        for ord in 0..5 {
            let out = c16.priority(ord, "x");
            assert!(
                !out.contains("38;2") && !out.contains("38;5"),
                "16-color never emits a deep sequence: {out:?}"
            );
        }
        assert!(c16.priority(1, "x").contains("31"), "ember → red");
        assert!(
            c16.priority(2, "x").contains("33"),
            "flame → yellow, the only warm tone left that is not red"
        );
    }

    #[test]
    fn ansi256_and_16_pick_red_not_truecolor() {
        let c256 = Theme::new(true, ColorLevel::Ansi256).accent("x");
        assert!(c256.contains("38;5;160"), "256 near-ember: {c256:?}");
        let c16 = Theme::new(true, ColorLevel::Ansi16).accent("x");
        // 16-color red is SGR 31; never a truecolor/256 sequence.
        assert!(
            c16.contains("31") && !c16.contains("38;2") && !c16.contains("38;5"),
            "16-color red: {c16:?}"
        );
    }
}
