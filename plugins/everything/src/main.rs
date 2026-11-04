//! File search backed by a running Everything (voidtools) instance.
use mytools_plugin_api::{Icon, Preview, Resource, SearchItem, i18n::Catalog};
use std::path::PathBuf;

#[cfg(windows)]
mod ipc;
mod relevance;

const MAX_RESULTS: usize = 50;
/// The host waits one second for a search; all tiered queries share this budget.
#[cfg(windows)]
const SEARCH_BUDGET: std::time::Duration = std::time::Duration::from_millis(800);

/// One Everything result: a file, folder or drive.
#[derive(Debug, PartialEq)]
pub struct Hit {
    pub path: PathBuf,
    pub folder: bool,
    pub drive: bool,
}

#[derive(Debug)]
pub enum SearchError {
    NotRunning,
    Failed(String),
}

#[cfg(windows)]
fn query_everything(query: &str) -> Result<Vec<Hit>, SearchError> {
    let deadline = std::time::Instant::now() + SEARCH_BUDGET;
    let mut hits = Vec::new();
    let mut last_error = None;
    for (text, limit) in relevance::tiers(query) {
        match ipc::search(&text, limit, deadline) {
            Ok(found) => hits.extend(found),
            Err(SearchError::NotRunning) => return Err(SearchError::NotRunning),
            // A slow later tier still leaves the stronger tiers' results to show.
            Err(error) => last_error = Some(error),
        }
    }
    match last_error {
        Some(error) if hits.is_empty() => Err(error),
        _ => Ok(relevance::rank_hits(hits, query, MAX_RESULTS)),
    }
}

#[cfg(not(windows))]
fn query_everything(_query: &str) -> Result<Vec<Hit>, SearchError> {
    Err(SearchError::Failed("unsupported".into()))
}

fn search(query: &str, catalog: &Catalog) -> Vec<SearchItem> {
    let query = query.trim();
    if query.is_empty() {
        return vec![hint(catalog.get("usage"), catalog.get("usage_detail"))];
    }
    if cfg!(not(windows)) {
        return vec![hint(catalog.get("unsupported"), "")];
    }
    match query_everything(query) {
        Ok(hits) if hits.is_empty() => vec![hint(
            &catalog.format("no_results", &[("query", query)]),
            catalog.get("usage_detail"),
        )],
        Ok(hits) => hits.into_iter().map(|hit| item(hit, catalog)).collect(),
        Err(SearchError::NotRunning) => vec![not_running(catalog)],
        Err(SearchError::Failed(error)) => vec![hint(
            &catalog.format("failed", &[("error", &error)]),
            catalog.get("usage_detail"),
        )],
    }
}

fn item(hit: Hit, catalog: &Catalog) -> SearchItem {
    let display = mytools_plugin_api::paths::display(&hit.path);
    let title = hit
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| display.clone());
    let parent = hit
        .path
        .parent()
        .map(mytools_plugin_api::paths::display)
        .unwrap_or_default();
    let description = match (hit.drive, hit.folder) {
        (true, _) => catalog.get("drive").to_owned(),
        (false, true) => format!("{} · {parent}", catalog.get("folder")),
        _ => parent,
    };
    let resource = if hit.folder || hit.drive {
        Resource::Folder {
            path: hit.path.clone(),
        }
    } else {
        Resource::File {
            path: hit.path.clone(),
        }
    };
    SearchItem {
        intent_match: false,
        title,
        icon: Some(Icon::Native { native: hit.path }),
        description,
        resource,
        actions: None,
        preview: None,
    }
}

fn hint(title: &str, detail: &str) -> SearchItem {
    SearchItem {
        intent_match: false,
        title: title.into(),
        icon: Some("builtin:file-search".into()),
        description: detail.into(),
        resource: Resource::Text {
            text: detail.into(),
        },
        actions: Some(vec![]),
        preview: Some(Preview {
            text: detail.into(),
            ..Default::default()
        }),
    }
}

fn not_running(catalog: &Catalog) -> SearchItem {
    let installed = everything_executable();
    let mut item = hint(
        catalog.get("not_running"),
        catalog.get("not_running_detail"),
    );
    item.icon = Some("builtin:triangle-alert".into());
    if let Some(path) = installed {
        item.resource = Resource::Application { path };
        item.actions = None;
    }
    item
}

fn everything_executable() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .flat_map(|root| {
            let root = PathBuf::from(root);
            [
                root.join("Everything").join("Everything.exe"),
                root.join("Programs")
                    .join("Everything")
                    .join("Everything.exe"),
            ]
        })
        .find(|path| path.is_file())
}

fn main() -> std::io::Result<()> {
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    mytools_plugin_api::serve(|query| search(query, &catalog))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::from_json(
            include_str!("../locales/en.json"),
            include_str!("../locales/en.json"),
        )
        .unwrap()
    }

    #[test]
    fn empty_query_shows_usage_without_actions() {
        let items = search("  ", &catalog());
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].actions.as_deref().map(<[_]>::len), Some(0));
    }

    #[test]
    fn hits_become_file_and_folder_resources_with_native_icons() {
        let file = item(
            Hit {
                path: r"C:\work\app\App.sln".into(),
                folder: false,
                drive: false,
            },
            &catalog(),
        );
        assert_eq!(file.title, "App.sln");
        assert_eq!(file.description, r"C:\work\app");
        assert!(matches!(file.resource, Resource::File { .. }));
        assert_eq!(
            file.icon,
            Some(Icon::Native {
                native: r"C:\work\app\App.sln".into()
            })
        );
        let folder = item(
            Hit {
                path: r"C:\work\app".into(),
                folder: true,
                drive: false,
            },
            &catalog(),
        );
        assert!(matches!(folder.resource, Resource::Folder { .. }));
        assert!(folder.description.starts_with("Folder"));
    }
}
