//! File search backed by a running Everything (voidtools) instance.
use mytools_plugin_api::{
    ActionId, ActionSpec, Icon, PluginActionRequest, PluginEffect, Preview, Resource, SearchItem,
    SearchResponse, i18n::Catalog,
};
use std::path::PathBuf;

#[cfg(windows)]
mod ipc;
mod relevance;

const MAX_RESULTS: usize = 50;
/// Used when the host does not say how long it waits (`timeoutMs`).
#[cfg(windows)]
const DEFAULT_BUDGET: std::time::Duration = std::time::Duration::from_millis(900);
/// Kept free before the host's deadline for ranking and writing the response.
#[cfg(windows)]
const RESPONSE_MARGIN: std::time::Duration = std::time::Duration::from_millis(100);
/// A tier is only started when at least this much of the budget is left.
#[cfg(windows)]
const MIN_TIER_TIME: std::time::Duration = std::time::Duration::from_millis(150);
/// Plugin command behind "press Enter to retry".
const RETRY: &str = "retry";

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
    /// Everything did not answer within the search budget, usually while it is busy
    /// updating its index after many file changes.
    Busy,
    /// The host sent a newer query; this result would be discarded.
    Cancelled,
    Failed(String),
}

/// Ranked hits, and whether some tiers were skipped because Everything was too slow.
#[cfg_attr(not(windows), allow(dead_code))]
struct Found {
    hits: Vec<Hit>,
    incomplete: bool,
}

#[cfg(windows)]
fn query_everything(query: &str) -> Result<Found, SearchError> {
    use std::time::Instant;
    let deadline = mytools_plugin_api::deadline()
        .map(|deadline| deadline.checked_sub(RESPONSE_MARGIN).unwrap_or(deadline))
        .unwrap_or_else(|| Instant::now() + DEFAULT_BUDGET);
    let tiers = relevance::tiers(query);
    let tier_count = tiers.len();
    let mut hits = Vec::new();
    let mut first_error = None;
    let mut finished = 0;
    for (text, limit) in tiers {
        if mytools_plugin_api::cancelled() {
            return Err(SearchError::Cancelled);
        }
        // Everything keeps running a query after we stop waiting for it, so a query that
        // cannot finish in time would only delay the next search; skip it instead.
        if finished > 0 && deadline.saturating_duration_since(Instant::now()) < MIN_TIER_TIME {
            break;
        }
        match ipc::search(&text, limit, deadline) {
            Ok(found) => {
                hits.extend(found);
                finished += 1;
            }
            Err(error @ (SearchError::NotRunning | SearchError::Cancelled)) => return Err(error),
            Err(SearchError::Busy) => {
                first_error.get_or_insert(SearchError::Busy);
                break; // Further queries would queue behind the unfinished one.
            }
            // A failing tier (e.g. unsupported syntax) still leaves the others to run.
            Err(error) => {
                first_error.get_or_insert(error);
                finished += 1;
            }
        }
    }
    match first_error {
        Some(error) if hits.is_empty() => Err(error),
        _ => Ok(Found {
            hits: relevance::rank_hits(hits, query, MAX_RESULTS),
            incomplete: finished < tier_count,
        }),
    }
}

#[cfg(not(windows))]
fn query_everything(_query: &str) -> Result<Found, SearchError> {
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
        Ok(found) if found.hits.is_empty() && found.incomplete => {
            vec![retry_hint(catalog.get("busy"), catalog)]
        }
        Ok(found) if found.hits.is_empty() => vec![hint(
            &catalog.format("no_results", &[("query", query)]),
            catalog.get("usage_detail"),
        )],
        Ok(found) => {
            let mut items: Vec<_> = found
                .hits
                .into_iter()
                .map(|hit| item(hit, catalog))
                .collect();
            if found.incomplete {
                // Last, so it never pushes real results down.
                items.push(retry_hint(catalog.get("incomplete"), catalog));
            }
            items
        }
        Err(SearchError::NotRunning) => vec![not_running(catalog)],
        Err(SearchError::Busy) => vec![retry_hint(catalog.get("busy"), catalog)],
        Err(SearchError::Cancelled) => vec![],
        Err(SearchError::Failed(error)) => vec![hint(
            &catalog.format("failed", &[("error", &error)]),
            catalog.get("usage_detail"),
        )],
    }
}

/// A hint whose Enter runs the same query again.
fn retry_hint(title: &str, catalog: &Catalog) -> SearchItem {
    let mut item = hint(title, catalog.get("retry_detail"));
    item.icon = Some("builtin:refresh-cw".into());
    item.resource = Resource::PluginItem { id: RETRY.into() };
    item.actions = Some(vec![ActionSpec {
        id: ActionId::Plugin,
        title: Some(catalog.get("retry").into()),
        command: Some(RETRY.into()),
        item: None,
    }]);
    item
}

fn action(request: PluginActionRequest) -> SearchResponse {
    if request.command != RETRY {
        return SearchResponse {
            error: Some(format!("unknown command {}", request.command)),
            ..Default::default()
        };
    }
    SearchResponse {
        effect: Some(PluginEffect {
            refresh: true,
            ..Default::default()
        }),
        ..Default::default()
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
    // The native icon already tells folders from files, so both show only the parent.
    let description = if hit.drive {
        catalog.get("drive").to_owned()
    } else {
        parent
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
    mytools_plugin_api::serve_interactive(
        |query| SearchResponse {
            items: search(query, &catalog),
            ..Default::default()
        },
        action,
    )
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
        assert_eq!(folder.description, r"C:\work");
    }
}
