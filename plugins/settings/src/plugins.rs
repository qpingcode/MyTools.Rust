//! Installed plugins as the settings window edits them: manifest defaults merged with the
//! user's overrides in `plugin-settings.json`.
use crate::{hotkeys, i18n};
use anyhow::{Context, Result, ensure};
use mytools_plugin_api::plugin_settings::{self, Override, SETTINGS_PLUGIN, Settings, Startup};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Manifest {
    id: String,
    name: Name,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    alias: Vec<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    startup: Option<Startup>,
    #[serde(default)]
    search: Search,
    #[serde(default)]
    hotkey: Option<String>,
    #[serde(default)]
    commands: Vec<mytools_plugin_api::commands::CommandManifest>,
}
#[derive(Default, Deserialize)]
struct Search {
    #[serde(default)]
    global: bool,
}
#[derive(Deserialize)]
struct Name {
    #[serde(default)]
    key: Option<String>,
    #[serde(rename = "defaultValue")]
    default_value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    /// A `builtin:` icon name; other icon kinds fall back to a generic one.
    pub icon: String,
    pub default_aliases: Vec<String>,
    pub aliases: Vec<String>,
    pub enabled: bool,
    /// The plugin's own startup policy; `None` for plugins without a resident process.
    pub default_startup: Option<Startup>,
    pub startup: Option<Startup>,
    /// Shortcuts of a search plugin: its scope's first, then its commands'.
    pub hotkeys: Vec<hotkeys::Hotkey>,
}
impl Entry {
    /// The settings plugin cannot be disabled, or nothing could turn it back on.
    pub fn locked(&self) -> bool {
        self.id == SETTINGS_PLUGIN
    }
}

/// Where plugins and their settings live, as the host told this process.
pub struct Paths {
    pub plugins: PathBuf,
    pub settings: PathBuf,
}
impl Paths {
    pub fn current() -> Result<Self> {
        let plugins = match std::env::var_os("MYTOOLS_PLUGIN_DIR") {
            Some(directory) => PathBuf::from(directory),
            // plugins/settings/mytools-settings.exe
            None => std::env::current_exe()?
                .parent()
                .and_then(Path::parent)
                .context(i18n::t("error.plugins"))?
                .to_path_buf(),
        };
        let settings = plugin_settings::path();
        Ok(Self { plugins, settings })
    }
}

/// Every readable plugin, sorted by name; unreadable manifests are the host's to report.
pub fn list(paths: &Paths) -> Result<Vec<Entry>> {
    let settings = plugin_settings::load(&paths.settings)?;
    let language = mytools_plugin_api::i18n::language();
    let mut entries = Vec::new();
    for directory in std::fs::read_dir(&paths.plugins)?.flatten() {
        let directory = directory.path();
        let Ok(bytes) = std::fs::read(directory.join("plugin.json")) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
            continue;
        };
        let name = match &manifest.name.key {
            Some(key) => mytools_plugin_api::i18n::plugin_text(
                &directory,
                key,
                &manifest.name.default_value,
                language,
            )
            .unwrap_or(manifest.name.default_value),
            None => manifest.name.default_value,
        };
        let search = manifest.kind.as_deref() == Some("search");
        let default_startup =
            search.then(|| Startup::default_for(manifest.startup, manifest.search.global));
        // Only resident search plugins can be opened or run by a shortcut.
        let hotkeys = if search {
            hotkeys::list(
                &manifest.id,
                manifest.hotkey.as_deref(),
                &manifest.commands,
                &settings,
                |command| command.title.resolve(&directory, language),
            )
        } else {
            Vec::new()
        };
        entries.push(Entry {
            hotkeys,
            startup: default_startup.map(|default| settings.startup(&manifest.id, default)),
            default_startup,
            name,
            icon: manifest
                .icon
                .filter(|icon| icon.starts_with("builtin:"))
                .unwrap_or_else(|| "builtin:puzzle".into()),
            aliases: settings.aliases(&manifest.id, &manifest.alias).to_vec(),
            enabled: settings.enabled(&manifest.id),
            default_aliases: manifest.alias,
            id: manifest.id,
        });
    }
    entries.sort_by_key(|entry| (entry.name.to_lowercase(), entry.id.clone()));
    Ok(entries)
}

/// Splits what the user typed into aliases; commas, Chinese commas and spaces separate them.
pub fn parse_aliases(text: &str) -> Vec<String> {
    let mut aliases: Vec<String> = Vec::new();
    for alias in text
        .split([',', '，', ' ', '\t'])
        .filter(|alias| !alias.is_empty())
    {
        if !aliases
            .iter()
            .any(|known| known.eq_ignore_ascii_case(alias))
        {
            aliases.push(alias.to_owned());
        }
    }
    aliases
}

/// Checks the edited list and writes only what differs from each manifest, keeping
/// overrides for plugins that are not installed right now.
pub fn save(paths: &Paths, entries: &[Entry]) -> Result<()> {
    validate(entries)?;
    let mut settings: Settings = plugin_settings::load(&paths.settings)?;
    for entry in entries {
        let mut item = Override {
            enabled: entry.enabled || entry.locked(),
            alias: (entry.aliases != entry.default_aliases).then(|| entry.aliases.clone()),
            startup: entry
                .startup
                .filter(|startup| Some(*startup) != entry.default_startup),
            ..Default::default()
        };
        hotkeys::apply(&entry.hotkeys, &mut item);
        if item == Override::default() {
            settings.plugins.remove(&entry.id);
        } else {
            settings.plugins.insert(entry.id.clone(), item);
        }
    }
    plugin_settings::save(&paths.settings, &settings)?;
    Ok(())
}

fn validate(entries: &[Entry]) -> Result<()> {
    hotkeys::validate(entries.iter().map(|entry| {
        (
            entry.name.as_str(),
            entry.enabled || entry.locked(),
            entry.hotkeys.as_slice(),
        )
    }))?;
    let mut seen: Vec<(String, &str)> = Vec::new();
    for entry in entries {
        for alias in &entry.aliases {
            ensure!(
                plugin_settings::valid_alias(alias),
                i18n::format("error.alias", &[("name", &entry.name), ("alias", alias)])
            );
            if !entry.enabled {
                continue;
            }
            let key = alias.to_lowercase();
            if let Some((_, other)) = seen.iter().find(|(known, _)| *known == key) {
                anyhow::bail!(i18n::format(
                    "error.duplicate",
                    &[("alias", alias), ("first", other), ("second", &entry.name)]
                ));
            }
            seen.push((key, &entry.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, id: &str, alias: &[&str]) {
        let directory = root.join(id);
        std::fs::create_dir_all(directory.join("locales")).unwrap();
        let manifest = serde_json::json!({
            "id": id, "name": {"key": "name", "defaultValue": id},
            "icon": "builtin:calculator", "alias": alias,
            "type": "search", "search": {"global": id != "translator"},
        });
        std::fs::write(directory.join("plugin.json"), manifest.to_string()).unwrap();
        std::fs::write(
            directory.join("locales/en.json"),
            format!(r#"{{"name":"{id} plugin"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn lists_merges_validates_and_saves_only_changes() {
        let root = std::env::temp_dir().join(format!("settings-plugins-{}", std::process::id()));
        let paths = Paths {
            plugins: root.join("plugins"),
            settings: root.join("plugin-settings.json"),
        };
        write(&paths.plugins, "calculator", &["calc"]);
        write(&paths.plugins, "settings", &["settings"]);
        write(&paths.plugins, "translator", &["tr"]);
        let mut entries = list(&paths).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "calculator plugin");
        assert!(entries.iter().all(|entry| entry.enabled));
        assert_eq!(entries[0].startup, Some(Startup::Eager));
        assert_eq!(entries[2].startup, Some(Startup::Lazy));

        // Only a policy that differs from the plugin's own is saved.
        entries[2].startup = Some(Startup::Eager);
        save(&paths, &entries).unwrap();
        let saved = plugin_settings::load(&paths.settings).unwrap();
        assert_eq!(saved.plugins["translator"].startup, Some(Startup::Eager));
        entries[2].startup = Some(Startup::Lazy);
        save(&paths, &entries).unwrap();
        assert!(
            plugin_settings::load(&paths.settings)
                .unwrap()
                .plugins
                .is_empty()
        );

        entries[0].aliases = parse_aliases("c， calc,c ");
        entries[2].enabled = false;
        save(&paths, &entries).unwrap();
        let saved = plugin_settings::load(&paths.settings).unwrap();
        assert_eq!(saved.plugins.len(), 2);
        assert_eq!(saved.aliases("calculator", &[]), ["c", "calc"]);
        assert!(!saved.enabled("translator"));
        assert_eq!(list(&paths).unwrap(), entries);

        // A disabled plugin's alias may repeat; an enabled one may not.
        entries[2].aliases = vec!["c".into()];
        save(&paths, &entries).unwrap();
        entries[2].enabled = true;
        assert!(save(&paths, &entries).is_err());
        entries[2].aliases = vec!["a b".into()];
        assert!(save(&paths, &entries).is_err());

        // Shortcuts: the scope's first, then each command's; only changes are saved.
        let clipboard = paths.plugins.join("clipboard");
        std::fs::create_dir_all(&clipboard).unwrap();
        let manifest = serde_json::json!({
            "id": "clipboard", "name": {"defaultValue": "Clipboard"}, "alias": ["cb"],
            "type": "search", "hotkey": "Ctrl+Shift+V",
            "commands": [{"id": "paste-plain", "title": {"defaultValue": "Paste plain"},
                          "hotkey": "Ctrl+Shift+Alt+V"}],
        });
        std::fs::write(clipboard.join("plugin.json"), manifest.to_string()).unwrap();
        entries = list(&paths).unwrap();
        let index = entries.iter().position(|e| e.id == "clipboard").unwrap();
        let shortcuts = &entries[index].hotkeys;
        assert_eq!(shortcuts.len(), 2);
        assert_eq!(shortcuts[0].hotkey, "Ctrl+Shift+V");
        assert_eq!(shortcuts[1].label, "Paste plain");
        entries[index].hotkeys[1].hotkey = "alt+shift+ctrl+p".into();
        entries[index].hotkeys[0].hotkey = String::new();
        save(&paths, &entries).unwrap();
        let saved = plugin_settings::load(&paths.settings).unwrap();
        assert_eq!(saved.hotkey("clipboard", Some("Ctrl+Shift+V")), None);
        assert_eq!(
            saved.command_hotkey("clipboard", "paste-plain", None),
            Some("Ctrl+Shift+Alt+P")
        );
        assert_eq!(
            list(&paths).unwrap()[index].hotkeys[1].hotkey,
            "Ctrl+Shift+Alt+P"
        );
        entries[index].hotkeys[0].hotkey = "Ctrl+Shift+Alt+P".into();
        assert!(save(&paths, &entries).is_err());
        std::fs::remove_dir_all(&clipboard).unwrap();
        entries = list(&paths).unwrap();

        // The settings plugin stays enabled whatever the form says.
        entries[2] = list(&paths).unwrap()[2].clone();
        entries[1].enabled = false;
        save(&paths, &entries).unwrap();
        assert!(list(&paths).unwrap()[1].enabled);
        std::fs::remove_dir_all(root).unwrap();
    }
}
