//! The user's overrides for installed plugins: whether each one is enabled, which
//! aliases it answers to and its global shortcuts. The host applies them; the settings
//! plugin edits them.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

/// The host passes the settings file to every plugin process in this variable.
pub const ENV: &str = "MYTOOLS_PLUGIN_SETTINGS";
/// The plugin that edits these settings is always enabled, so it can never be locked out.
pub const SETTINGS_PLUGIN: &str = "settings";
const MAX_ALIAS_LEN: usize = 64;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Settings {
    #[serde(default)]
    pub plugins: BTreeMap<String, Override>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Override {
    #[serde(default = "enabled", skip_serializing_if = "is_enabled")]
    pub enabled: bool,
    /// Replaces the manifest's aliases when present; an empty list removes them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<Vec<String>>,
    /// Replaces the plugin's own startup policy when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup: Option<Startup>,
    /// Replaces the manifest's `hotkey` (which opens the launcher in this plugin's scope)
    /// when present; `""` means no shortcut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<String>,
    /// Overrides for the plugin's manifest `commands`, by command id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub commands: BTreeMap<String, CommandOverride>,
}
impl Default for Override {
    fn default() -> Self {
        Self {
            enabled: true,
            alias: None,
            startup: None,
            hotkey: None,
            commands: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CommandOverride {
    /// Replaces the command's manifest `hotkey` when present; `""` means no shortcut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<String>,
}

/// When the host starts a search plugin's process.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Startup {
    /// When MyTools starts, so it is ready (and can work in the background) at once.
    Eager,
    /// On the first search or action that needs it.
    Lazy,
}

impl Startup {
    /// A plugin's own policy: its manifest's `startup`, else eager only for plugins that
    /// take part in global search.
    pub fn default_for(manifest: Option<Startup>, global: bool) -> Self {
        manifest.unwrap_or(if global { Self::Eager } else { Self::Lazy })
    }
}
fn enabled() -> bool {
    true
}
fn is_enabled(value: &bool) -> bool {
    *value
}

impl Settings {
    pub fn enabled(&self, id: &str) -> bool {
        id == SETTINGS_PLUGIN || self.plugins.get(id).is_none_or(|item| item.enabled)
    }

    /// The startup policy in effect: the user's choice when set, otherwise `default`.
    pub fn startup(&self, id: &str, default: Startup) -> Startup {
        self.plugins
            .get(id)
            .and_then(|item| item.startup)
            .unwrap_or(default)
    }

    /// The aliases in effect: the user's list when set, otherwise the manifest's.
    pub fn aliases<'a>(&'a self, id: &str, manifest: &'a [String]) -> &'a [String] {
        self.plugins
            .get(id)
            .and_then(|item| item.alias.as_deref())
            .unwrap_or(manifest)
    }

    /// The shortcut that opens a plugin's scope: the user's when set, otherwise the
    /// manifest's. `None` (also for `""`) means no shortcut.
    pub fn hotkey<'a>(&'a self, id: &str, manifest: Option<&'a str>) -> Option<&'a str> {
        self.plugins
            .get(id)
            .and_then(|item| item.hotkey.as_deref())
            .or(manifest)
            .filter(|hotkey| !hotkey.trim().is_empty())
    }

    /// A command's shortcut, like [`Settings::hotkey`].
    pub fn command_hotkey<'a>(
        &'a self,
        id: &str,
        command: &str,
        manifest: Option<&'a str>,
    ) -> Option<&'a str> {
        self.plugins
            .get(id)
            .and_then(|item| item.commands.get(command))
            .and_then(|item| item.hotkey.as_deref())
            .or(manifest)
            .filter(|hotkey| !hotkey.trim().is_empty())
    }
}

/// An alias is one word: it routes queries like `alias text`.
pub fn valid_alias(alias: &str) -> bool {
    !alias.is_empty() && alias.len() <= MAX_ALIAS_LEN && !alias.chars().any(char::is_whitespace)
}

/// The settings file the host told this process about, else `plugin-settings.json` in the
/// data folder ([`crate::dirs`]).
pub fn path() -> PathBuf {
    std::env::var_os(ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::dirs::root().join("plugin-settings.json"))
}

/// A missing file means no overrides.
pub fn load(path: &Path) -> io::Result<Settings> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(error) => Err(error),
    }
}

/// Writes through a temporary file so the host never reads a half-written file.
pub fn save(path: &Path, settings: &Settings) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(settings).map_err(io::Error::other)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overrides_apply_and_round_trip_without_defaults() {
        let mut settings = Settings::default();
        let manifest = vec!["calc".to_string()];
        assert!(settings.enabled("calculator"));
        assert_eq!(
            settings.aliases("calculator", &manifest),
            manifest.as_slice()
        );
        settings.plugins.insert(
            "calculator".into(),
            Override {
                enabled: false,
                alias: Some(vec!["c".into()]),
                startup: Some(Startup::Eager),
                ..Default::default()
            },
        );
        settings.plugins.insert(
            SETTINGS_PLUGIN.into(),
            Override {
                enabled: false,
                ..Default::default()
            },
        );
        assert!(!settings.enabled("calculator"));
        assert!(settings.enabled(SETTINGS_PLUGIN));
        assert_eq!(settings.aliases("calculator", &manifest), ["c".to_string()]);
        assert_eq!(
            settings.startup("calculator", Startup::Lazy),
            Startup::Eager
        );
        assert_eq!(settings.startup("translator", Startup::Lazy), Startup::Lazy);
        assert_eq!(Startup::default_for(None, true), Startup::Eager);
        assert_eq!(Startup::default_for(None, false), Startup::Lazy);
        assert_eq!(
            Startup::default_for(Some(Startup::Eager), false),
            Startup::Eager
        );
        let path =
            std::env::temp_dir().join(format!("plugin-settings-{}.json", std::process::id()));
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        settings
            .plugins
            .insert("translator".into(), Override::default());
        save(&path, &settings).unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("\"enabled\": true")
        );
        std::fs::remove_file(&path).unwrap();
        assert_eq!(load(&path).unwrap(), Settings::default());
        assert!(valid_alias("设置") && !valid_alias("a b") && !valid_alias(""));
    }

    #[test]
    fn hotkey_overrides_replace_or_remove_manifest_shortcuts() {
        let settings: Settings = serde_json::from_str(
            r#"{"plugins":{
                "clipboard":{"hotkey":"Ctrl+Alt+V","commands":{"paste-plain":{"hotkey":""}}},
                "translator":{"hotkey":""}
            }}"#,
        )
        .unwrap();
        assert_eq!(
            settings.hotkey("clipboard", Some("Ctrl+Shift+V")),
            Some("Ctrl+Alt+V")
        );
        assert_eq!(settings.hotkey("translator", Some("Alt+V")), None);
        assert_eq!(settings.hotkey("calculator", Some("Alt+C")), Some("Alt+C"));
        assert_eq!(settings.hotkey("calculator", None), None);
        assert_eq!(
            settings.command_hotkey("clipboard", "paste-plain", Some("Ctrl+Shift+Alt+V")),
            None
        );
        assert_eq!(
            settings.command_hotkey("clipboard", "other", Some("Ctrl+F1")),
            Some("Ctrl+F1")
        );
        // Empty command overrides are not written.
        let json = serde_json::to_string(&Override::default()).unwrap();
        assert_eq!(json, "{}");
    }
}
