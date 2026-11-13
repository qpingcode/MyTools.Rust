//! The selection through UI Automation, which most applications (browsers, Teams, Word,
//! Outlook) expose without touching the clipboard; otherwise (VS Code, Notepad++) by sending
//! Ctrl+C. Whether it can be replaced comes from, in order: the selected range's read-only
//! attribute, an editable value of the focused field, and a blinking caret.
use super::Selection;
use anyhow::Result;
use std::time::{Duration, Instant};
use windows::Win32::{
    Foundation::HWND,
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
            GetWindowThreadProcessId,
        },
    },
};

/// The longest selection read through UI Automation, in UTF-16 units.
const LIMIT: i32 = 16 * 1024;
/// How far up from the focused element a text pattern may sit (e.g. a page's document).
const ANCESTORS: usize = 12;

pub fn capture() -> Result<Selection> {
    let window = unsafe { GetForegroundWindow() };
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
    if selection.text.trim().is_empty() && !is_terminal(window) {
        let previous = arboard::Clipboard::new()
            .ok()
            .and_then(|mut clipboard| clipboard.get_text().ok());
        if let Some(text) = copy()? {
            selection.text = text;
            selection.previous_clipboard = Some(previous);
        }
    }
    selection.editable = read_only == Some(false)
        || focused.as_ref().is_some_and(editable_field)
        || has_caret(window);
    Ok(selection)
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
    let thread = unsafe { GetWindowThreadProcessId(window, None) };
    let mut info = GUITHREADINFO {
        cbSize: size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(thread, &mut info) }.is_ok() && !info.hwndCaret.is_invalid()
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

/// Copies the selection with Ctrl+C once the shortcut's keys are up, and returns the copied
/// text; `None` when nothing was copied.
fn copy() -> Result<Option<String>> {
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
    let before = unsafe { GetClipboardSequenceNumber() };
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
    let deadline = Instant::now() + Duration::from_millis(600);
    while unsafe { GetClipboardSequenceNumber() } == before {
        if Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // The owner may still be writing every format; give it a moment.
    std::thread::sleep(Duration::from_millis(30));
    for _ in 0..10 {
        if let Ok(text) = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            return Ok(Some(text));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(None)
}
