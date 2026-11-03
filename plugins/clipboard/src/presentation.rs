use crate::{
    i18n,
    store::{Content, Entry, Store},
};
use mytools_plugin_api::{ActionId, ActionSpec, Preview, Resource, SearchItem, SearchResponse};

fn action(command: &str) -> ActionSpec {
    ActionSpec {
        id: ActionId::Plugin,
        command: Some(command.into()),
        title: Some(i18n::text(command)),
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
            let (text, image, icon) = match &entry.content {
                Content::Text { text, .. } => (text.clone(), None, "builtin:file-text"),
                Content::Files { paths } => (
                    paths
                        .iter()
                        .map(|path| mytools_plugin_api::paths::display(path))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    None,
                    "builtin:folder",
                ),
                Content::Image { .. } => (
                    String::new(),
                    Some(store.image_path(&entry.id, true)),
                    "builtin:image",
                ),
            };
            let mut preview: String = text.chars().take(2048).collect();
            if text.chars().count() > 2048 {
                preview.push_str(&format!("\n\n{}", i18n::text("truncated")));
            }
            let mut actions = vec![action("paste")];
            if matches!(entry.content, Content::Text { .. }) {
                actions.push(action("plain"));
            }
            actions.extend([action("copy"), action("delete"), action("clear")]);
            SearchItem {
                intent_match: false,
                title,
                description: metadata.clone(),
                icon: Some(icon.into()),
                resource: Resource::PluginItem {
                    id: entry.id.clone(),
                },
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
