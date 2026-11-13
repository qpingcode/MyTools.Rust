use crate::i18n;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Ps1,
    Cmd,
}
impl Kind {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Ps1 => "ps1",
            Self::Cmd => "cmd",
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
    pub fn update(&self, script: Script, original: Option<&Script>) -> Result<()> {
        script.validate()?;
        self.mutate(|scripts| {
            let existing = scripts.iter().find(|s| s.id == script.id);
            ensure!(existing == original, i18n::t("store.conflict"));
            ensure!(
                !scripts.iter().any(|s| s.id != script.id
                    && !script.alias.is_empty()
                    && mytools_plugin_api::matching::normalize(&s.alias)
                        == mytools_plugin_api::matching::normalize(&script.alias)),
                i18n::t("validation.duplicate")
            );
            if let Some(current) = scripts.iter_mut().find(|s| s.id == script.id) {
                *current = script;
            } else {
                scripts.push(script);
            }
            Ok(())
        })
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
        }
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
}
