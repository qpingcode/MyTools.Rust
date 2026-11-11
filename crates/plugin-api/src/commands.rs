//! Commands a plugin declares in `plugin.json`: entries the host can search and run (and
//! bind to a global shortcut) without asking the plugin first.
//!
//! ```json
//! "commands": [{
//!   "id": "paste-plain",
//!   "title": { "key": "command.paste_plain", "defaultValue": "Paste as Plain Text" },
//!   "keywords": ["plain", "纯文本"],
//!   "icon": "builtin:type",
//!   "hotkey": "Ctrl+Shift+Alt+V"
//! }]
//! ```
//!
//! Running one sends the plugin the action `{"command":"<id>"}` without an `item`.
use serde::Deserialize;
use std::path::Path;

/// A display text with an optional key into the plugin's own `locales/<language>.json`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct LocalizedText {
    #[serde(default)]
    pub key: Option<String>,
    #[serde(rename = "defaultValue")]
    pub default_value: String,
}

impl LocalizedText {
    /// The text in `language`, falling back to English and then `defaultValue`.
    pub fn resolve(&self, plugin_directory: &Path, language: &str) -> String {
        match &self.key {
            Some(key) => {
                crate::i18n::plugin_text(plugin_directory, key, &self.default_value, language)
                    .unwrap_or_else(|_| self.default_value.clone())
            }
            None => self.default_value.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandManifest {
    /// Sent back as the action's `command`; unique within the plugin.
    pub id: String,
    pub title: LocalizedText,
    /// Extra words the command is found by, besides its title.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// A `builtin:` icon; omitted means the plugin's icon.
    #[serde(default)]
    pub icon: Option<String>,
    /// The default global shortcut; the user may change it in the settings.
    #[serde(default)]
    pub hotkey: Option<String>,
}

/// A command id is one word, so it can name the command in `plugin-settings.json`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_commands_and_rejects_unknown_fields() {
        let command: CommandManifest = serde_json::from_str(
            r#"{"id":"paste-plain","title":{"defaultValue":"Paste"},"hotkey":"Ctrl+Shift+Alt+V"}"#,
        )
        .unwrap();
        assert_eq!(command.hotkey.as_deref(), Some("Ctrl+Shift+Alt+V"));
        assert!(command.keywords.is_empty());
        assert!(
            serde_json::from_str::<CommandManifest>(
                r#"{"id":"x","title":{"defaultValue":"X"},"shortcut":"Ctrl+X"}"#
            )
            .is_err()
        );
        assert!(valid_id("paste-plain") && !valid_id("a.b") && !valid_id(""));
    }
}
