//! The user's overrides for installed plugins: whether each one is enabled and which
//! aliases it answers to. The host applies them; the settings plugin edits them.
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
}
impl Default for Override {
    fn default() -> Self {
        Self {
            enabled: true,
            alias: None,
        }
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

    /// The aliases in effect: the user's list when set, otherwise the manifest's.
    pub fn aliases<'a>(&'a self, id: &str, manifest: &'a [String]) -> &'a [String] {
        self.plugins
            .get(id)
            .and_then(|item| item.alias.as_deref())
            .unwrap_or(manifest)
    }
}

/// An alias is one word: it routes queries like `alias text`.
pub fn valid_alias(alias: &str) -> bool {
    !alias.is_empty() && alias.len() <= MAX_ALIAS_LEN && !alias.chars().any(char::is_whitespace)
}

/// The settings file the host told this process about.
pub fn path() -> Option<PathBuf> {
    std::env::var_os(ENV).map(PathBuf::from)
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
            },
        );
        settings.plugins.insert(
            SETTINGS_PLUGIN.into(),
            Override {
                enabled: false,
                alias: None,
            },
        );
        assert!(!settings.enabled("calculator"));
        assert!(settings.enabled(SETTINGS_PLUGIN));
        assert_eq!(settings.aliases("calculator", &manifest), ["c".to_string()]);
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
}
