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
//! Running one sends the plugin the action `{"command":"<id>"}` without an `item`; with
//! `"input": "selection"` the host first reads the text selected in the application in front
//! and sends it as the `item` (see [`CommandInput::Selection`]).
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
    /// What the host passes as the action's `item`; see [`CommandInput`].
    #[serde(default)]
    pub input: CommandInput,
}

/// What a command works on.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommandInput {
    /// Nothing: the action has no `item`.
    #[default]
    None,
    /// The text selected in the application in front, as the action's `item`. The plugin
    /// returns the replacement as `effect.copy_text` with `paste: true`: the host replaces
    /// the selection when it is editable, and otherwise opens the plugin's scope with the
    /// selected text (or copies the result when the plugin has no alias).
    Selection,
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
        assert_eq!(command.input, CommandInput::None);
        let selection: CommandManifest =
            serde_json::from_str(r#"{"id":"t","title":{"defaultValue":"T"},"input":"selection"}"#)
                .unwrap();
        assert_eq!(selection.input, CommandInput::Selection);
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
