//! Reading the text selected in the application in front, for commands that work on it
//! (such as translating a selection in Teams or a web page).
use anyhow::Result;

#[cfg(windows)]
#[path = "selection_windows.rs"]
mod platform;

/// What [`capture`] found.
#[derive(Debug, Default)]
pub struct Selection {
    /// As selected, including leading or trailing whitespace; empty when nothing is selected.
    pub text: String,
    /// Whether the selection is in a text field, so pasting over it replaces it.
    pub editable: bool,
    /// The clipboard text before it was used to read the selection, to restore afterwards;
    /// `None` when the clipboard was not touched.
    pub previous_clipboard: Option<Option<String>>,
}

/// Reads the selection of the window in front. It blocks for up to a few seconds while the
/// shortcut's keys are held, so call it off the UI thread.
pub fn capture() -> Result<Selection> {
    #[cfg(windows)]
    return platform::capture();
    #[cfg(not(windows))]
    anyhow::bail!(crate::i18n::t("selection.unsupported"))
}

/// Keeps the whitespace around the original selection, which some applications include
/// (e.g. a trailing line break in Outlook), around its replacement.
pub fn replacement(selected: &str, result: &str) -> String {
    let start = selected.len() - selected.trim_start().len();
    let end = selected.trim_end().len();
    if start >= end {
        return result.to_owned();
    }
    format!(
        "{}{}{}",
        &selected[..start],
        result.trim(),
        &selected[end..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_keeps_the_selections_surrounding_whitespace() {
        assert_eq!(replacement("hello", "你好"), "你好");
        assert_eq!(replacement(" hello world\r", "你好世界\n"), " 你好世界\r");
        assert_eq!(replacement("\n\tgood\n", "好"), "\n\t好\n");
        assert_eq!(replacement("  ", "x"), "x");
    }
}
