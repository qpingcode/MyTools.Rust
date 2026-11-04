use crate::i18n;
use crate::search_process::SearchProcess;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Deserialize)]
struct PluginName {
    #[serde(default)]
    key: Option<String>,
    #[serde(rename = "defaultValue")]
    default_value: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    #[default]
    Window,
    Search,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    id: String,
    name: PluginName,
    version: String,
    #[serde(default, rename = "type")]
    kind: PluginType,
    entry: PathBuf,
    #[serde(default)]
    icon: Option<mytools_plugin_api::Icon>,
    #[serde(default)]
    alias: Vec<String>,
    #[serde(default)]
    search: Search,
    #[serde(default)]
    view: mytools_plugin_api::View,
    #[serde(default)]
    hotkey: Option<String>,
    #[serde(default)]
    contributes: Contributes,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Contributes {
    /// Extra actions this plugin adds to other results, e.g. "Open in Rider" on paths.
    #[serde(default)]
    actions: Vec<ActionRuleManifest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActionRuleManifest {
    #[serde(default)]
    when: When,
    actions: Vec<ContributedActionManifest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContributedActionManifest {
    command: String,
    title: PluginName,
    #[serde(default)]
    icon: Option<String>,
}

/// Result types a rule can match; `path` is shorthand for `file` plus `folder`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Url,
    File,
    Folder,
    Path,
    Text,
    Application,
    PluginItem,
}

impl ResourceKind {
    fn matches(self, resource: &mytools_plugin_api::Resource) -> bool {
        use mytools_plugin_api::Resource as R;
        matches!(
            (self, resource),
            (Self::Url, R::Url { .. })
                | (Self::File | Self::Path, R::File { .. })
                | (Self::Folder | Self::Path, R::Folder { .. })
                | (Self::Text, R::Text { .. })
                | (Self::Application, R::Application { .. })
                | (Self::PluginItem, R::PluginItem { .. })
        )
    }
}

/// Which results a rule applies to. Fields combine with AND, list entries with OR, and an
/// omitted or empty field matches everything.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    #[serde(default, rename = "type")]
    pub types: Vec<ResourceKind>,
    /// Ids of the plugins whose results the rule applies to.
    #[serde(default)]
    pub source: Vec<String>,
}

impl When {
    pub fn matches(&self, resource: &mytools_plugin_api::Resource, source: &str) -> bool {
        (self.types.is_empty() || self.types.iter().any(|kind| kind.matches(resource)))
            && (self.source.is_empty() || self.source.iter().any(|id| id == source))
    }
}

/// A plugin command added to matching results; the result's main value is sent as the item.
#[derive(Clone, Debug)]
pub struct ContributedAction {
    pub command: String,
    pub title: String,
    pub icon: String,
}

#[derive(Clone, Debug)]
pub struct ActionRule {
    pub when: When,
    pub actions: Vec<ContributedAction>,
}

#[derive(Debug, Default, Deserialize)]
struct Search {
    #[serde(default)]
    global: bool,
    /// How long a search through this plugin's alias may take; global searches always
    /// use the shared one-second limit.
    #[serde(default, rename = "aliasTimeoutMs")]
    alias_timeout_ms: Option<u64>,
}

/// Default and maximum wait for a search through a plugin's alias.
pub const ALIAS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
pub const MAX_ALIAS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

fn alias_timeout(ms: Option<u64>) -> std::time::Duration {
    ms.map(std::time::Duration::from_millis)
        .unwrap_or(ALIAS_TIMEOUT)
        .clamp(crate::search_process::SEARCH_TIMEOUT, MAX_ALIAS_TIMEOUT)
}

#[derive(Clone)]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub icon: mytools_plugin_api::Icon,
    pub aliases: Vec<String>,
    pub global: bool,
    pub alias_timeout: std::time::Duration,
    pub executable: PathBuf,
    pub process: Option<Arc<SearchProcess>>,
    pub view: mytools_plugin_api::View,
    pub action_rules: Vec<ActionRule>,
}

impl Plugin {
    pub fn load(path: &Path) -> Result<Self> {
        let manifest: Manifest =
            serde_json::from_slice(&fs::read(path)?).context(i18n::t("error.manifest_invalid"))?;
        let executable = manifest.validate(path)?;
        let language = mytools_plugin_api::i18n::language();
        let name = localized_name(&manifest.name, path, language)?;
        // Only resident search plugins can receive the commands.
        let action_rules = if manifest.kind == PluginType::Search {
            manifest
                .contributes
                .actions
                .iter()
                .map(|rule| {
                    let actions = rule
                        .actions
                        .iter()
                        .filter(|action| !action.command.trim().is_empty())
                        .map(|action| {
                            Ok(ContributedAction {
                                command: action.command.clone(),
                                title: localized_name(&action.title, path, language)?,
                                icon: action
                                    .icon
                                    .clone()
                                    .filter(|icon| icon.starts_with("builtin:"))
                                    .unwrap_or_else(|| "builtin:external-link".into()),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Ok(ActionRule {
                        when: rule.when.clone(),
                        actions,
                    })
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let process = if manifest.kind == PluginType::Search {
            Some(Arc::new(SearchProcess::start(&executable)?))
        } else {
            None
        };
        Ok(Self {
            id: manifest.id,
            name,
            version: manifest.version,
            icon: mytools_plugin_api::icons::resolve_reference(
                manifest.icon.as_ref(),
                &mytools_plugin_api::icons::DEFAULT_ICON.into(),
            ),
            aliases: manifest.alias,
            global: manifest.search.global,
            alias_timeout: alias_timeout(manifest.search.alias_timeout_ms),
            executable,
            process,
            view: manifest.view,
            action_rules,
        })
    }
}

pub fn directory() -> PathBuf {
    std::env::var_os("MYTOOLS_PLUGIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap_or_default()
                .with_file_name("plugins")
        })
}

pub struct ActivationShortcut {
    pub plugin_id: String,
    pub hotkey: String,
    pub query: String,
}

/// Read activation metadata without starting another plugin process.
pub fn activation_shortcuts() -> Result<Vec<ActivationShortcut>> {
    let mut shortcuts = Vec::new();
    for path in manifest_paths(&directory())? {
        // Catalog reports invalid manifests; other plugins keep their shortcuts.
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
            continue;
        };
        if manifest.kind == PluginType::Search
            && let (Some(hotkey), Some(alias)) = (manifest.hotkey, manifest.alias.first())
        {
            shortcuts.push(ActivationShortcut {
                plugin_id: manifest.id,
                hotkey,
                query: format!("{alias} "),
            });
        }
    }
    Ok(shortcuts)
}

fn localized_name(name: &PluginName, manifest: &Path, language: &str) -> Result<String> {
    let Some(key) = &name.key else {
        return Ok(name.default_value.clone());
    };
    let directory = manifest
        .parent()
        .context(i18n::t("error.manifest_directory"))?
        .join("locales");
    let read = |language: &str| -> Result<String> {
        match fs::read_to_string(directory.join(format!("{language}.json"))) {
            Ok(json) => Ok(json),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("{}".into()),
            Err(error) => Err(error.into()),
        }
    };
    let catalog = mytools_plugin_api::i18n::Catalog::from_json(&read(language)?, &read("en")?)?;
    Ok(catalog.lookup(key).unwrap_or(&name.default_value).into())
}

impl Manifest {
    fn validate(&self, path: &Path) -> Result<PathBuf> {
        if self.id.trim().is_empty() || self.name.default_value.trim().is_empty() {
            bail!(i18n::t("error.manifest_name"));
        }
        if self
            .alias
            .iter()
            .any(|alias| alias.is_empty() || alias.chars().any(char::is_whitespace))
        {
            bail!(i18n::t("error.manifest_alias"));
        }
        if self.entry.as_os_str().is_empty()
            || self
                .entry
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            bail!(i18n::t("error.manifest_entry"));
        }
        let directory = path
            .parent()
            .context(i18n::t("error.manifest_directory"))?
            .canonicalize()?;
        let mut entry = self.entry.clone();
        if cfg!(windows) && entry.extension().is_none() {
            entry.set_extension("exe");
        }
        let executable = directory.join(&entry).canonicalize().with_context(|| {
            i18n::format(
                "error.entry_missing",
                &[("path", &entry.display().to_string())],
            )
        })?;
        if !executable.starts_with(&directory) || !executable.is_file() {
            bail!(i18n::t("error.manifest_program"));
        }
        Ok(executable)
    }
}

pub fn manifest_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths: Vec<_> = fs::read_dir(root)?
        .flatten()
        .map(|dir| dir.path().join("plugin.json"))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_timeout_defaults_and_is_clamped() {
        use std::time::Duration;
        assert_eq!(alias_timeout(None), ALIAS_TIMEOUT);
        assert_eq!(alias_timeout(Some(5000)), Duration::from_secs(5));
        assert_eq!(alias_timeout(Some(10)), Duration::from_secs(1));
        assert_eq!(alias_timeout(Some(60_000)), MAX_ALIAS_TIMEOUT);
    }

    #[test]
    fn action_rules_match_by_type_and_source() {
        use mytools_plugin_api::Resource;
        let manifest: Manifest =
            serde_json::from_str(include_str!("../../../plugins/dev-open/plugin.json")).unwrap();
        let rule = &manifest.contributes.actions[0];
        assert_eq!(rule.actions.len(), 3);
        let file = Resource::File {
            path: "C:\\a.txt".into(),
        };
        let folder = Resource::Folder {
            path: "C:\\".into(),
        };
        let text = Resource::Text { text: "x".into() };
        assert!(rule.when.matches(&file, "everything"));
        assert!(rule.when.matches(&folder, "clipboard"));
        assert!(!rule.when.matches(&text, "everything"));

        let when: When =
            serde_json::from_str(r#"{"type":["plugin_item"],"source":["process-killer"]}"#)
                .unwrap();
        let item = Resource::PluginItem { id: "42".into() };
        assert!(when.matches(&item, "process-killer"));
        assert!(!when.matches(&item, "clipboard"));
        assert!(!when.matches(&file, "process-killer"));
        assert!(When::default().matches(&text, "anything"));
    }

    #[test]
    fn action_rules_reject_unknown_types_and_fields() {
        assert!(serde_json::from_str::<When>(r#"{"type":["paths"]}"#).is_err());
        assert!(serde_json::from_str::<When>(r#"{"from":["clipboard"]}"#).is_err());
    }

    #[test]
    fn reads_both_plugin_types_without_removed_fields() {
        for (json, kind) in [
            (
                r#"{"id":"window-fixture","name":{"defaultValue":"Window fixture"},"version":"0.1.0","type":"window","entry":"fixture"}"#,
                PluginType::Window,
            ),
            (
                include_str!("../../../plugins/calculator/plugin.json"),
                PluginType::Search,
            ),
            (
                include_str!("../../../plugins/applications/plugin.json"),
                PluginType::Search,
            ),
            (
                include_str!("../../../plugins/clipboard/plugin.json"),
                PluginType::Search,
            ),
        ] {
            let value: serde_json::Value = serde_json::from_str(json).unwrap();
            assert!(value.get("protocolVersion").is_none());
            assert!(value.get("capabilities").is_none());
            let manifest: Manifest = serde_json::from_value(value).unwrap();
            assert_eq!(manifest.kind, kind);
        }
    }

    #[test]
    fn validates_entry_and_alias() {
        let path = Path::new("plugin.json");
        let mut manifest: Manifest =
            serde_json::from_str(include_str!("../../../plugins/calculator/plugin.json")).unwrap();
        manifest.entry = "../outside.exe".into();
        assert!(
            manifest
                .validate(path)
                .unwrap_err()
                .to_string()
                .contains(&i18n::t("error.manifest_entry"))
        );
        manifest.alias = vec!["two words".into()];
        assert!(
            manifest
                .validate(path)
                .unwrap_err()
                .to_string()
                .contains(&i18n::t("error.manifest_alias"))
        );
    }

    #[test]
    fn resolves_portable_entry_on_the_current_platform() {
        let directory =
            std::env::temp_dir().join(format!("mytools-portable-entry-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let executable = directory.join(format!(
            "mytools-calculator{}",
            std::env::consts::EXE_SUFFIX
        ));
        fs::write(&executable, b"fixture").unwrap();
        let manifest: Manifest =
            serde_json::from_str(include_str!("../../../plugins/calculator/plugin.json")).unwrap();
        assert_eq!(
            manifest.validate(&directory.join("plugin.json")).unwrap(),
            executable.canonicalize().unwrap()
        );
        fs::remove_file(executable).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn plugin_name_uses_own_catalog_then_english_then_default() {
        let directory =
            std::env::temp_dir().join(format!("mytools-name-locale-{}", std::process::id()));
        let locales = directory.join("locales");
        fs::create_dir_all(&locales).unwrap();
        fs::write(locales.join("en.json"), r#"{"name":"Calculator"}"#).unwrap();
        fs::write(locales.join("zh-CN.json"), r#"{"name":"计算器"}"#).unwrap();
        let name = PluginName {
            key: Some("name".into()),
            default_value: "Default".into(),
        };
        let manifest = directory.join("plugin.json");
        assert_eq!(localized_name(&name, &manifest, "zh-CN").unwrap(), "计算器");
        assert_eq!(
            localized_name(&name, &manifest, "fr").unwrap(),
            "Calculator"
        );
        fs::write(locales.join("en.json"), "{}").unwrap();
        assert_eq!(localized_name(&name, &manifest, "fr").unwrap(), "Default");
        for file in ["en.json", "zh-CN.json"] {
            fs::remove_file(locales.join(file)).unwrap();
        }
        fs::remove_dir(locales).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
