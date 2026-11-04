use crate::{
    i18n,
    store::{Content, Entry, Store},
};
use mytools_plugin_api::{
    ActionId, ActionSpec, Icon, Preview, Resource, SearchItem, SearchResponse,
};
use std::path::Path;

/// Every action names its history entry: the result's resource may be a real file.
fn action(command: &str, id: &str) -> ActionSpec {
    ActionSpec {
        id: ActionId::Plugin,
        command: Some(command.into()),
        title: Some(i18n::text(command)),
        item: Some(id.into()),
    }
}

/// A single absolute file or folder is exposed as what it is, so the host and other
/// plugins (for example "Open in Rider") can act on it. Several files, text and images
/// stay opaque history items.
fn resource(entry: &Entry) -> Resource {
    match &entry.content {
        Content::Files { paths } => match paths.as_slice() {
            [path] if path.is_absolute() => {
                let folder = if path.exists() {
                    path.is_dir()
                } else {
                    path.extension().is_none()
                };
                let path = path.clone();
                if folder {
                    Resource::Folder { path }
                } else {
                    Resource::File { path }
                }
            }
            _ => Resource::PluginItem {
                id: entry.id.clone(),
            },
        },
        _ => Resource::PluginItem {
            id: entry.id.clone(),
        },
    }
}

fn title(entry: &Entry) -> String {
    match &entry.content {
        Content::Text { text, .. } => text
            .split_whitespace()
            .flat_map(|word| word.chars().chain(Some(' ')))
            .take(100)
            .collect::<String>()
            .trim()
            .to_owned(),
        Content::Image { width, height } => i18n::format(
            "image",
            &[
                ("width", &width.to_string()),
                ("height", &height.to_string()),
            ],
        ),
        Content::Files { paths } => i18n::format("files", &[("count", &paths.len().to_string())]),
    }
}

/// Picks an icon describing what was copied rather than a generic document glyph.
fn icon(content: &Content) -> Icon {
    match content {
        Content::Text { text, html } => text_icon(text, html.is_some()).into(),
        Content::Image { .. } => "builtin:image".into(),
        Content::Files { paths } => match paths.as_slice() {
            [path] => file_icon(path),
            _ => "builtin:files".into(),
        },
    }
}

/// A single existing file or folder shows its real system icon (for example the .docx icon).
fn file_icon(path: &Path) -> Icon {
    if path.is_absolute() && path.exists() {
        Icon::Native {
            native: path.to_path_buf(),
        }
    } else if path.extension().is_none() {
        "builtin:folder".into()
    } else {
        "builtin:file".into()
    }
}

fn text_icon(text: &str, has_html: bool) -> &'static str {
    let trimmed = text.trim();
    let single_line = !trimmed.contains('\n');
    let lower = trimmed.to_ascii_lowercase();
    if single_line && (lower.starts_with("http://") || lower.starts_with("https://")) {
        "builtin:link-2"
    } else if single_line && is_color(trimmed) {
        "builtin:palette"
    } else if single_line && looks_like_path(trimmed) {
        "builtin:folder-search"
    } else if has_html || looks_like_code(trimmed) {
        "builtin:code-xml"
    } else {
        "builtin:type"
    }
}

fn is_color(text: &str) -> bool {
    text.strip_prefix('#').is_some_and(|hex| {
        matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
    })
}

fn looks_like_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    let drive = bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && &bytes[1..3] == b":\\";
    drive || text.starts_with("\\\\")
}

fn looks_like_code(text: &str) -> bool {
    let structured = matches!(
        (text.chars().next(), text.chars().last()),
        (Some('{'), Some('}')) | (Some('['), Some(']')) | (Some('<'), Some('>'))
    );
    structured || (text.lines().count() > 1 && text.contains(';') && text.contains('{'))
}

pub fn search(store: &Store, query: &str) -> SearchResponse {
    let terms: Vec<_> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let items = store
        .entries
        .iter()
        .filter(|entry| {
            if terms.is_empty() {
                return true;
            }
            let text = match &entry.content {
                Content::Text { text, .. } => text.clone(),
                Content::Files { paths } => paths
                    .iter()
                    .map(|path| mytools_plugin_api::paths::display(path))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Content::Image { .. } => title(entry),
            }
            .to_lowercase();
            terms.iter().all(|term| text.contains(term))
        })
        .take(50)
        .map(|entry| {
            let title = title(entry);
            let time = chrono::DateTime::from_timestamp_millis(entry.timestamp as i64)
                .map(|time| {
                    time.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M:%S")
                        .to_string()
                })
                .unwrap_or_default();
            let metadata = i18n::format(
                "meta",
                &[("time", &time), ("size", &entry.bytes.to_string())],
            );
            let icon = icon(&entry.content);
            let (text, image) = match &entry.content {
                Content::Text { text, .. } => (text.clone(), None),
                Content::Files { paths } => (
                    paths
                        .iter()
                        .map(|path| mytools_plugin_api::paths::display(path))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None,
                ),
                Content::Image { .. } => (String::new(), Some(store.image_path(&entry.id, true))),
            };
            let mut preview: String = text.chars().take(2048).collect();
            if text.chars().count() > 2048 {
                preview.push_str(&format!("\n\n{}", i18n::text("truncated")));
            }
            let mut actions = vec![action("paste", &entry.id)];
            if matches!(entry.content, Content::Text { .. }) {
                actions.push(action("plain", &entry.id));
            }
            actions.extend(["copy", "delete", "clear"].map(|command| action(command, &entry.id)));
            SearchItem {
                intent_match: false,
                title,
                description: metadata.clone(),
                icon: Some(icon),
                resource: resource(entry),
                actions: Some(actions),
                preview: Some(Preview {
                    title: String::new(),
                    text: preview,
                    metadata,
                    image,
                    text_file: (!matches!(entry.content, Content::Image { .. }))
                        .then(|| store.root.join(format!("{}.txt", entry.id))),
                }),
            }
        })
        .collect::<Vec<_>>();
    let items = if items.is_empty() && query.trim().is_empty() {
        vec![SearchItem {
            intent_match: false,
            title: i18n::text("empty"),
            description: i18n::text("empty_detail"),
            icon: Some("builtin:clipboard".into()),
            resource: Resource::PluginItem { id: "empty".into() },
            actions: Some(vec![]),
            preview: Some(Preview {
                text: i18n::text("empty_detail"),
                ..Default::default()
            }),
        }]
    } else {
        items
    };
    SearchResponse {
        items,
        revision: Some(store.revision),
        error: store.last_error.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_icons_describe_the_content() {
        assert_eq!(text_icon("https://example.com/a", false), "builtin:link-2");
        assert_eq!(text_icon("#94ded1", false), "builtin:palette");
        assert_eq!(text_icon(r"C:\git\repo", false), "builtin:folder-search");
        assert_eq!(text_icon(r#"{"a":1}"#, false), "builtin:code-xml");
        assert_eq!(text_icon("hello", true), "builtin:code-xml");
        assert_eq!(text_icon("hello world", false), "builtin:type");
    }

    #[test]
    fn single_files_and_folders_expose_their_real_type() {
        let entry = |content| Entry {
            id: "e1".into(),
            timestamp: 0,
            bytes: 0,
            content,
        };
        let files = |paths: Vec<std::path::PathBuf>| resource(&entry(Content::Files { paths }));
        let folder = std::env::temp_dir();
        assert_eq!(
            files(vec![folder.clone()]),
            Resource::Folder {
                path: folder.clone()
            }
        );
        assert_eq!(
            files(vec![r"Z:\missing\report.pdf".into()]),
            Resource::File {
                path: r"Z:\missing\report.pdf".into()
            }
        );
        let opaque = Resource::PluginItem { id: "e1".into() };
        assert_eq!(files(vec![folder.clone(), folder]), opaque);
        assert_eq!(files(vec!["relative.txt".into()]), opaque);
        assert_eq!(
            resource(&entry(Content::Text {
                text: "C:\\a.txt".into(),
                html: None
            })),
            opaque
        );
    }

    #[test]
    fn file_icons_use_native_icon_for_one_existing_path() {
        let existing = std::env::temp_dir();
        let files = |paths: Vec<std::path::PathBuf>| icon(&Content::Files { paths });
        assert_eq!(
            files(vec![existing.clone()]),
            Icon::Native {
                native: existing.clone()
            }
        );
        assert_eq!(
            files(vec![existing.clone(), existing]),
            "builtin:files".into()
        );
        assert_eq!(
            files(vec![r"Z:\missing\nothing.txt".into()]),
            "builtin:file".into()
        );
    }
}
