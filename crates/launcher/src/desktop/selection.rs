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

/// A foreground window sampled when the attachment shortcut fired.
#[derive(Clone, Copy)]
pub struct Source {
    #[cfg(windows)]
    window: isize,
}

impl Source {
    pub fn foreground() -> Self {
        Self {
            #[cfg(windows)]
            window: unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() }.0
                as isize,
        }
    }

    pub fn capture(self) -> Result<Option<mytools_plugin_api::CommandArgument>> {
        #[cfg(windows)]
        {
            use mytools_plugin_api::CommandArgument;
            use windows::Win32::{
                Foundation::HWND,
                UI::WindowsAndMessaging::{
                    GetClassNameW, GetForegroundWindow, GetWindowThreadProcessId,
                },
            };
            let window = HWND(self.window as *mut _);
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
            let unavailable = if pid == 0 {
                Some("no_source_window")
            } else if pid == std::process::id() {
                Some("source_is_launcher")
            } else if unsafe { GetForegroundWindow() } != window {
                Some("foreground_changed")
            } else {
                None
            };
            if let Some(reason) = unavailable {
                mytools_plugin_api::log::info("selection.no_source", reason);
                return Ok(None);
            }
            let mut name = [0; 32];
            let len = unsafe { GetClassNameW(window, &mut name) }.max(0) as usize;
            if matches!(
                String::from_utf16_lossy(&name[..len]).as_str(),
                "CabinetWClass" | "ExploreWClass"
            ) || explorer::is_desktop_window(window)
            {
                let paths =
                    explorer::selected_files(window, &crate::i18n::t("explorer_probe.ambiguous"))?;
                return Ok(paths
                    .filter(|paths| !paths.is_empty())
                    .map(|paths| CommandArgument::Files { paths }));
            }
            let selection = platform::capture_from(window, true)?;
            Ok(
                (!selection.text.is_empty()).then_some(CommandArgument::Text {
                    text: selection.text,
                }),
            )
        }
        #[cfg(not(windows))]
        anyhow::bail!(crate::i18n::t("selection.unsupported"))
    }
}

#[cfg(windows)]
#[path = "explorer_windows.rs"]
mod explorer;

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
