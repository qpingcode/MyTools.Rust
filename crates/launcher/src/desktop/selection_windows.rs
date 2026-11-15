//! The selection through UI Automation, which most applications (browsers, Teams, Word,
//! Outlook) expose without touching the clipboard; otherwise (VS Code, Notepad++) by sending
//! Ctrl+C. Whether it can be replaced comes from, in order: the selected range's read-only
//! attribute, an editable value of the focused field, and a blinking caret.
use super::Selection;
use anyhow::Result;
use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::{
        Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        DataExchange::GetClipboardSequenceNumber,
        Variant::{VT_BOOL, VariantClear},
    },
    UI::{
        Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
            IUIAutomationValuePattern, UIA_EditControlTypeId, UIA_IsReadOnlyAttributeId,
            UIA_TextPatternId, UIA_ValuePatternId,
        },
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
            KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_C, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN,
            VK_SHIFT,
        },
        WindowsAndMessaging::{
            GUITHREADINFO, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo,
            GetWindowThreadProcessId, IsChild, SMTO_ABORTIFHUNG, SendMessageTimeoutW,
        },
    },
};

/// The longest selection read through UI Automation, in UTF-16 units.
const LIMIT: i32 = 16 * 1024;
/// How far up from the focused element a text pattern may sit (e.g. a page's document).
const ANCESTORS: usize = 12;

pub fn capture() -> Result<Selection> {
    capture_from(unsafe { GetForegroundWindow() }, false)
}

pub(super) fn capture_from(window: HWND, attachment: bool) -> Result<Selection> {
    anyhow::ensure!(
        unsafe { GetForegroundWindow() } == window,
        crate::i18n::t("selection.no_window")
    );
    anyhow::ensure!(!window.is_invalid(), crate::i18n::t("selection.no_window"));
    let _com = Com::new();
    let automation: Option<IUIAutomation> =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok();
    let focused = automation
        .as_ref()
        .and_then(|automation| unsafe { automation.GetFocusedElement() }.ok());
    let mut selection = Selection::default();
    let mut read_only = None;
    if let (Some(automation), Some(focused)) = (&automation, &focused)
        && let Some((text, range_read_only)) = selected_range(automation, focused)
    {
        selection.text = text;
        read_only = range_read_only;
    }
    if selection.text.trim().is_empty() {
        let uia = match (&automation, &focused) {
            (None, _) => "automation_unavailable",
            (Some(_), None) => "no_focused_element",
            (Some(_), Some(_)) => "no_selected_text",
        };
        if is_terminal(window) {
            log_copy_miss(attachment, uia, "skipped_terminal");
        } else {
            match copy_selection(window, attachment)? {
                Ok((text, previous)) => {
                    if text.is_empty() {
                        log_copy_miss(attachment, uia, "copied_empty_text");
                    }
                    selection.text = text;
                    selection.previous_clipboard = previous;
                }
                Err(reason) => log_copy_miss(attachment, uia, reason),
            }
        }
    }
    selection.editable = read_only == Some(false)
        || focused.as_ref().is_some_and(editable_field)
        || has_caret(window);
    Ok(selection)
}

fn log_copy_miss(attachment: bool, uia: &str, copy: &str) {
    if attachment {
        mytools_plugin_api::log::info("selection.no_text", format!("uia={uia} copy={copy}"));
    }
}

struct Com(bool);
impl Com {
    fn new() -> Self {
        Self(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok())
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}

/// The selected text of the nearest element with a text pattern, and whether that range is
/// read-only (`None` when the application does not say).
fn selected_range(
    automation: &IUIAutomation,
    focused: &IUIAutomationElement,
) -> Option<(String, Option<bool>)> {
    let walker = unsafe { automation.ControlViewWalker() }.ok()?;
    let mut element = focused.clone();
    for _ in 0..ANCESTORS {
        if let Ok(pattern) =
            unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
        {
            let ranges = unsafe { pattern.GetSelection() }.ok()?;
            let count = unsafe { ranges.Length() }.unwrap_or(0);
            let mut text = String::new();
            let mut read_only = None;
            for index in 0..count {
                let Ok(range) = (unsafe { ranges.GetElement(index) }) else {
                    continue;
                };
                if let Ok(part) = unsafe { range.GetText(LIMIT) } {
                    text.push_str(&part.to_string());
                }
                if read_only.is_none() {
                    read_only = unsafe { range.GetAttributeValue(UIA_IsReadOnlyAttributeId) }
                        .ok()
                        .and_then(|mut value| {
                            let flag = unsafe {
                                let inner = &value.Anonymous.Anonymous;
                                (inner.vt == VT_BOOL).then(|| inner.Anonymous.boolVal.0 != 0)
                            };
                            let _ = unsafe { VariantClear(&mut value) };
                            flag
                        });
                }
            }
            return Some((text, read_only));
        }
        element = unsafe { walker.GetParentElement(&element) }.ok()?;
    }
    None
}

/// A focused edit control whose value may be changed.
fn editable_field(element: &IUIAutomationElement) -> bool {
    let edit =
        unsafe { element.CurrentControlType() }.is_ok_and(|kind| kind == UIA_EditControlTypeId);
    edit && unsafe { element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }
        .and_then(|pattern| unsafe { pattern.CurrentIsReadOnly() })
        .is_ok_and(|read_only| !read_only.as_bool())
}

/// Classic Win32 editors (Notepad++, Notepad) show a system caret while editing.
fn has_caret(window: HWND) -> bool {
    gui_thread_info(window).is_some_and(|info| !info.hwndCaret.is_invalid())
}

fn gui_thread_info(window: HWND) -> Option<GUITHREADINFO> {
    let thread = unsafe { GetWindowThreadProcessId(window, None) };
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(thread, &mut info) }.ok()?;
    Some(info)
}

/// In a console, Ctrl+C without a selection stops the running program.
fn is_terminal(window: HWND) -> bool {
    let mut name = [0u16; 64];
    let length = unsafe { GetClassNameW(window, &mut name) }.max(0) as usize;
    let name = String::from_utf16_lossy(&name[..length]);
    matches!(
        name.as_str(),
        "ConsoleWindowClass" | "CASCADIA_HOSTING_WINDOW_CLASS" | "mintty" | "VirtualConsoleClass"
    )
}

#[path = "selection_clipboard_windows.rs"]
mod clipboard;

type CopiedSelection = (String, Option<Option<String>>);

fn copy_selection(
    window: HWND,
    attachment: bool,
) -> Result<std::result::Result<CopiedSelection, &'static str>> {
    if attachment {
        let snapshot = match clipboard::Snapshot::capture() {
            Ok(snapshot) => snapshot,
            Err(reason) => return Ok(Err(reason)),
        };
        return Ok(copy(window, Some(snapshot))?.map(|text| (text, None)));
    }
    let previous = arboard::Clipboard::new()
        .ok()
        .and_then(|mut clipboard| clipboard.get_text().ok());
    Ok(copy(window, None)?.map(|text| (text, Some(previous))))
}

/// Copies from a focused Scintilla editor directly, or sends Ctrl+C to other applications.
fn copy(
    window: HWND,
    snapshot: Option<clipboard::Snapshot>,
) -> Result<std::result::Result<String, &'static str>> {
    let editor = snapshot.as_ref().and_then(|_| focused_scintilla(window));
    let before = if let Some(editor) = editor {
        anyhow::ensure!(
            unsafe { GetForegroundWindow() } == window && focused_scintilla(window) == Some(editor),
            crate::i18n::t("selection.no_window")
        );
        const SCI_COPY: u32 = 2178;
        let before = copy_sequence(snapshot.as_ref())?;
        let dispatched = unsafe {
            SendMessageTimeoutW(
                editor,
                SCI_COPY,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                500,
                None,
            )
        };
        if dispatched.0 == 0 {
            return Ok(Err("scintilla_copy_failed"));
        }
        before
    } else {
        send_copy_keys(window, snapshot.as_ref())?
    };
    let deadline = Instant::now() + Duration::from_millis(600);
    while unsafe { GetClipboardSequenceNumber() } == before {
        if Instant::now() >= deadline {
            return Ok(Err(if editor.is_some() {
                "scintilla_clipboard_did_not_change"
            } else {
                "clipboard_did_not_change"
            }));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // The owner may still be writing every format; give it a moment.
    std::thread::sleep(Duration::from_millis(30));
    let copied_sequence = unsafe { GetClipboardSequenceNumber() };
    let mut result = Err("copied_text_unavailable");
    for _ in 0..10 {
        if let Ok(text) = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            result = Ok(text);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if let Some(snapshot) = snapshot {
        snapshot.restore(copied_sequence)?;
    }
    Ok(result)
}

fn focused_scintilla(window: HWND) -> Option<HWND> {
    let focus = gui_thread_info(window)?.hwndFocus;
    if !unsafe { IsChild(window, focus) }.as_bool() {
        return None;
    }
    let mut name = [0; 32];
    let len = unsafe { GetClassNameW(focus, &mut name) }.max(0) as usize;
    (String::from_utf16_lossy(&name[..len]) == "Scintilla").then_some(focus)
}

fn copy_sequence(snapshot: Option<&clipboard::Snapshot>) -> Result<u32> {
    let sequence = unsafe { GetClipboardSequenceNumber() };
    anyhow::ensure!(
        snapshot.is_none_or(|snapshot| snapshot.sequence == sequence),
        "clipboard_changed_before_copy"
    );
    Ok(sequence)
}

fn send_copy_keys(window: HWND, snapshot: Option<&clipboard::Snapshot>) -> Result<u32> {
    let modifiers = [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN];
    let released = || {
        modifiers
            .iter()
            .all(|key| unsafe { GetAsyncKeyState(key.0 as i32) } >= 0)
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !released() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    anyhow::ensure!(released(), crate::i18n::t("selection.keys_held"));
    anyhow::ensure!(
        unsafe { GetForegroundWindow() } == window,
        crate::i18n::t("selection.no_window")
    );
    let before = copy_sequence(snapshot)?;
    let key = |code: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: code,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let none = KEYBD_EVENT_FLAGS(0);
    let inputs = [
        key(VK_CONTROL, none),
        key(VK_C, none),
        key(VK_C, KEYEVENTF_KEYUP),
        key(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
    anyhow::ensure!(
        sent as usize == inputs.len(),
        crate::i18n::t("selection.failed")
    );
    Ok(before)
}
