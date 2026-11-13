use mytools_plugin_api::{Resource, matching::normalize};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const DAY: u64 = 86_400_000;
const REUSE_WINDOW: u64 = 14 * DAY;
const DECAY_WINDOW: f64 = (30 * DAY) as f64;
const LIMIT: usize = 4096;

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    pub query: String,
    pub key: String,
}

impl Selection {
    pub fn new(plugin: &str, resource: &Resource, query: &str) -> Self {
        let mut resource = resource.clone();
        if let Resource::Application { path }
        | Resource::File { path }
        | Resource::Folder { path } = &mut resource
        {
            let normalized = path.canonicalize().unwrap_or_else(|_| path.clone());
            let value = mytools_plugin_api::paths::display(&normalized);
            *path = if cfg!(windows) {
                value.to_lowercase().into()
            } else {
                value.into()
            };
        }
        // Explicit FNV-1a keeps keys stable across runs without storing copied text.
        let identity = serde_json::to_vec(&(plugin, resource)).expect("resource serialization");
        let hash = identity.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
        Self {
            query: normalize(query),
            key: format!("{plugin}:{hash:016x}"),
        }
    }

    pub fn record(&self) -> anyhow::Result<()> {
        let path =
            path().ok_or_else(|| anyhow::anyhow!(crate::i18n::t("error.history_directory")))?;
        record_at(&path, self, now())
    }
}

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct History {
    records: Vec<Record>,
}

#[derive(Deserialize, Serialize)]
struct Record {
    query: String,
    key: String,
    last: u64,
    uses: f64,
}

impl History {
    pub fn load() -> Self {
        path()
            .and_then(|path| Self::read(&path).ok())
            .unwrap_or_default()
    }

    fn read(path: &Path) -> anyhow::Result<Self> {
        let file = match fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 8 * 1024 * 1024, "history size limit");
        let history: Self = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            history.records.len() <= LIMIT
                && history
                    .records
                    .iter()
                    .all(|r| r.uses.is_finite() && r.uses >= 0.0),
            "invalid history"
        );
        Ok(history)
    }

    pub fn reused<'a>(
        &self,
        query: &str,
        keys: impl Iterator<Item = &'a str>,
        time: u64,
    ) -> Option<String> {
        let query = normalize(query);
        let keys: std::collections::HashSet<_> = keys.collect();
        self.records
            .iter()
            .filter(|r| {
                r.query == query
                    && time.saturating_sub(r.last) <= REUSE_WINDOW
                    && keys.contains(r.key.as_str())
            })
            .max_by(|a, b| a.last.cmp(&b.last).then_with(|| a.key.cmp(&b.key)))
            .map(|r| r.key.clone())
    }

    pub fn frequency(&self, key: &str, time: u64) -> u32 {
        let total: f64 = self
            .records
            .iter()
            .filter(|r| r.key == key)
            .map(|r| decayed(r, time))
            .sum();
        ((1.0 - (-total / 10.0).exp()) * 1000.0) as u32
    }

    fn select(&mut self, selection: &Selection, time: u64) {
        if selection.query.is_empty() {
            return;
        }
        // Preserve selection order even when two actions finish within one millisecond.
        let time = time.max(
            self.records
                .iter()
                .map(|r| r.last)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        );
        if let Some(record) = self
            .records
            .iter_mut()
            .find(|r| r.query == selection.query && r.key == selection.key)
        {
            record.uses = decayed(record, time) + 1.0;
            record.last = time;
        } else {
            self.records.push(Record {
                query: selection.query.clone(),
                key: selection.key.clone(),
                last: time,
                uses: 1.0,
            });
        }
        self.records.sort_by_key(|r| std::cmp::Reverse(r.last));
        self.records.truncate(LIMIT);
    }
}

fn decayed(record: &Record, time: u64) -> f64 {
    record.uses * (-(time.saturating_sub(record.last) as f64) / DECAY_WINDOW).exp()
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn path() -> Option<PathBuf> {
    Some(std::env::var_os("MYTOOLS_HISTORY_PATH").map_or_else(
        || mytools_plugin_api::dirs::root().join("search-history.json"),
        PathBuf::from,
    ))
}

fn record_at(path: &Path, selection: &Selection, time: u64) -> anyhow::Result<()> {
    if selection.query.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    // Never overwrite an unreadable or corrupt history with an empty one.
    let mut history = History::read(path)?;
    history.select(selection, time);
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = fs::File::create(&temporary)?;
        serde_json::to_writer(&mut file, &history)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selection(query: &str, key: &str) -> Selection {
        Selection {
            query: normalize(query),
            key: key.into(),
        }
    }

    #[test]
    fn latest_available_selection_wins_and_expires_independently_of_frequency() {
        let mut history = History::default();
        for time in 1..100 {
            history.select(&selection(" DEV ", "old"), time);
        }
        history.select(&selection("dev", "new"), 100);
        assert_eq!(
            history
                .reused("ＤＥＶ", ["old", "new"].into_iter(), 101)
                .as_deref(),
            Some("new")
        );
        assert_eq!(
            history.reused("dev", ["old"].into_iter(), 101).as_deref(),
            Some("old")
        );
        assert!(
            history
                .reused("dev", ["old", "new"].into_iter(), REUSE_WINDOW + 101)
                .is_none()
        );
        assert!(history.reused("de", ["old"].into_iter(), 101).is_none());
    }

    #[test]
    fn frequency_combines_queries_decays_and_is_bounded_per_result() {
        let mut history = History::default();
        history.select(&selection("dev", "app"), 1);
        history.select(&selection("启动", "app"), 2);
        assert!(history.frequency("app", 2) > 100);
        assert_eq!(history.frequency("other", 2), 0);
        assert!(history.frequency("app", 30 * DAY) < history.frequency("app", 2));
        for time in 3..1000 {
            history.select(&selection("dev", "app"), time);
        }
        assert!(history.frequency("app", 1000) <= 1000);
    }

    #[test]
    fn persists_repeated_writes_and_rejects_corruption() {
        let root =
            std::env::temp_dir().join(format!("mytools-history-{}-{}", std::process::id(), now()));
        let path = root.join("history.json");
        record_at(&path, &selection("dev", "app"), 1).unwrap();
        record_at(&path, &selection("dev", "other"), 2).unwrap();
        assert_eq!(
            History::read(&path)
                .unwrap()
                .reused("dev", ["app", "other"].into_iter(), 3)
                .as_deref(),
            Some("other")
        );
        fs::write(&path, b"invalid").unwrap();
        assert!(record_at(&path, &selection("dev", "app"), 4).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identity_ignores_titles_and_separates_plugins_and_resource_types() {
        let resource = Resource::PluginItem {
            id: "stable-id".into(),
        };
        assert_eq!(
            Selection::new("one", &resource, "a").key,
            Selection::new("one", &resource, "b").key
        );
        assert_ne!(
            Selection::new("one", &resource, "a").key,
            Selection::new("two", &resource, "a").key
        );
        assert_ne!(
            Selection::new("one", &resource, "a").key,
            Selection::new(
                "one",
                &Resource::Text {
                    text: "stable-id".into()
                },
                "a"
            )
            .key
        );
    }
}
