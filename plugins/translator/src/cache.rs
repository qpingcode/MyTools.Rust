//! The most recent translations, kept on disk so repeated lookups need no API call and an
//! empty `tr` query can list them.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Entry {
    pub input: String,
    pub target: String,
    pub translation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phonetic: Option<String>,
    /// A short English definition of the English word, for single-word lookups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<Example>,
}

/// An English example sentence using the looked-up word, with its Chinese translation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Example {
    pub sentence: String,
    pub translation: String,
}

pub struct Cache {
    path: Option<PathBuf>,
    limit: usize,
    /// Most recently translated first.
    entries: Vec<Entry>,
}

impl Cache {
    pub fn open(limit: usize) -> Self {
        Self::at(default_path(), limit)
    }

    pub fn at(path: Option<PathBuf>, limit: usize) -> Self {
        let mut entries: Vec<Entry> = path
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        entries.truncate(limit);
        Self {
            path,
            limit,
            entries,
        }
    }

    /// Applies a changed `cache_size`; shrinking drops the oldest entries.
    pub fn set_limit(&mut self, limit: usize) {
        if limit != self.limit {
            self.limit = limit;
            if self.entries.len() > limit {
                self.entries.truncate(limit);
                self.save();
            }
        }
    }

    pub fn get(&self, input: &str, target: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|entry| entry.input == input && entry.target == target)
    }

    /// Stores a new or repeated translation as the most recent one.
    pub fn put(&mut self, entry: Entry) {
        if self.limit == 0 {
            return;
        }
        self.entries
            .retain(|old| !(old.input == entry.input && old.target == entry.target));
        self.entries.insert(0, entry);
        self.entries.truncate(self.limit);
        self.save();
    }

    /// Not shown at the moment; a history command is planned to list these again.
    #[cfg(test)]
    pub fn recent(&self) -> &[Entry] {
        &self.entries
    }

    /// Best effort: a cache that cannot be written still works for this session.
    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Err(error) = write_atomically(path, &self.entries) {
            eprintln!("translator cache: {error}");
        }
    }
}

fn write_atomically(path: &Path, entries: &[Entry]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(entries)?)?;
    std::fs::rename(&temporary, path)
}

fn default_path() -> Option<PathBuf> {
    Some(std::env::var_os("MYTOOLS_TRANSLATOR_CACHE").map_or_else(
        || mytools_plugin_api::dirs::plugin("translator").join("cache.json"),
        PathBuf::from,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(input: &str, translation: &str) -> Entry {
        Entry {
            input: input.into(),
            target: "Simplified Chinese".into(),
            translation: translation.into(),
            phonetic: None,
            definition: None,
            examples: vec![],
        }
    }

    #[test]
    fn keeps_the_most_recent_entries_and_persists_them() {
        let path = std::env::temp_dir().join(format!(
            "mytools-translator-cache-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let mut cache = Cache::at(Some(path.clone()), 2);
        cache.put(entry("one", "一"));
        cache.put(entry("two", "二"));
        cache.put(entry("one", "一（新）"));
        cache.put(entry("three", "三"));
        let inputs = |cache: &Cache| {
            cache
                .recent()
                .iter()
                .map(|entry| entry.input.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(inputs(&cache), ["three", "one"]);
        assert_eq!(
            cache.get("one", "Simplified Chinese").unwrap().translation,
            "一（新）"
        );
        assert!(cache.get("two", "Simplified Chinese").is_none());

        let mut reopened = Cache::at(Some(path.clone()), 2);
        assert_eq!(inputs(&reopened), ["three", "one"]);
        reopened.set_limit(1);
        assert_eq!(inputs(&Cache::at(Some(path.clone()), 5)), ["three"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn zero_size_disables_caching() {
        let mut cache = Cache::at(None, 0);
        cache.put(entry("one", "一"));
        assert!(cache.recent().is_empty());
    }
}
