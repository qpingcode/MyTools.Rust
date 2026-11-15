use mytools_plugin_api::hotkeys::Hotkey;
use slint::{SharedString, platform::Key};

pub fn capture(text: SharedString, ctrl: bool, shift: bool, alt: bool, meta: bool) -> SharedString {
    let Some(key) = key_name(&text, shift) else {
        return SharedString::default();
    };
    Hotkey {
        ctrl,
        shift,
        alt,
        meta,
        key,
    }
    .to_string()
    .into()
}

fn key_name(text: &str, shift: bool) -> Option<String> {
    let mut chars = text.chars();
    let value = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if value.is_ascii_alphanumeric() {
        return Some(value.to_ascii_uppercase().to_string());
    }
    if shift
        && let Some(index) = "!@#$%^&*()_+{}|:\"<>?~"
            .chars()
            .position(|key| key == value)
    {
        return "1234567890-=[]\\;',./`"
            .chars()
            .nth(index)
            .map(|key| key.to_string());
    }
    if "`-=[]\\;',./".contains(value) {
        return Some(value.to_string());
    }
    if value == ' ' {
        return Some("Space".into());
    }
    let named = [
        (Key::Return, "Enter"),
        (Key::Tab, "Tab"),
        (Key::Escape, "Escape"),
        (Key::Backspace, "Backspace"),
        (Key::Delete, "Delete"),
        (Key::Insert, "Insert"),
        (Key::Home, "Home"),
        (Key::End, "End"),
        (Key::PageUp, "PageUp"),
        (Key::PageDown, "PageDown"),
        (Key::UpArrow, "Up"),
        (Key::DownArrow, "Down"),
        (Key::LeftArrow, "Left"),
        (Key::RightArrow, "Right"),
        (Key::F1, "F1"),
        (Key::F2, "F2"),
        (Key::F3, "F3"),
        (Key::F4, "F4"),
        (Key::F5, "F5"),
        (Key::F6, "F6"),
        (Key::F7, "F7"),
        (Key::F8, "F8"),
        (Key::F9, "F9"),
        (Key::F10, "F10"),
        (Key::F11, "F11"),
        (Key::F12, "F12"),
        (Key::F13, "F13"),
        (Key::F14, "F14"),
        (Key::F15, "F15"),
        (Key::F16, "F16"),
        (Key::F17, "F17"),
        (Key::F18, "F18"),
        (Key::F19, "F19"),
        (Key::F20, "F20"),
    ];
    named
        .into_iter()
        .find(|(key, _)| char::from(*key) == value)
        .map(|(_, name)| name.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_raw_key_events_to_shortcuts() {
        assert_eq!(
            capture("P".into(), true, true, false, false),
            "Ctrl+Shift+P"
        );
        assert_eq!(
            capture("!".into(), true, true, false, false),
            "Ctrl+Shift+1"
        );
        assert_eq!(capture(Key::F8.into(), false, false, false, false), "F8");
        assert!(capture(Key::Control.into(), true, false, false, false).is_empty());
        for reserved in ["Ctrl+A", "Ctrl+C", "Ctrl+V", "Alt+F4"] {
            assert!(
                Hotkey::parse(reserved).unwrap().is_system_reserved(),
                "{reserved}"
            );
        }
        assert!(!Hotkey::parse("Ctrl+Shift+P").unwrap().is_system_reserved());
    }
}
