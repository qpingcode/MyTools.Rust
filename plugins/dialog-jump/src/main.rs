//! Moves the Save/Open dialog the user came from to another folder, like Listary's quick
//! switch: press the plugin's hotkey in a file dialog, pick a folder or file, and the dialog
//! shows that folder (a file's own folder), keeping the file name. Files and folders are
//! searched with Everything; file and folder results of other plugins get the same command
//! through `contributes.actions`.
use candidates::{Candidate, Source};
use mytools_everything::SearchError;
use mytools_plugin_api::{
    ActionId, ActionSpec, Icon, PluginActionRequest, PluginEffect, Resource, SearchItem,
    SearchResponse, i18n::Catalog, paths,
};
use std::path::{Path, PathBuf};

mod candidates;
#[cfg(windows)]
mod dialog;
#[cfg(windows)]
mod explorer;
#[cfg(windows)]
mod win;

const JUMP: &str = "jump";

fn search(query: &str, catalog: &Catalog) -> Vec<SearchItem> {
    if cfg!(not(windows)) {
        return vec![hint(catalog.get("unsupported"), "")];
    }
    if !dialog_in_front() {
        return vec![hint(
            catalog.get("no_dialog"),
            catalog.get("no_dialog_detail"),
        )];
    }
    let (indexed, problem) = match search_everything(query) {
        Ok(hits) => (hits, None),
        Err(SearchError::Cancelled) => return Vec::new(),
        Err(error) => (Vec::new(), Some(everything_hint(error, catalog))),
    };
    let mut items: Vec<SearchItem> = candidates::collect(query, open_folders(), indexed)
        .into_iter()
        .map(|candidate| item(candidate, catalog))
        .collect();
    // Last, so that it never pushes real results down.
    items.extend(problem);
    if items.is_empty() {
        items.push(hint(
            catalog.get("no_results"),
            catalog.get("no_results_detail"),
        ));
    }
    items
}

/// Everything's hits as `(path, is_file)`; none for an empty query or a typed path.
fn search_everything(query: &str) -> Result<Vec<(PathBuf, bool)>, SearchError> {
    if query.trim().is_empty() || candidates::typed_path(query).is_some() {
        return Ok(Vec::new());
    }
    let found = mytools_everything::query_everything(query.trim())?;
    Ok(found
        .hits
        .into_iter()
        .map(|hit| (hit.path, !hit.folder && !hit.drive))
        .collect())
}

fn everything_hint(error: SearchError, catalog: &Catalog) -> SearchItem {
    let title = match error {
        SearchError::NotRunning => catalog.get("everything.not_running").to_owned(),
        SearchError::Busy => catalog.get("everything.busy").to_owned(),
        SearchError::Failed(error) => catalog.format("everything.failed", &[("error", &error)]),
        SearchError::Cancelled => String::new(),
    };
    let mut item = hint(&title, catalog.get("no_results_detail"));
    item.icon = Some("builtin:triangle-alert".into());
    item
}

fn item(candidate: Candidate, catalog: &Catalog) -> SearchItem {
    let display = paths::display(&candidate.path);
    let description = match candidate.source {
        Source::Explorer => catalog.format("explorer", &[("path", &display)]),
        _ if candidate.file => candidate
            .path
            .parent()
            .map(paths::display)
            .unwrap_or_default(),
        _ => display.clone(),
    };
    let resource = if candidate.file {
        Resource::File {
            path: candidate.path.clone(),
        }
    } else {
        Resource::Folder {
            path: candidate.path.clone(),
        }
    };
    SearchItem {
        intent_match: false,
        title: candidates::display_name(&candidate.path),
        icon: Some(Icon::Native {
            native: candidate.path,
        }),
        description,
        resource,
        actions: Some(vec![ActionSpec {
            id: ActionId::Plugin,
            title: Some(catalog.get("action.jump").into()),
            command: Some(JUMP.into()),
            item: Some(display),
        }]),
        preview: None,
    }
}

fn hint(title: &str, detail: &str) -> SearchItem {
    SearchItem {
        intent_match: false,
        title: title.into(),
        icon: Some("builtin:folder-search".into()),
        description: detail.into(),
        resource: Resource::Text {
            text: detail.into(),
        },
        actions: Some(vec![]),
        preview: None,
    }
}

/// The folder to show for a result: the folder itself, or the folder a file is in.
fn target_folder(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    if path.is_dir() {
        return Some(path.to_path_buf());
    }
    path.is_file()
        .then(|| path.parent().map(Path::to_path_buf))
        .flatten()
}

fn action(request: PluginActionRequest, catalog: &Catalog) -> SearchResponse {
    if request.command != JUMP {
        return failure(format!("unknown command {}", request.command));
    }
    let path = PathBuf::from(request.item.as_deref().unwrap_or_default().trim());
    let Some(folder) = target_folder(&path) else {
        return failure(catalog.format("missing_path", &[("path", &paths::display(&path))]));
    };
    match start_jump(&folder) {
        // The dialog comes back to the front once the launcher hides.
        Ok(()) => SearchResponse {
            effect: Some(PluginEffect::hide()),
            ..Default::default()
        },
        Err(()) => failure(catalog.get("dialog_gone").into()),
    }
}

fn failure(error: String) -> SearchResponse {
    SearchResponse {
        error: Some(error),
        ..Default::default()
    }
}

#[cfg(windows)]
fn dialog_in_front() -> bool {
    dialog::behind_launcher().is_some()
}

#[cfg(not(windows))]
fn dialog_in_front() -> bool {
    false
}

#[cfg(windows)]
fn open_folders() -> Vec<PathBuf> {
    explorer::open_folders()
}

#[cfg(not(windows))]
fn open_folders() -> Vec<PathBuf> {
    Vec::new()
}

/// Finds the dialog now, while the launcher is still in front, and jumps on a thread once
/// the launcher has hidden; the reply must come first so that the launcher hides.
#[cfg(windows)]
fn start_jump(folder: &Path) -> Result<(), ()> {
    let target = dialog::behind_launcher().ok_or(())?;
    let folder = folder.to_path_buf();
    std::thread::spawn(move || {
        if let Err(error) = dialog::jump(target, &folder) {
            mytools_plugin_api::log::warn(
                "dialog-jump.failed",
                format!("{}: {error}", target.title()),
            );
        }
    });
    Ok(())
}

#[cfg(not(windows))]
fn start_jump(_folder: &Path) -> Result<(), ()> {
    Err(())
}

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
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
        |request| action(request, &catalog),
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
    fn folder_results_jump_with_their_path() {
        let item = item(
            Candidate {
                path: PathBuf::from(r"C:\work\reports"),
                file: false,
                source: Source::Explorer,
            },
            &catalog(),
        );
        assert_eq!(item.title, "reports");
        assert!(item.description.contains(r"C:\work\reports"));
        assert!(matches!(item.resource, Resource::Folder { .. }));
        let action = &item.actions.unwrap()[0];
        assert_eq!(action.command.as_deref(), Some(JUMP));
        assert_eq!(action.item.as_deref(), Some(r"C:\work\reports"));
    }

    #[test]
    fn file_results_show_their_folder_and_jump_to_it() {
        let item = item(
            Candidate {
                path: PathBuf::from(r"C:\work\report.pdf"),
                file: true,
                source: Source::Everything,
            },
            &catalog(),
        );
        assert_eq!(item.title, "report.pdf");
        assert_eq!(item.description, r"C:\work");
        assert!(matches!(item.resource, Resource::File { .. }));
        let exe = std::env::current_exe().unwrap();
        assert_eq!(
            target_folder(&exe).as_deref(),
            exe.parent(),
            "a file jumps to its own folder"
        );
        let folder = exe.parent().unwrap();
        assert_eq!(target_folder(folder).as_deref(), Some(folder));
    }

    #[test]
    fn jumping_to_a_missing_path_fails_without_hiding() {
        let response = action(
            PluginActionRequest {
                command: JUMP.into(),
                item: Some(r"C:\definitely\missing\folder".into()),
                editable: None,
                argument: None,
            },
            &catalog(),
        );
        assert!(response.effect.is_none());
        assert!(response.error.unwrap().contains("no longer exists"));
    }

    #[test]
    fn locales_have_the_same_keys() {
        let keys = |text: &str| {
            let map: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(text).unwrap();
            map.keys().cloned().collect::<Vec<_>>()
        };
        assert_eq!(
            keys(include_str!("../locales/en.json")),
            keys(include_str!("../locales/zh-CN.json"))
        );
    }
}
