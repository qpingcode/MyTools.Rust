mod store;
use mytools_plugin_api::{
    PluginActionRequest, PluginEffect, Resource, SearchResponse, i18n::Catalog, text,
};
use std::cell::RefCell;
use store::{Phrase, Store};

struct State {
    store: Store,
    pending: Option<(String, Phrase)>,
    revision: u64,
}

fn draft(query: &str) -> Result<Option<Phrase>, String> {
    let Some(value) = query.strip_prefix("add ") else {
        return Ok(None);
    };
    let (trigger, content) = value
        .split_once('|')
        .ok_or_else(|| "add <trigger> | <text>".to_owned())?;
    let trigger = trigger.trim();
    if trigger.is_empty() || trigger.len() > 128 {
        return Err("1..128 bytes".into());
    }
    // @file supports multiline snippets; inline content keeps its internal whitespace.
    let content = text::input(content.trim_start())?;
    if content.is_empty() {
        return Err("1..16384 bytes".into());
    }
    Ok(Some(Phrase {
        trigger: trigger.into(),
        content,
    }))
}

impl State {
    fn search(&mut self, query: &str, catalog: &Catalog) -> SearchResponse {
        let mut items = vec![];
        match draft(query) {
            Ok(Some(phrase)) => {
                let id = format!("draft:{}", serde_json::to_string(&phrase).unwrap());
                if self
                    .pending
                    .as_ref()
                    .is_none_or(|(previous, _)| previous != &id)
                {
                    self.revision += 1;
                }
                let mut item = text::item(
                    catalog.format("save.title", &[("trigger", &phrase.trigger)]),
                    phrase.content.clone(),
                    catalog.get("save.hint"),
                );
                item.resource = Resource::PluginItem { id: id.clone() };
                item.actions = Some(vec![text::command("save", catalog.get("save").into())]);
                self.pending = Some((id, phrase));
                items.push(item);
            }
            Err(error) => {
                self.pending = None;
                items.push(text::hint(catalog.get("usage"), error));
            }
            Ok(None) => {
                self.pending = None;
                let query = query.trim().to_lowercase();
                let mut phrases: Vec<_> = self
                    .store
                    .phrases
                    .iter()
                    .filter(|phrase| {
                        mytools_plugin_api::matching::rank(&phrase.trigger, &query) < 4
                            || phrase.content.to_lowercase().contains(&query)
                    })
                    .collect();
                phrases.sort_by_key(|phrase| {
                    (
                        !phrase.trigger.to_lowercase().starts_with(&query),
                        phrase.trigger.to_lowercase(),
                    )
                });
                for phrase in phrases.into_iter().take(50) {
                    let summary: String = phrase.content.chars().take(150).collect();
                    let mut item = text::item(&phrase.trigger, phrase.content.clone(), summary);
                    item.resource = Resource::PluginItem {
                        id: format!("phrase:{}", phrase.trigger),
                    };
                    item.actions = Some(vec![
                        text::command("copy", catalog.get("copy").into()),
                        text::command("delete", catalog.get("delete").into()),
                    ]);
                    items.push(item);
                }
                // This hint is only returned for an empty query; ordinary global searches stay clean.
                if items.is_empty() && query.is_empty() {
                    items.push(text::hint(
                        catalog.get("usage"),
                        catalog.get("usage.detail"),
                    ));
                }
            }
        }
        // Many long snippets must still fit in the host's 1 MiB JSON-line limit.
        let mut bytes = 0;
        items.retain(|item| {
            bytes += serde_json::to_vec(item).map_or(1_048_576, |json| json.len());
            bytes < 900_000
        });
        // Input changes are reset by the host; saved data changes increment the revision.
        SearchResponse {
            items,
            revision: Some(self.revision),
            ..Default::default()
        }
    }

    fn execute(
        &mut self,
        request: PluginActionRequest,
        catalog: &Catalog,
    ) -> Result<PluginEffect, String> {
        let mut phrases = self.store.phrases.clone();
        let message = match request.command.as_str() {
            "save" => {
                let phrase = self
                    .pending
                    .as_ref()
                    .filter(|(id, _)| *id == request.item)
                    .map(|(_, phrase)| phrase.clone())
                    .ok_or_else(|| catalog.get("missing").to_owned())?;
                phrases.retain(|saved| saved.trigger != phrase.trigger);
                phrases.push(phrase);
                "saved"
            }
            "copy" | "delete" => {
                let trigger = request
                    .item
                    .strip_prefix("phrase:")
                    .ok_or_else(|| catalog.get("missing").to_owned())?;
                let phrase = phrases
                    .iter()
                    .find(|phrase| phrase.trigger == trigger)
                    .ok_or_else(|| catalog.get("missing").to_owned())?;
                if request.command == "copy" {
                    return Ok(PluginEffect {
                        copy_text: Some(phrase.content.clone()),
                        message: catalog.get("copied").into(),
                        ..Default::default()
                    });
                }
                phrases.retain(|phrase| phrase.trigger != trigger);
                "deleted"
            }
            _ => return Err(catalog.get("missing").into()),
        };
        self.store
            .commit(phrases)
            .map_err(|error| catalog.format("store.error", &[("error", &error)]))?;
        self.revision += 1;
        Ok(PluginEffect {
            refresh: true,
            message: catalog.get(message).into(),
            ..Default::default()
        })
    }
}

fn main() -> std::io::Result<()> {
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    let store = store::directory()
        .ok_or_else(|| "data directory unavailable".into())
        .and_then(Store::open);
    let state = RefCell::new(store.map(|store| State {
        store,
        pending: None,
        revision: 0,
    }));
    mytools_plugin_api::serve_interactive(
        |query| match &mut *state.borrow_mut() {
            Ok(state) => state.search(query, &catalog),
            Err(error) => SearchResponse {
                error: Some(catalog.format("store.error", &[("error", error)])),
                ..Default::default()
            },
        },
        |request| {
            let result = match &mut *state.borrow_mut() {
                Ok(state) => state.execute(request, &catalog),
                Err(error) => Err(error.clone()),
            };
            match result {
                Ok(effect) => SearchResponse {
                    effect: Some(effect),
                    ..Default::default()
                },
                Err(error) => SearchResponse {
                    error: Some(error),
                    ..Default::default()
                },
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(command: &str, item: &str) -> PluginActionRequest {
        PluginActionRequest {
            command: command.into(),
            item: item.into(),
        }
    }

    #[test]
    fn search_does_not_save_and_actions_update_copy_delete_and_persist() {
        let root =
            std::env::temp_dir().join(format!("mytools-quick-actions-{}", std::process::id()));
        let mut state = State {
            store: Store::open(root.clone()).unwrap(),
            pending: None,
            revision: 0,
        };
        let catalog = Catalog::from_json("{}", "{}").unwrap();
        for content in ["first", "中文\nsecond"] {
            let query = format!("add mail | {content}");
            let response = state.search(&query, &catalog);
            assert_eq!(state.store.phrases.len(), usize::from(content != "first"));
            let Resource::PluginItem { id } = &response.items[0].resource else {
                panic!()
            };
            assert!(state.execute(action("save", id), &catalog).unwrap().refresh);
        }
        assert_eq!(state.store.phrases.len(), 1);
        let response = state.search("second", &catalog);
        let Resource::PluginItem { id } = &response.items[0].resource else {
            panic!()
        };
        assert_eq!(
            state
                .execute(action("copy", id), &catalog)
                .unwrap()
                .copy_text
                .as_deref(),
            Some("中文\nsecond")
        );
        assert_eq!(state.store.phrases.len(), 1);
        state.execute(action("delete", id), &catalog).unwrap();
        assert!(state.search("mail", &catalog).items.is_empty());
        assert!(state.execute(action("copy", id), &catalog).is_err());
        drop(state);
        assert!(Store::open(root.clone()).unwrap().phrases.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn many_long_snippets_stay_within_protocol_limit() {
        let root = std::env::temp_dir().join(format!("mytools-quick-size-{}", std::process::id()));
        let mut store = Store::open(root.clone()).unwrap();
        store.phrases = (0..50)
            .map(|index| Phrase {
                trigger: index.to_string(),
                content: "\0".repeat(16384),
            })
            .collect();
        let mut state = State {
            store,
            pending: None,
            revision: 0,
        };
        let response = state.search("", &Catalog::from_json("{}", "{}").unwrap());
        assert!(!response.items.is_empty());
        assert!(serde_json::to_vec(&response).unwrap().len() < 1_048_576);
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }
}
