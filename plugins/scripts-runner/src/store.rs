use crate::i18n;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    /// Windows PowerShell 5.1, which ships with the system.
    Ps1,
    Cmd,
    /// PowerShell 7.
    Pwsh,
}
impl Kind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Ps1 | Self::Pwsh => "ps1",
            Self::Cmd => "cmd",
        }
    }
    /// The stored name, also shown next to the script in search results.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ps1 => "ps1",
            Self::Cmd => "cmd",
            Self::Pwsh => "pwsh",
        }
    }
}

/// How a run is shown: a console that stays open, one that closes after success, or none.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Window,
    CloseOnSuccess,
    Background,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Script {
    pub id: String,
    pub name: String,
    pub alias: String,
    pub kind: Kind,
    pub content: String,
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub mode: Mode,
    /// When the script was created, in milliseconds since the Unix epoch; 0 for scripts
    /// saved before this was recorded, which keep their order in the store instead.
    #[serde(default)]
    pub created: u64,
}
impl Script {
    pub fn validate(&self) -> Result<()> {
        uuid::Uuid::parse_str(&self.id)?;
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 256,
            i18n::t("validation.name")
        );
        ensure!(
            self.alias.len() <= 128 && !self.alias.contains(['\n', '\r', '\0']),
            i18n::t("validation.alias")
        );
        ensure!(
            !self.content.trim().is_empty()
                && self.content.len() <= 64 * 1024
                && !self.content.contains('\0'),
            i18n::t("validation.content")
        );
        ensure!(
            self.directory.is_empty()
                || (std::path::Path::new(&self.directory).is_absolute()
                    && std::path::Path::new(&self.directory).is_dir()),
            i18n::t("validation.directory")
        );
        Ok(())
    }
}

/// Preferences of the manager window, kept apart from the scripts.
const SETTINGS: &str = "manager.json";
#[derive(Deserialize, Serialize)]
struct Settings {
    sort_by_name: bool,
}

pub struct Store {
    pub root: PathBuf,
}
impl Store {
    pub fn new() -> Result<Self> {
        let root = std::env::var_os("MYTOOLS_SCRIPTS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| mytools_plugin_api::dirs::root().join("scripts"));
        Ok(Self { root })
    }
    pub fn load(&self) -> Result<Vec<Script>> {
        let file = match fs::File::open(self.root.join("scripts.json")) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        let mut bytes = Vec::new();
        file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 8 * 1024 * 1024, i18n::t("store.limit"));
        let scripts: Vec<Script> = serde_json::from_slice(&bytes)?;
        ensure!(scripts.len() <= 1000, i18n::t("store.limit"));
        let mut ids = std::collections::HashSet::new();
        ensure!(
            scripts
                .iter()
                .all(|s| uuid::Uuid::parse_str(&s.id).is_ok() && ids.insert(s.id.clone())),
            i18n::t("store.invalid")
        );
        Ok(scripts)
    }
    #[cfg(test)]
    pub fn update(&self, script: Script, original: Option<&Script>) -> Result<()> {
        self.save_all(&[(script, original.cloned())])
    }
    /// Saves several edited scripts together, each paired with the version it was edited
    /// from (`None` for a new script): either all are written or none is.
    pub fn save_all(&self, changes: &[(Script, Option<Script>)]) -> Result<()> {
        for (script, _) in changes {
            script.validate()?;
        }
        self.mutate(|scripts| {
            for (script, original) in changes {
                let existing = scripts.iter().find(|s| s.id == script.id);
                ensure!(existing == original.as_ref(), i18n::t("store.conflict"));
                match scripts.iter_mut().find(|s| s.id == script.id) {
                    Some(current) => *current = script.clone(),
                    None => scripts.push(script.clone()),
                }
            }
            // Checked once everything is applied, so two scripts can swap aliases.
            for (script, _) in changes {
                let alias = mytools_plugin_api::matching::normalize(&script.alias);
                ensure!(
                    script.alias.is_empty()
                        || !scripts.iter().any(|s| s.id != script.id
                            && mytools_plugin_api::matching::normalize(&s.alias) == alias),
                    i18n::format("validation.duplicate", &[("name", &script.name)])
                );
            }
            Ok(())
        })
    }
    /// Whether the manager lists scripts by name (the default) or newest first.
    pub fn sort_by_name(&self) -> bool {
        fs::read(self.root.join(SETTINGS))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).ok())
            .is_none_or(|settings| settings.sort_by_name)
    }
    pub fn set_sort_by_name(&self, sort_by_name: bool) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::write(
            self.root.join(SETTINGS),
            serde_json::to_vec_pretty(&Settings { sort_by_name })?,
        )?;
        Ok(())
    }
    pub fn delete(&self, original: &Script) -> Result<()> {
        self.mutate(|scripts| {
            ensure!(
                scripts.iter().find(|s| s.id == original.id) == Some(original),
                i18n::t("store.conflict")
            );
            scripts.retain(|s| s.id != original.id);
            Ok(())
        })
    }
    fn mutate(&self, change: impl FnOnce(&mut Vec<Script>) -> Result<()>) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join("store.lock"))?;
        lock.lock()?;
        let mut scripts = self.load()?;
        change(&mut scripts)?;
        ensure!(scripts.len() <= 1000, i18n::t("store.limit"));
        let bytes = serde_json::to_vec_pretty(&scripts)?;
        ensure!(bytes.len() <= 8 * 1024 * 1024, i18n::t("store.limit"));
        let temp = self
            .root
            .join(format!("scripts.{}.tmp", std::process::id()));
        let mut file = fs::File::create(&temp)?;
        use std::io::Write;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        let result = fs::rename(&temp, self.root.join("scripts.json"));
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub fn sample() -> Script {
        Script {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Example".into(),
            alias: "dev".into(),
            kind: Kind::Ps1,
            content: "Write-Output '中文'\n# multiline".into(),
            directory: String::new(),
            mode: Mode::Background,
            created: 0,
        }
    }
    #[test]
    fn powershell_7_is_stored_as_pwsh_and_runs_a_ps1_file() {
        assert_eq!(serde_json::to_string(&Kind::Pwsh).unwrap(), r#""pwsh""#);
        assert_eq!(serde_json::from_str::<Kind>(r#""ps1""#).unwrap(), Kind::Ps1);
        assert_eq!(Kind::Pwsh.extension(), "ps1");
    }
    #[test]
    fn stable_identity_roundtrip_duplicate_alias_and_concurrent_edits() {
        let store = Store {
            root: std::env::temp_dir().join(format!("scripts-store-{}", uuid::Uuid::new_v4())),
        };
        let script = sample();
        store.update(script.clone(), None).unwrap();
        assert_eq!(store.load().unwrap(), vec![script.clone()]);
        let mut changed = script.clone();
        changed.name = "Renamed".into();
        store.update(changed.clone(), Some(&script)).unwrap();
        assert!(store.update(script.clone(), Some(&script)).is_err());
        let mut duplicate = sample();
        duplicate.alias = "DEV".into();
        assert!(store.update(duplicate, None).is_err());
        assert_eq!(store.load().unwrap()[0].id, script.id);
        assert!(store.delete(&script).is_err());
        store.delete(&changed).unwrap();
        assert!(store.load().unwrap().is_empty());
        fs::remove_dir_all(store.root).unwrap();
    }
    #[test]
    fn saves_several_scripts_together_or_none() {
        let store = Store {
            root: std::env::temp_dir().join(format!("scripts-store-{}", uuid::Uuid::new_v4())),
        };
        let (mut first, mut second) = (sample(), sample());
        second.alias = "other".into();
        store
            .save_all(&[(first.clone(), None), (second.clone(), None)])
            .unwrap();
        // Swapping aliases is allowed because the check runs after both are applied.
        let (old_first, old_second) = (first.clone(), second.clone());
        (first.alias, second.alias) = (second.alias.clone(), first.alias.clone());
        store
            .save_all(&[
                (first.clone(), Some(old_first)),
                (second.clone(), Some(old_second.clone())),
            ])
            .unwrap();
        // One invalid script keeps the other from being written.
        let mut renamed = second.clone();
        renamed.name = "Renamed".into();
        let mut invalid = sample();
        invalid.content = String::new();
        assert!(
            store
                .save_all(&[(renamed, Some(second.clone())), (invalid, None)])
                .is_err()
        );
        assert_eq!(store.load().unwrap(), vec![first, second]);
        assert!(store.sort_by_name());
        store.set_sort_by_name(false).unwrap();
        assert!(!store.sort_by_name());
        fs::remove_dir_all(store.root).unwrap();
    }
}
