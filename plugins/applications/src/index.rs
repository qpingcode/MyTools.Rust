use mytools_plugin_api::{Icon, Resource, SearchItem};
use std::path::PathBuf;

#[derive(Default)]
pub struct Index {
    entries: Vec<Application>,
}

struct Application {
    path: PathBuf,
    name: String,
    lowercase: String,
    compact: String,
    initials: String,
}

impl Index {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        let mut entries: Vec<_> = paths
            .into_iter()
            .map(|path| {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let lowercase = name.to_lowercase();
                let initials = lowercase
                    .split(|ch: char| !ch.is_alphanumeric())
                    .filter_map(|word| word.chars().next())
                    .collect();
                let compact = compact(&lowercase);
                Application {
                    path,
                    name,
                    lowercase,
                    compact,
                    initials,
                }
            })
            .collect();
        entries.sort_by(|left, right| {
            left.lowercase
                .cmp(&right.lowercase)
                .then(left.path.cmp(&right.path))
        });
        Self { entries }
    }

    pub fn search(&self, query: &str) -> Vec<SearchItem> {
        let query = query.trim().to_lowercase();
        let compact = compact(&query);
        let terms: Vec<_> = query.split_whitespace().collect();
        let mut matches: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| {
                (mytools_plugin_api::matching::rank(&entry.name, &query) < 4
                    || terms.iter().all(|term| entry.lowercase.contains(term))
                    || (!compact.is_empty()
                        && (entry.compact.contains(&compact)
                            || entry.initials.starts_with(&compact))))
                    && entry.path.exists()
            })
            .collect();
        // Preserve alphabetical order within each rank, preferring direct prefixes.
        matches.sort_by_key(|entry| {
            (
                !entry.lowercase.starts_with(&query),
                !compact.is_empty() && !entry.compact.starts_with(&compact),
            )
        });
        matches
            .into_iter()
            .take(50)
            .map(|entry| SearchItem {
                intent_match: false,
                title: entry.name.clone(),
                description: mytools_plugin_api::paths::display(&entry.path),
                icon: Some(Icon::Native {
                    native: entry.path.clone(),
                }),
                resource: Resource::Application {
                    path: entry.path.clone(),
                },
                actions: None,
                preview: None,
            })
            .collect()
    }
}

fn compact(value: &str) -> String {
    value.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_terms_compact_names_initials_and_omits_deleted_entries() {
        let root = std::env::temp_dir().join(format!("mytools-app-index-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let paths: Vec<_> = ["Visual Studio Code.lnk", "Code Editor.lnk", "记事本.lnk"]
            .into_iter()
            .map(|name| {
                let path = root.join(name);
                std::fs::write(&path, b"fixture").unwrap();
                path
            })
            .collect();
        let index = Index::new(paths.clone());
        for query in ["studio VISUAL", "visualstudio", "vsc"] {
            assert_eq!(index.search(query)[0].title, "Visual Studio Code");
        }
        assert_eq!(index.search("code")[0].title, "Code Editor");
        assert_eq!(index.search("记事")[0].title, "记事本");
        assert_eq!(index.search(" ").len(), 3);
        assert!(index.search("unmatched").is_empty());
        std::fs::remove_file(&paths[0]).unwrap();
        assert!(index.search("vsc").is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
