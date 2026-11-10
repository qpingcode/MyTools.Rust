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
    /// Indexes shortcut files by file name, plus packaged apps that no shortcut already covers.
    pub fn new(paths: Vec<PathBuf>, packaged: Vec<crate::packaged::App>) -> Self {
        let mut named: Vec<(PathBuf, String)> = paths
            .into_iter()
            .map(|path| {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                (path, name)
            })
            .collect();
        let shortcuts: std::collections::HashSet<String> =
            named.iter().map(|(_, name)| name.to_lowercase()).collect();
        named.extend(
            packaged
                .into_iter()
                .filter(|app| !shortcuts.contains(&app.name.to_lowercase()))
                .map(|app| (app.path, app.name)),
        );
        let mut entries: Vec<_> = named
            .into_iter()
            .map(|(path, name)| {
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
                    && (entry.path.exists()
                        || mytools_plugin_api::paths::is_packaged_app(&entry.path))
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
        let packaged = vec![
            crate::packaged::App {
                name: "Claude".into(),
                path: mytools_plugin_api::paths::packaged_app("Claude_x!Claude"),
            },
            // Already covered by a shortcut of the same name.
            crate::packaged::App {
                name: "记事本".into(),
                path: mytools_plugin_api::paths::packaged_app("Notepad_x!App"),
            },
        ];
        let index = Index::new(paths.clone(), packaged);
        assert_eq!(index.search("claude")[0].title, "Claude");
        assert_eq!(index.search("记事").len(), 1);
        for query in ["studio VISUAL", "visualstudio", "vsc"] {
            assert_eq!(index.search(query)[0].title, "Visual Studio Code");
        }
        assert_eq!(index.search("code")[0].title, "Code Editor");
        assert_eq!(index.search("记事")[0].title, "记事本");
        assert_eq!(index.search(" ").len(), 4);
        assert!(index.search("unmatched").is_empty());
        std::fs::remove_file(&paths[0]).unwrap();
        assert!(index.search("vsc").is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
