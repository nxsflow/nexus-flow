//! The reusable arrow-key chooser widget (aye.25 point 5), manufakt-Forge themed. Up/Down + Enter;
//! the recommended default is pre-selected and ember-highlighted; each option shows a short label
//! plus a plain-text description line. Respects `NO_COLOR` (drops the ember tint, keeps weight) and
//! is **never** invoked off a TTY — the interactive prompt returns [`ChooseError::NotInteractive`]
//! rather than corrupting a pipe, so callers can fall back to a flag/error.
//!
//! This is exactly the widget `nxs init` (module selection, S4) and `nxf init` (the plugin
//! chooser, hai.3) share, instead of each building its own.

use std::fmt;

/// One selectable option: a short `label` and a plain-text `description` line shown beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub description: String,
}

impl Choice {
    pub fn new(label: impl Into<String>, description: impl Into<String>) -> Choice {
        Choice {
            label: label.into(),
            description: description.into(),
        }
    }
}

/// Why an interactive choose could not return a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChooseError {
    /// stdin/stderr is not a terminal — the caller must offer a non-interactive path instead.
    NotInteractive,
    /// No choices were offered (a caller bug — nothing to select).
    Empty,
    /// The user aborted (Esc / Ctrl-C).
    Cancelled,
    /// The terminal backend failed.
    Io(String),
}

impl fmt::Display for ChooseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChooseError::NotInteractive => write!(f, "no interactive terminal for the chooser"),
            ChooseError::Empty => write!(f, "no choices to select from"),
            ChooseError::Cancelled => write!(f, "selection cancelled"),
            ChooseError::Io(e) => write!(f, "chooser failed: {e}"),
        }
    }
}

impl std::error::Error for ChooseError {}

/// The display line for one option in the list: `label — description`. Pure (unit-testable without
/// a terminal); the same rendering the interactive list shows.
pub fn choice_line(choice: &Choice) -> String {
    if choice.description.is_empty() {
        choice.label.clone()
    } else {
        format!("{} — {}", choice.label, choice.description)
    }
}

/// Whether the current process can host the interactive chooser (stdin AND stderr are terminals —
/// inquire reads keys from stdin and draws to stderr).
pub fn is_interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Prompt the user to pick one of `choices` with the arrow keys, `default_index` pre-selected.
/// Returns the chosen index. Requires a TTY — returns [`ChooseError::NotInteractive`] otherwise so
/// the caller can fall back (a flag, or a loud error). manufakt-Forge themed; honors `NO_COLOR`.
pub fn choose(
    prompt: &str,
    choices: &[Choice],
    default_index: usize,
) -> Result<usize, ChooseError> {
    if choices.is_empty() {
        return Err(ChooseError::Empty);
    }
    if !is_interactive() {
        return Err(ChooseError::NotInteractive);
    }
    let start = default_index.min(choices.len() - 1);
    let options: Vec<String> = choices.iter().map(choice_line).collect();
    let result = inquire::Select::new(prompt, options)
        .with_starting_cursor(start)
        .with_render_config(forge_render_config())
        .raw_prompt();
    match result {
        Ok(opt) => Ok(opt.index),
        Err(inquire::InquireError::OperationCanceled)
        | Err(inquire::InquireError::OperationInterrupted) => Err(ChooseError::Cancelled),
        Err(e) => Err(ChooseError::Io(e.to_string())),
    }
}

/// Prompt the user to pick ZERO OR MORE of `choices` with the arrow keys + space to toggle,
/// `default_selected` pre-checked. Returns the chosen indices (into `choices`), in option order.
/// The multi-select sibling of [`choose`] — same TTY gate (returns [`ChooseError::NotInteractive`]
/// off a terminal so a caller can fall back to flags), same manufakt-Forge theme. This is the
/// `nxs init` module picker ("Which tools do you want to use?"), with the recommended set checked.
pub fn choose_many(
    prompt: &str,
    choices: &[Choice],
    default_selected: &[usize],
) -> Result<Vec<usize>, ChooseError> {
    if choices.is_empty() {
        return Err(ChooseError::Empty);
    }
    if !is_interactive() {
        return Err(ChooseError::NotInteractive);
    }
    let options: Vec<String> = choices.iter().map(choice_line).collect();
    let result = inquire::MultiSelect::new(prompt, options)
        .with_default(default_selected)
        .with_render_config(forge_render_config())
        .raw_prompt();
    match result {
        Ok(selected) => Ok(selected.into_iter().map(|opt| opt.index).collect()),
        Err(inquire::InquireError::OperationCanceled)
        | Err(inquire::InquireError::OperationInterrupted) => Err(ChooseError::Cancelled),
        Err(e) => Err(ChooseError::Io(e.to_string())),
    }
}

/// The manufakt-Forge render config, branching on the active `NO_COLOR` state.
fn forge_render_config() -> inquire::ui::RenderConfig<'static> {
    forge_render_config_for(anstyle_query::no_color())
}

/// The manufakt-Forge render config (nexus-flow-0yz) with the `NO_COLOR` decision injected so both
/// branches are unit-testable. Ember is the CURSOR accent, used **sparingly**: only the `›` cursor
/// prefix carries it, while the highlighted row itself is weight-only (bold) — never a full ember
/// tint, so a list with several `[x]` is not a red wall. Selection is a SEPARATE channel: the
/// checked marker is weight-only (a bold `[x]`, no color), so the cursor (ember arrow + bold row)
/// and a selection (bold checkbox) stay visually distinct. Under `NO_COLOR` the only difference is
/// the cursor prefix dropping its ember — everything else is already weight, never a raw color code
/// (the default configs tint the checkbox green, which this overrides away in both branches).
fn forge_render_config_for(no_color: bool) -> inquire::ui::RenderConfig<'static> {
    use inquire::ui::{Attributes, Color, RenderConfig, StyleSheet, Styled};

    // Selection is marked by WEIGHT, not color, in EVERY mode: a bold `[x]` for a checked option, a
    // plain `[ ]` otherwise — distinct from the cursor's ember arrow and never a wall of red. This
    // also overrides the default configs' green `[x]` (a stray color code, even under NO_COLOR).
    let selected_checkbox = Styled::new("[x]").with_attr(Attributes::BOLD);
    let unselected_checkbox = Styled::new("[ ]");
    // The highlighted (cursor) row is bold — weight, not a full ember tint.
    let cursor_row = StyleSheet::new().with_attr(Attributes::BOLD);

    if no_color {
        // NO_COLOR: weight only, no ember anywhere — the cursor arrow is a plain `›`.
        return RenderConfig::default()
            .with_highlighted_option_prefix(Styled::new("›"))
            .with_selected_option(Some(cursor_row))
            .with_selected_checkbox(selected_checkbox)
            .with_unselected_checkbox(unselected_checkbox);
    }
    let ember = Color::rgb(0xDC, 0x26, 0x26);
    RenderConfig::default_colored()
        .with_prompt_prefix(Styled::new("›").with_fg(ember))
        // Ember marks ONLY the cursor arrow (sparing); the row stays weight-only.
        .with_highlighted_option_prefix(Styled::new("›").with_fg(ember))
        .with_selected_option(Some(cursor_row))
        .with_selected_checkbox(selected_checkbox)
        .with_unselected_checkbox(unselected_checkbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_line_pairs_label_and_description() {
        let c = Choice::new("issue-tracker", "epics + issues, P0–P4 ranking");
        assert_eq!(
            choice_line(&c),
            "issue-tracker — epics + issues, P0–P4 ranking"
        );
    }

    #[test]
    fn choice_line_omits_the_dash_when_there_is_no_description() {
        assert_eq!(choice_line(&Choice::new("flow", "")), "flow");
    }

    #[test]
    fn choose_with_no_choices_is_empty_error() {
        assert_eq!(
            choose("pick", &[], 0).unwrap_err(),
            ChooseError::Empty,
            "nothing to select from is a caller bug, surfaced loudly"
        );
    }

    #[test]
    fn choose_off_a_tty_refuses_rather_than_corrupting_the_pipe() {
        // Under `cargo test` stdin/stderr are not terminals — the interactive prompt must refuse so
        // a caller knows to fall back, never block or scribble control codes into a pipe.
        let choices = [Choice::new("a", "first"), Choice::new("b", "second")];
        assert_eq!(
            choose("pick", &choices, 0).unwrap_err(),
            ChooseError::NotInteractive
        );
    }

    #[test]
    fn render_config_constructs_in_both_color_modes() {
        // Build both branches (color + NO_COLOR) so the themed config can't silently fail to build.
        let _ = forge_render_config();
        let _ = forge_render_config_for(true);
        let _ = forge_render_config_for(false);
    }

    #[test]
    fn forge_config_marks_the_cursor_with_ember_but_leaves_the_row_untinted() {
        // 0yz: ember is the CURSOR accent and is used sparingly — only the `›` prefix carries it.
        // The highlighted row itself is weight-only (bold), NEVER fully ember-tinted, so a list of
        // selections does not become a red wall.
        use inquire::ui::{Attributes, Color};
        let cfg = forge_render_config_for(false);
        assert_eq!(
            cfg.highlighted_option_prefix.style.fg,
            Some(Color::rgb(0xDC, 0x26, 0x26)),
            "the cursor arrow carries ember"
        );
        let row = cfg.selected_option.expect("the cursor row is styled");
        assert!(row.att.contains(Attributes::BOLD), "cursor row is bold");
        assert_eq!(
            row.fg, None,
            "cursor row is weight-only, not ember-tinted (no Volltönung)"
        );
    }

    #[test]
    fn forge_config_separates_selection_from_cursor_by_weight_not_ember() {
        // The checked marker is its OWN channel: a weight-only (bold) `[x]`, never ember. So the
        // cursor (ember arrow + bold row) and a selection (bold checkbox) read as distinct, and the
        // selected rows carry no red tint.
        use inquire::ui::Attributes;
        let cfg = forge_render_config_for(false);
        assert!(
            cfg.selected_checkbox.style.att.contains(Attributes::BOLD),
            "a checked option is marked by weight"
        );
        assert_eq!(
            cfg.selected_checkbox.style.fg, None,
            "the selection marker carries no ember (no red wall of [x])"
        );
    }

    #[test]
    fn forge_config_no_color_path_is_weight_only_everywhere() {
        // NO_COLOR keeps the SAME structure but drops every color: cursor arrow, cursor row, and the
        // selection marker are all weight-only (no fg), with the checked marker still bold so
        // selection stays visible without color.
        use inquire::ui::Attributes;
        let cfg = forge_render_config_for(true);
        assert_eq!(
            cfg.highlighted_option_prefix.style.fg, None,
            "no ember arrow"
        );
        assert_eq!(
            cfg.selected_option.expect("cursor row styled").fg,
            None,
            "cursor row weight-only"
        );
        assert_eq!(cfg.selected_checkbox.style.fg, None, "no colored checkbox");
        assert!(
            cfg.selected_checkbox.style.att.contains(Attributes::BOLD),
            "checked marker still distinguished by weight under NO_COLOR"
        );
    }

    #[test]
    fn choose_many_with_no_choices_is_empty_error() {
        assert_eq!(
            choose_many("pick", &[], &[]).unwrap_err(),
            ChooseError::Empty
        );
    }

    #[test]
    fn choose_many_off_a_tty_refuses_rather_than_corrupting_the_pipe() {
        // Same TTY guard as `choose`: under `cargo test` stdin/stderr are not terminals, so the
        // multi-select must refuse (the caller falls back to flags) instead of blocking a pipe.
        let choices = [
            Choice::new("flow", "issue tracker"),
            Choice::new("memory", "agent memory"),
        ];
        assert_eq!(
            choose_many("Which tools?", &choices, &[0]).unwrap_err(),
            ChooseError::NotInteractive
        );
    }
}
