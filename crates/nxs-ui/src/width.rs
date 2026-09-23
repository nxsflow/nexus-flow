//! Terminal-width logic (aye.25 point 2): detect the column count, apply the min/max rules (box
//! collapses below ~40 columns, content caps at ~80), and measure display width correctly with
//! `unicode-width` (a CJK/emoji cell is two columns — byte length is wrong for any non-ASCII text).
//!
//! The box renderer is presentation: it draws a framed box ONLY for a styled theme at adequate
//! width, and otherwise collapses to a plain heading + lines. That keeps non-TTY / `--json` /
//! piped output byte-stable plain ASCII (no Unicode box-drawing glyphs leak there).

use crate::style::Theme;
use unicode_width::UnicodeWidthStr;

/// Below this many columns the box collapses to plain lines — a frame would wrap and look broken.
pub const MIN_BOX_WIDTH: usize = 40;
/// Content never renders wider than this, however wide the terminal — long lines hurt readability.
pub const CONTENT_CAP: usize = 80;

/// The terminal's column count, or `None` when stdout is not a terminal (piped/redirected).
pub fn terminal_width() -> Option<usize> {
    terminal_size::terminal_size().map(|(terminal_size::Width(w), _)| w as usize)
}

/// The width to lay content out at: the terminal width clamped to [`CONTENT_CAP`], falling back to
/// the cap when the width is unknown (non-TTY callers render plain anyway).
pub fn content_width() -> usize {
    terminal_width().unwrap_or(CONTENT_CAP).min(CONTENT_CAP)
}

/// Display columns occupied by `s` (Unicode-aware), for wrapping + box padding.
pub fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Display columns occupied by `s`, IGNORING ANSI SGR escape sequences (`\x1b[…m`) — they are
/// zero-width on screen. Box content lines may already carry styling (e.g. a muted line), and those
/// escapes must not inflate the measured width, or wrapping/padding would misalign the frame.
fn visible_width(s: &str) -> usize {
    if !s.contains('\u{1b}') {
        return display_width(s); // fast path: no escapes
    }
    let mut visible = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip the escape sequence up to and including its final byte (`m` for SGR).
            for x in chars.by_ref() {
                if x.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            visible.push(c);
        }
    }
    display_width(&visible)
}

/// Whether a box should collapse to plain lines at `width` (below [`MIN_BOX_WIDTH`]).
pub fn should_collapse(width: usize) -> bool {
    width < MIN_BOX_WIDTH
}

/// Render a titled box around `lines`, picking the width from the terminal. Convenience over
/// [`boxed_at`] for the common case.
pub fn boxed(theme: &Theme, title: &str, lines: &[String]) -> String {
    boxed_at(theme, title, lines, content_width())
}

/// Render a titled box at an explicit `width` — the testable core. Collapses to a plain heading +
/// lines when the theme is unstyled (byte-stability: no box glyphs in plain output) or `width` is
/// below [`MIN_BOX_WIDTH`]. Otherwise draws a Unicode frame in the border tone with the title on
/// the top edge, each line padded to the inner width (Unicode-aware).
pub fn boxed_at(theme: &Theme, title: &str, lines: &[String], width: usize) -> String {
    if !theme.is_styled() || should_collapse(width) {
        // Plain fallback: heading then the lines, no frame. (heading() is plain under an unstyled
        // theme, so non-TTY output stays byte-stable ASCII.)
        let mut out = theme.heading(title);
        for line in lines {
            out.push('\n');
            out.push_str(line);
        }
        return out;
    }

    // nexus-flow-92k geometry: a `MARGIN`-space inner padding on each side, the title flush with the
    // body, and a blank padding row under the title edge + above the bottom edge so the box breathes.
    const MARGIN: usize = 2;
    let title_w = display_width(title);
    // The content area. Borders take 2 columns and the margins `2*MARGIN`; clamp up so the title
    // still fits when the terminal is narrow. The full box is then `inner + 2*MARGIN + 2` wide.
    let inner = width.saturating_sub(2 + 2 * MARGIN).max(title_w);
    let box_w = inner + 2 * MARGIN + 2;
    let margin = " ".repeat(MARGIN);

    // Top edge: `┌─ <title> ──────┐`. The `┌` + (MARGIN-1) rules + a space lead-in is exactly
    // `MARGIN + 1` columns, so the title sits flush with the body content (which starts after the
    // `│` + `MARGIN` spaces). Border-toned except the ember-accented title.
    let lead = format!("┌{} ", "─".repeat(MARGIN - 1));
    let trailing = box_w.saturating_sub((MARGIN + 1) + title_w + 2); // + the space + the `┐`
    let mut out = format!(
        "{}{} {}",
        theme.box_border(&lead),
        theme.accent(title),
        theme.box_border(&format!("{}┐", "─".repeat(trailing))),
    );

    // One content row: `│` + left margin + the inner-padded content + right margin + `│`.
    // Padding is measured by VISIBLE width, so a pre-styled (ANSI) content line still aligns.
    let render_row = |content: &str| {
        let pad = inner.saturating_sub(visible_width(content));
        format!(
            "{}{margin}{content}{}{margin}{}",
            theme.box_border("│"),
            " ".repeat(pad),
            theme.box_border("│"),
        )
    };
    let mut push_row = |row: String| {
        out.push('\n');
        out.push_str(&row);
    };

    push_row(render_row("")); // padding row under the title edge
    for line in lines {
        // Wrap each content line to the inner width FIRST (a line longer than `inner` would
        // otherwise get zero padding and shove the right border out — nexus-flow-lid), then render
        // each resulting row padded to the inner width so every `│` aligns.
        for row in wrap_to_width(line, inner) {
            push_row(render_row(&row));
        }
    }
    push_row(render_row("")); // padding row above the bottom edge

    out.push('\n');
    out.push_str(&theme.box_border(&format!("└{}┘", "─".repeat(box_w - 2))));
    out
}

/// Wrap `line` to at most `width` display columns per row (Unicode-aware, word-wise, hard-breaking
/// an over-long word). The public primitive over the box's internal wrapping, for callers that lay
/// out their own multi-line text — e.g. the picker's "learn more" details panel (nexus-flow-huy).
pub fn wrap(line: &str, width: usize) -> Vec<String> {
    wrap_to_width(line, width)
}

/// Wrap `line` to at most `width` display columns per row, Unicode-aware (rows are measured with
/// [`display_width`], so a CJK/emoji double-width cell counts as two columns — never a byte split).
/// Breaks on ASCII spaces between words (and on an embedded `\n`, a hard line break); a single word
/// wider than `width` is hard-broken at a column boundary rather than left to overflow the frame. An
/// empty line yields one empty row, so a box padding/spacer row is preserved (not collapsed to zero
/// rows). `width == 0` is returned verbatim.
fn wrap_to_width(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    // A literal newline is a hard line break: wrap each segment independently so an embedded `\n`
    // never lands inside a single rendered row (which would shove a box border mid-string).
    if line.contains('\n') {
        return line
            .split('\n')
            .flat_map(|segment| wrap_to_width(segment, width))
            .collect();
    }
    let mut rows: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for word in line.split(' ') {
        // Measure by VISIBLE width so a word carrying ANSI escapes (a pre-styled line) is not
        // counted as wider than it renders.
        let word_w = visible_width(word);
        let sep = usize::from(!cur.is_empty()); // a separating space, except on an empty row
        if cur_w + sep + word_w <= width {
            if sep == 1 {
                cur.push(' ');
                cur_w += 1;
            }
            cur.push_str(word);
            cur_w += word_w;
            continue;
        }
        // The word does not fit the current row: flush what we have, then place the word — whole if
        // it fits a fresh row, else hard-broken across rows at column boundaries.
        if !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if word_w <= width {
            cur.push_str(word);
            cur_w = word_w;
        } else {
            for ch in word.chars() {
                let ch_w = display_width(ch.encode_utf8(&mut [0u8; 4]));
                if cur_w + ch_w > width && !cur.is_empty() {
                    rows.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
                cur.push(ch);
                cur_w += ch_w;
            }
        }
    }
    rows.push(cur);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_counts_columns_not_bytes() {
        assert_eq!(display_width("abc"), 3);
        // A CJK glyph is two columns though it is three UTF-8 bytes.
        assert_eq!(display_width("世"), 2);
        assert_eq!(
            "世".len(),
            3,
            "…and three bytes — byte length would be wrong"
        );
    }

    #[test]
    fn content_width_caps_at_eighty() {
        // Whatever the terminal, content never exceeds the cap.
        assert!(content_width() <= CONTENT_CAP);
    }

    #[test]
    fn collapse_below_min_width() {
        assert!(should_collapse(MIN_BOX_WIDTH - 1));
        assert!(!should_collapse(MIN_BOX_WIDTH));
    }

    #[test]
    fn boxed_collapses_to_plain_when_unstyled() {
        // The byte-stability guard: a plain theme draws NO frame and NO escapes — just the heading
        // and the lines, plain ASCII. Width is irrelevant when unstyled.
        let out = boxed_at(&Theme::plain(), "Welcome", &["line one".into()], 80);
        assert!(!out.contains('\u{1b}'), "no escapes: {out:?}");
        assert!(
            !out.contains('┌') && !out.contains('│'),
            "no box glyphs: {out:?}"
        );
        assert_eq!(out, "Welcome\nline one");
    }

    #[test]
    fn boxed_collapses_to_plain_below_min_width_even_when_styled() {
        use crate::style::ColorLevel;
        let t = Theme::new(true, ColorLevel::TrueColor);
        let out = boxed_at(&t, "T", &["x".into()], MIN_BOX_WIDTH - 1);
        assert!(!out.contains('┌'), "too narrow → no frame: {out:?}");
    }

    /// A `ColorLevel::None` styled theme draws the frame but emits NO color escapes, so the BODY
    /// rows (border glyph + content, no accent) are plain — letting a test measure their display
    /// width and right-border alignment directly. (Only the title on the top edge is accented.)
    fn frame_theme() -> Theme {
        use crate::style::ColorLevel;
        Theme::new(true, ColorLevel::None)
    }

    /// The body rows of a rendered box (each `│ … │`), in order — the lines that must stay aligned.
    fn body_rows(out: &str) -> Vec<&str> {
        out.lines().filter(|l| l.starts_with('│')).collect()
    }

    /// Strip ANSI SGR escapes (`\x1b[…m`) so a styled line can be measured/compared by its visible
    /// text. Under `frame_theme` only the accented title carries escapes; this lets the top edge be
    /// compared like the (already-plain) body rows.
    fn strip_escapes(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                for x in chars.by_ref() {
                    if x == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// A box row is a "padding row" when its interior (between the two `│`) is all spaces.
    fn is_padding_row(line: &str) -> bool {
        line.starts_with('│')
            && line.ends_with('│')
            && line
                .trim_start_matches('│')
                .trim_end_matches('│')
                .chars()
                .all(|c| c == ' ')
    }

    #[test]
    fn boxed_at_wraps_a_long_line_so_the_right_border_aligns() {
        // nexus-flow-lid: a content line longer than the inner width used to get zero padding and
        // shove its `│` far to the right. It must wrap onto multiple padded rows, each closing its
        // border at the SAME column. (The box width still equals `width`; 92k narrowed the inner
        // content area to make room for the 2-space margins, but every edge stays aligned.)
        let t = frame_theme();
        let width = 40;
        let long = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda";
        assert!(
            display_width(long) > width,
            "fixture is longer than the inner width"
        );
        let out = boxed_at(&t, "T", &[long.to_string()], width);
        let rows = body_rows(&out);
        assert!(
            rows.len() >= 2,
            "the long line wrapped onto >1 row: {out:?}"
        );
        for row in &rows {
            assert!(row.ends_with('│'), "row keeps its right border: {row:?}");
            assert_eq!(
                display_width(row),
                width,
                "every row padded to the full box width so borders align: {row:?}"
            );
        }
        // No word is dropped across the wrap.
        let joined = rows
            .iter()
            .map(|r| r.trim_matches('│').trim())
            .collect::<Vec<_>>()
            .join(" ");
        for word in long.split_whitespace() {
            assert!(joined.contains(word), "word {word:?} preserved: {joined:?}");
        }
    }

    #[test]
    fn boxed_at_hard_breaks_a_word_longer_than_inner() {
        // A single unbreakable token wider than `inner` must be hard-broken across rows, never
        // allowed to overflow the frame.
        let t = frame_theme();
        let width = 40; // inner = 36
        let long_word = "x".repeat(50);
        let out = boxed_at(&t, "T", &[long_word], width);
        let rows = body_rows(&out);
        assert!(rows.len() >= 2, "unbreakable word hard-wrapped: {out:?}");
        for row in &rows {
            assert_eq!(
                display_width(row),
                width,
                "row padded to box width: {row:?}"
            );
        }
        let xs: usize = rows.iter().map(|r| r.matches('x').count()).sum();
        assert_eq!(xs, 50, "every character preserved across the hard break");
    }

    #[test]
    fn wrap_to_width_is_unicode_aware_not_byte_wrap() {
        // `世` is two columns but three bytes. With width 4, two fill a row and the third wraps —
        // a byte-wrap would split mid-glyph or miscount.
        assert_eq!(
            wrap_to_width("世世世", 4),
            vec!["世世".to_string(), "世".to_string()]
        );
    }

    #[test]
    fn wrap_to_width_splits_on_embedded_newlines() {
        // A literal `\n` is a hard line break: each segment wraps independently, so an embedded
        // newline never lands inside a single rendered row (which would shove a box border
        // mid-string). Each segment is still wrapped to the width.
        assert_eq!(
            wrap_to_width("alpha\nbeta", 40),
            vec!["alpha".to_string(), "beta".to_string()]
        );
    }

    #[test]
    fn boxed_at_renders_a_newline_containing_line_as_separate_aligned_rows() {
        // Defensive (Integrity #2): a content line carrying a `\n` renders as separate, fully-padded
        // rows — never one row with the right border buried mid-string.
        let t = frame_theme();
        let out = boxed_at(&t, "T", &["first\nsecond".to_string()], 40);
        let rows = body_rows(&out);
        assert!(
            rows.iter()
                .any(|r| r.contains("first") && !r.contains("second")),
            "first segment is its own row: {out:?}"
        );
        assert!(
            rows.iter()
                .any(|r| r.contains("second") && !r.contains("first")),
            "second segment is its own row: {out:?}"
        );
        for r in &rows {
            assert_eq!(
                display_width(r),
                40,
                "each row padded to the box width: {r:?}"
            );
        }
    }

    #[test]
    fn wrap_to_width_keeps_an_empty_line_as_one_empty_row() {
        // The init box uses blank spacer lines; wrapping must not drop them to zero rows (that would
        // delete a padding row), so an empty line yields exactly one empty row.
        assert_eq!(wrap_to_width("", 36), vec![String::new()]);
    }

    #[test]
    fn boxed_at_aligns_the_title_flush_with_the_body_at_a_two_space_margin() {
        // nexus-flow-92k: the title used to sit one column right of the body (`┌─ title` vs
        // `│ line`), reading as a random indent. Now the `┌─ ` lead-in and the `│  ` body margin
        // both place their text at the SAME column, with a 2-space inner margin on each side.
        let t = frame_theme();
        let out = boxed_at(&t, "Forge", &["hello".to_string()], 60);
        let lines: Vec<String> = out.lines().map(strip_escapes).collect();
        assert!(
            lines[0].starts_with("┌─ Forge"),
            "title flush after a 2-column lead-in: {:?}",
            lines[0]
        );
        let body = lines
            .iter()
            .find(|l| l.starts_with('│') && l.contains("hello"))
            .expect("a content row");
        assert!(
            body.starts_with("│  hello"),
            "body uses a 2-space left margin, flush with the title: {body:?}"
        );
        assert!(
            body.ends_with("  │"),
            "and a 2-space right margin before the border: {body:?}"
        );
    }

    #[test]
    fn boxed_at_breathes_with_a_padding_row_under_the_title_and_above_the_bottom() {
        // 92k: the box content used to sit flush against the top/bottom edges (gedrungen). One empty
        // padding row directly under the title edge and one directly above the bottom edge give it a
        // calmer rhythm.
        let t = frame_theme();
        let out = boxed_at(&t, "T", &["content".to_string()], 50);
        let lines: Vec<&str> = out.lines().collect();
        let n = lines.len();
        assert!(
            n >= 5,
            "top + padding + content + padding + bottom: {out:?}"
        );
        assert!(
            is_padding_row(lines[1]),
            "a blank padding row sits under the title: {:?}",
            lines[1]
        );
        assert!(
            is_padding_row(lines[n - 2]),
            "a blank padding row sits above the bottom: {:?}",
            lines[n - 2]
        );
        assert!(
            strip_escapes(lines[0]).starts_with('┌') && lines[n - 1].starts_with('└'),
            "framed top and bottom edges: {out:?}"
        );
    }

    #[test]
    fn boxed_at_measures_visible_width_ignoring_ansi_escapes() {
        // A pre-styled content line (e.g. a themed `muted` hint) carries ANSI SGR
        // escapes that are ZERO width on screen. The box must pad by the VISIBLE width so the right
        // border still aligns, and must not wrap a line that visibly fits just because the escapes
        // inflate the byte/measured length.
        let t = frame_theme();
        let visible = "a muted hint line, dimmed on a terminal!"; // 40 visible columns
        assert_eq!(display_width(visible), 40);
        let styled = format!("\u{1b}[2m{visible}\u{1b}[0m"); // dim wrapper, +escapes
        let out = boxed_at(&t, "T", &[styled], 50); // inner = 44, so 40 visible fits one row
        let rows = body_rows(&out);
        // Count every non-padding row, not only the ones carrying the head of the text: a wrap
        // would leave the head on one row and push its tail onto a second.
        let texted: Vec<&&str> = rows.iter().filter(|r| !is_padding_row(r)).collect();
        assert_eq!(
            texted.len(),
            1,
            "the styled line fits one row, not split: {out:?}"
        );
        assert!(texted[0].contains("muted hint"), "{out:?}");
        assert_eq!(
            display_width(&strip_escapes(texted[0])),
            50,
            "padded to the box width by VISIBLE columns (escapes are zero-width): {:?}",
            texted[0]
        );
    }

    #[test]
    fn boxed_at_keeps_every_edge_at_one_width() {
        // Every line — top, padding rows, wrapped content, bottom — shares one visible width, so the
        // frame is a clean rectangle at any content length.
        let t = frame_theme();
        let out = boxed_at(
            &t,
            "Forge",
            &["a longer line of content that will wrap across the inner width here".to_string()],
            50,
        );
        let widths: Vec<usize> = out
            .lines()
            .map(|l| display_width(&strip_escapes(l)))
            .collect();
        assert!(
            widths.iter().all(|&w| w == widths[0]),
            "all rows share one width: {widths:?}"
        );
        assert_eq!(widths[0], 50, "and that width is the requested box width");
    }

    #[test]
    fn boxed_draws_a_frame_when_styled_and_wide_enough() {
        use crate::style::ColorLevel;
        let t = Theme::new(true, ColorLevel::TrueColor);
        let out = boxed_at(&t, "Forge", &["build complete".into()], 60);
        assert!(out.contains('┌') && out.contains('┘'), "framed: {out:?}");
        assert!(out.contains("Forge"), "title on the top edge: {out:?}");
    }
}
