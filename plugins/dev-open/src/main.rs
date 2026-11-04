//! Opens files and folders in Rider, VS Code or Visual Studio.
//! The host offers these commands on every file/folder result (see `contributes.actions`
//! in plugin.json); typing an existing path also lists them directly.
use ides::Ide;
use mytools_plugin_api::{
    ActionId, ActionSpec, PluginActionRequest, PluginEffect, Resource, SearchItem, SearchResponse,
    i18n::Catalog, paths,
};
use std::path::{Path, PathBuf};

mod ides;
mod project;

const IDES: [Ide; 3] = [Ide::Rider, Ide::VsCode, Ide::VisualStudio];

fn parse_path(query: &str) -> Option<PathBuf> {
    let text = query.trim().trim_matches('"').trim();
    let path = PathBuf::from(text);
    (!text.is_empty() && path.is_absolute() && path.exists()).then_some(path)
}

fn icon(ide: Ide) -> &'static str {
    match ide {
        Ide::Rider => "builtin:square-code",
        Ide::VsCode => "builtin:code-xml",
        Ide::VisualStudio => "builtin:code",
    }
}

fn title(ide: Ide, catalog: &Catalog) -> String {
    catalog.get(&format!("action.{}", ide.command())).to_owned()
}

/// What each IDE will open for `path`, or why it cannot.
fn target(ide: Ide, path: &Path, catalog: &Catalog) -> Result<PathBuf, String> {
    match ide {
        Ide::VsCode => Ok(folder_of(path).to_path_buf()),
        Ide::Rider | Ide::VisualStudio => project::find(path).ok_or_else(|| {
            catalog.format(
                "no_project",
                &[("ide", ide.display_name()), ("path", &paths::display(path))],
            )
        }),
    }
}

fn folder_of(path: &Path) -> &Path {
    if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    }
}

/// Only existing absolute paths produce results, so global search stays quiet otherwise.
fn search(query: &str, catalog: &Catalog) -> SearchResponse {
    let Some(path) = parse_path(query) else {
        return SearchResponse::default();
    };
    let items = IDES
        .iter()
        .map(|&ide| {
            let description = match target(ide, &path, catalog) {
                Ok(target) => catalog.format("will_open", &[("target", &paths::display(&target))]),
                Err(error) => error,
            };
            SearchItem {
                intent_match: false,
                title: title(ide, catalog),
                icon: Some(icon(ide).into()),
                description,
                resource: Resource::PluginItem {
                    id: paths::display(&path),
                },
                actions: Some(vec![ActionSpec {
                    id: ActionId::Plugin,
                    title: Some(title(ide, catalog)),
                    command: Some(ide.command().into()),
                    item: Some(paths::display(&path)),
                }]),
                preview: None,
            }
        })
        .collect();
    SearchResponse {
        items,
        ..Default::default()
    }
}

fn open(ide: Ide, path: &Path, catalog: &Catalog) -> Result<String, String> {
    if !path.is_absolute() || !path.exists() {
        return Err(catalog.format("missing_path", &[("path", &paths::display(path))]));
    }
    let target = target(ide, path, catalog)?;
    let executable = ides::locate(ide, &ides::Config::load())
        .ok_or_else(|| catalog.format("not_installed", &[("ide", ide.display_name())]))?;
    let working_directory = folder_of(&target).to_path_buf();
    let arguments: Vec<&Path> = match ide {
        // Open the folder as workspace and, for a file, also focus that file.
        Ide::VsCode if path.is_file() => vec![target.as_path(), path],
        _ => vec![target.as_path()],
    };
    ides::launch(&executable, &arguments, &working_directory).map_err(|error| {
        catalog.format(
            "launch_failed",
            &[("ide", ide.display_name()), ("error", &error.to_string())],
        )
    })?;
    Ok(catalog.format(
        "opened",
        &[
            ("ide", ide.display_name()),
            ("target", &paths::display(&target)),
        ],
    ))
}

fn action(request: PluginActionRequest, catalog: &Catalog) -> SearchResponse {
    let result = Ide::from_command(&request.command)
        .ok_or_else(|| format!("unknown command {}", request.command))
        .and_then(|ide| {
            let path = request.item.as_deref().unwrap_or_default().trim();
            open(ide, Path::new(path), catalog)
        });
    match result {
        Ok(message) => SearchResponse {
            effect: Some(PluginEffect {
                message,
                dismiss: true,
                ..Default::default()
            }),
            ..Default::default()
        },
        Err(error) => SearchResponse {
            error: Some(error),
            ..Default::default()
        },
    }
}

fn main() -> std::io::Result<()> {
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    mytools_plugin_api::serve_interactive(
        |query| search(query, &catalog),
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
    fn only_existing_absolute_paths_produce_items() {
        assert!(search("hello", &catalog()).items.is_empty());
        assert!(search("", &catalog()).items.is_empty());
        assert!(
            search(r"Z:\definitely\missing", &catalog())
                .items
                .is_empty()
        );
        let temp = std::env::temp_dir();
        let items = search(&format!("\"{}\"", temp.display()), &catalog()).items;
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].title, "Open in VS Code");
    }

    #[test]
    fn unknown_command_and_missing_path_report_errors() {
        let unknown = action(
            PluginActionRequest {
                command: "emacs".into(),
                item: Some("C:\\".into()),
            },
            &catalog(),
        );
        assert!(unknown.error.is_some());
        let missing = action(
            PluginActionRequest {
                command: "rider".into(),
                item: Some(r"Z:\missing\App.sln".into()),
            },
            &catalog(),
        );
        assert!(missing.error.unwrap().contains("does not exist"));
    }
}
