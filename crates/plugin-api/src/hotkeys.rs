//! Global shortcut text as manifests, settings and the host use it, e.g. `Ctrl+Shift+Alt+V`.
//!
//! Parsing accepts the usual spellings of modifiers and keys; [`Hotkey`]'s `Display` gives
//! one canonical spelling, so the settings window can compare and show shortcuts, and
//! [`Hotkey::registration`] gives the spelling the host's hotkey library registers.
use std::{fmt, path::PathBuf};

/// The host-owned global shortcuts in config.json.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub struct LauncherHotkeys {
    pub launcher_hotkey: String,
    pub selection_hotkey: Option<String>,
}

impl Default for LauncherHotkeys {
    fn default() -> Self {
        Self {
            launcher_hotkey: "Alt+Space".into(),
            selection_hotkey: Some("Alt+X".into()),
        }
    }
}

impl LauncherHotkeys {
    pub fn selection(&self) -> Option<&str> {
        if cfg!(windows) {
            self.selection_hotkey.as_deref()
        } else {
            None
        }
    }
}

pub fn launcher_config_path() -> PathBuf {
    std::env::var_os("MYTOOLS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::dirs::root().join("config.json"))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Windows key; Command on macOS.
    pub meta: bool,
    /// The canonical key name, e.g. `V`, `F5`, `Space`.
    pub key: String,
}

/// Named keys: the accepted spellings (upper case) and the canonical name.
const NAMED: &[(&[&str], &str)] = &[
    (&["SPACE"], "Space"),
    (&["ENTER", "RETURN"], "Enter"),
    (&["TAB"], "Tab"),
    (&["ESC", "ESCAPE"], "Escape"),
    (&["BACKSPACE"], "Backspace"),
    (&["DELETE", "DEL"], "Delete"),
    (&["INSERT", "INS"], "Insert"),
    (&["HOME"], "Home"),
    (&["END"], "End"),
    (&["PAGEUP", "PGUP"], "PageUp"),
    (&["PAGEDOWN", "PGDN"], "PageDown"),
    (&["UP", "ARROWUP"], "Up"),
    (&["DOWN", "ARROWDOWN"], "Down"),
    (&["LEFT", "ARROWLEFT"], "Left"),
    (&["RIGHT", "ARROWRIGHT"], "Right"),
    (&["PRINTSCREEN"], "PrintScreen"),
    (&["`", "BACKQUOTE"], "`"),
    (&["-", "MINUS"], "-"),
    (&["=", "EQUAL"], "="),
    (&["[", "BRACKETLEFT"], "["),
    (&["]", "BRACKETRIGHT"], "]"),
    (&["\\", "BACKSLASH"], "\\"),
    (&[";", "SEMICOLON"], ";"),
    (&["'", "QUOTE"], "'"),
    (&[",", "COMMA"], ","),
    (&[".", "PERIOD"], "."),
    (&["/", "SLASH"], "/"),
];

impl Hotkey {
    /// Standard text editing and system shortcuts that custom bindings must leave alone.
    pub fn is_system_reserved(&self) -> bool {
        let key = self.key.as_str();
        if self.alt && !self.ctrl && !self.meta && matches!(key, "Tab" | "F4" | "Escape" | "Space")
        {
            return true;
        }
        if self.ctrl && self.alt && key == "Delete" {
            return true;
        }
        if self.meta
            && cfg!(windows)
            && !self.ctrl
            && !self.alt
            && matches!(
                key,
                "D" | "E"
                    | "I"
                    | "L"
                    | "R"
                    | "S"
                    | "Tab"
                    | "Space"
                    | "Left"
                    | "Right"
                    | "Up"
                    | "Down"
            )
        {
            return true;
        }
        let editing_modifier = if cfg!(target_os = "macos") {
            self.meta || self.ctrl
        } else {
            self.ctrl
        };
        editing_modifier
            && !self.alt
            && (!self.meta || cfg!(target_os = "macos"))
            && (matches!(key, "Tab" | "Backspace" | "Delete")
                || (!self.shift
                    && matches!(
                        key,
                        "A" | "C"
                            | "V"
                            | "X"
                            | "Y"
                            | "Z"
                            | "S"
                            | "F"
                            | "N"
                            | "O"
                            | "P"
                            | "W"
                            | "T"
                            | "R"
                            | "Q"
                            | "L"
                    ))
                || (self.shift && key == "Z"))
    }

    /// Reads `Modifier+…+Key`, ignoring case and spaces around `+`. A shortcut needs a
    /// modifier unless its key is a function key, so plain typing is never captured.
    pub fn parse(text: &str) -> Option<Self> {
        let mut hotkey = Self {
            ctrl: false,
            shift: false,
            alt: false,
            meta: false,
            key: String::new(),
        };
        let tokens: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key, modifiers) = tokens.split_last()?;
        for token in modifiers {
            let flag = match token.to_uppercase().as_str() {
                "CTRL" | "CONTROL" => &mut hotkey.ctrl,
                "SHIFT" => &mut hotkey.shift,
                "ALT" | "OPTION" => &mut hotkey.alt,
                "WIN" | "WINDOWS" | "SUPER" | "META" | "CMD" | "COMMAND" => &mut hotkey.meta,
                _ => return None,
            };
            if *flag {
                return None;
            }
            *flag = true;
        }
        hotkey.key = key_name(key)?;
        let function = hotkey.key.starts_with('F') && hotkey.key.len() > 1;
        (function || hotkey.ctrl || hotkey.shift || hotkey.alt || hotkey.meta).then_some(hotkey)
    }

    /// The spelling the host registers with `global-hotkey`.
    pub fn registration(&self) -> String {
        let mut parts = self.modifiers(["Ctrl", "Shift", "Alt", "Super"]);
        parts.push(match self.key.as_str() {
            "Up" | "Down" | "Left" | "Right" => format!("Arrow{}", self.key),
            key => key.into(),
        });
        parts.join("+")
    }

    fn modifiers(&self, names: [&str; 4]) -> Vec<String> {
        [self.ctrl, self.shift, self.alt, self.meta]
            .into_iter()
            .zip(names)
            .filter(|(on, _)| *on)
            .map(|(_, name)| name.to_owned())
            .collect()
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let meta = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Win"
        };
        let mut parts = self.modifiers(["Ctrl", "Shift", "Alt", meta]);
        parts.push(self.key.clone());
        formatter.write_str(&parts.join("+"))
    }
}

fn key_name(token: &str) -> Option<String> {
    let upper = token.to_uppercase();
    let mut chars = upper.chars();
    if let (Some(single), None) = (chars.next(), chars.clone().next())
        && single.is_ascii_alphanumeric()
    {
        return Some(single.to_string());
    }
    if let Some(number) = upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
        && (1..=20).contains(&number)
    {
        return Some(format!("F{number}"));
    }
    if let Some(digit) = upper
        .strip_prefix("NUMPAD")
        .or_else(|| upper.strip_prefix("NUM"))
        .filter(|digit| digit.len() == 1 && digit.chars().all(|c| c.is_ascii_digit()))
    {
        return Some(format!("Numpad{digit}"));
    }
    NAMED
        .iter()
        .find(|(names, _)| names.contains(&upper.as_str()))
        .map(|(_, name)| (*name).to_owned())
}

/// The canonical spelling of a shortcut, `Some("")` for an explicitly empty one (no
/// shortcut), or `None` when the text is not a shortcut.
pub fn normalize(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return Some(String::new());
    }
    Hotkey::parse(text).map(|hotkey| hotkey.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_spellings_into_one_canonical_form() {
        let meta = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Win"
        };
        assert_eq!(
            normalize("alt + shift+ctrl+v").as_deref(),
            Some("Ctrl+Shift+Alt+V")
        );
        assert_eq!(normalize("Control+Space").as_deref(), Some("Ctrl+Space"));
        assert_eq!(
            normalize("super+num5").as_deref(),
            Some(format!("{meta}+Numpad5").as_str())
        );
        assert_eq!(normalize("f13").as_deref(), Some("F13"));
        assert_eq!(normalize("  ").as_deref(), Some(""));
    }

    #[test]
    fn rejects_text_that_is_not_a_shortcut() {
        for text in [
            "V",
            "Ctrl+",
            "Ctrl+Ctrl+V",
            "Ctrl+V+Shift",
            "Hyper+V",
            "Ctrl+F21",
            "Ctrl+VV",
        ] {
            assert_eq!(normalize(text), None, "{text}");
        }
    }

    #[test]
    fn registration_uses_the_hotkey_library_names() {
        let hotkey = Hotkey::parse("Win+Alt+Up").unwrap();
        assert_eq!(hotkey.registration(), "Alt+Super+ArrowUp");
        assert_eq!(
            Hotkey::parse("Ctrl+Shift+Alt+V").unwrap().registration(),
            "Ctrl+Shift+Alt+V"
        );
    }
}
