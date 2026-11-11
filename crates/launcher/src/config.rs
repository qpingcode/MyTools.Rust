use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub language: String,
    /// Old home of plugin shortcuts, by plugin id; moved into `plugin-settings.json` at
    /// startup (see [`migrate_plugin_shortcuts`]).
    pub plugin_shortcuts: std::collections::HashMap<String, String>,
    /// Optional overrides for `ui/theme.slint` (colors, font sizes, metrics).
    pub theme: crate::theme::Overrides,
    /// How long a plugin may take to start before it is paused; plugins can override it
    /// with `startupTimeoutMs` in their `plugin.json`.
    pub plugin_startup_timeout_ms: Option<u64>,
    /// The least severe log records kept: `debug`, `info` (default), `warn` or `error`.
    pub log_level: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "auto".into(),
            plugin_shortcuts: Default::default(),
            theme: Default::default(),
            plugin_startup_timeout_ms: None,
            log_level: None,
        }
    }
}

impl Config {
    pub fn load_options(&self) -> mytools_core::LoadOptions {
        let mut options = mytools_core::LoadOptions::default();
        if let Some(ms) = self.plugin_startup_timeout_ms {
            options.startup_timeout =
                mytools_core::startup_timeout(Some(ms), options.startup_timeout);
        }
        options
    }
}

pub fn path() -> anyhow::Result<PathBuf> {
    Ok(std::env::var_os("MYTOOLS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or(default_path()?))
}

fn default_path() -> anyhow::Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| anyhow::anyhow!("Missing user home directory"))?;
        Ok(PathBuf::from(home).join("Library/Application Support/MyTools/config.json"))
    }
    #[cfg(not(target_os = "macos"))]
    Ok(std::env::current_exe()?.with_file_name("config.json"))
}

pub fn load() -> anyhow::Result<Config> {
    match std::fs::read(path()?) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn save_language(language: &str) -> anyhow::Result<()> {
    save_language_at(&path()?, language)
}

fn save_language_at(path: &std::path::Path, language: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        matches!(language, "auto" | "zh-CN" | "en"),
        "Unsupported language"
    );
    update_at(path, |object| {
        object.insert("language".into(), language.into());
    })
}

/// Moves `plugin_shortcuts` into the plugin settings, where the settings window edits them,
/// then removes the field so it cannot later undo a change made there.
pub fn migrate_plugin_shortcuts(config: &Config) {
    use mytools_plugin_api::log;
    if config.plugin_shortcuts.is_empty() {
        return;
    }
    let result = mytools_core::migrate_legacy_hotkeys(&config.plugin_shortcuts).and_then(|moved| {
        update_at(&path()?, |object| {
            object.remove("plugin_shortcuts");
        })?;
        Ok(moved)
    });
    match result {
        Ok(moved) => log::info(
            "config.migrated",
            format!(
                "moved plugin_shortcuts to plugin-settings.json (changed: {})",
                moved.join(", ")
            ),
        ),
        Err(error) => log::warn(
            "config.migrate_failed",
            format!("could not move plugin_shortcuts to plugin-settings.json: {error:#}"),
        ),
    }
}

/// Changes the configuration file in place, keeping every field it does not touch.
fn update_at(
    path: &std::path::Path,
    change: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>),
) -> anyhow::Result<()> {
    let mut value: serde_json::Value = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error.into()),
    };
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Configuration must be a JSON object"))?;
    change(object);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn language_is_optional_but_must_be_a_string() {
        assert_eq!(
            serde_json::from_str::<Config>("{}").unwrap().language,
            "auto"
        );
        assert_eq!(
            serde_json::from_str::<Config>(r#"{"language":"en"}"#)
                .unwrap()
                .language,
            "en"
        );
        assert!(serde_json::from_str::<Config>(r#"{"language":123}"#).is_err());
    }

    #[test]
    fn plugin_startup_timeout_is_optional_and_clamped() {
        use std::time::Duration;
        let default = serde_json::from_str::<Config>("{}").unwrap().load_options();
        assert_eq!(default.startup_timeout, Duration::from_secs(10));
        let config: Config =
            serde_json::from_str(r#"{"plugin_startup_timeout_ms":500000}"#).unwrap();
        assert_eq!(
            config.load_options().startup_timeout,
            Duration::from_secs(120)
        );
    }

    #[test]
    fn saves_language_preserving_other_settings_and_rejecting_invalid_config() {
        let path =
            std::env::temp_dir().join(format!("mytools-settings-{}.json", std::process::id()));
        std::fs::write(&path, r#"{"language":"auto","other":{"enabled":true}}"#).unwrap();
        save_language_at(&path, "en").unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["language"], "en");
        assert_eq!(value["other"]["enabled"], true);
        assert!(save_language_at(&path, "invalid").is_err());
        std::fs::write(&path, b"invalid").unwrap();
        assert!(save_language_at(&path, "zh-CN").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        std::fs::remove_file(path).unwrap();
    }
}
