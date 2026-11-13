//! Ask AI: a conversation with a chat model (DeepSeek by default) in the `chat` view. Tab
//! in the main search box starts one with what was typed; a command starts one on the
//! selected text. The model can search the web and call the tools of MCP servers.
mod answer;
mod config;
mod history;
mod session;
mod tools;
mod web;

use mytools_plugin_api::{
    ActionId, ActionSpec, PluginActionRequest, PluginEffect, Resource, SearchItem, SearchResponse,
    chat::{Chat, Role},
    i18n::Catalog,
    text,
};
use session::{Notes, Session};
use std::sync::{Arc, Mutex};

/// Conversations offered by Ctrl+K besides the current one.
const RECENT: usize = 8;

/// The conversation, and one result whose actions Ctrl+K shows.
fn search(session: &Session, catalog: &Catalog) -> SearchResponse {
    let chat = session.chat();
    let config = config::Config::load().unwrap_or_default();
    SearchResponse {
        items: vec![conversation_item(session, &chat, &config, catalog)],
        chat: Some(Box::new(chat)),
        ..Default::default()
    }
}

/// The last complete answer.
fn last_answer(chat: &Chat) -> Option<&str> {
    chat.messages
        .iter()
        .rev()
        .filter(|message| message.role == Role::Assistant && !message.error)
        .filter(|message| !message.streaming)
        .map(|message| message.text.trim())
        .find(|text| !text.is_empty())
}

fn conversation_item(
    session: &Session,
    chat: &Chat,
    config: &config::Config,
    catalog: &Catalog,
) -> SearchItem {
    let model = session.model().unwrap_or_else(|| config.model.clone());
    // The status bar shows the model next to the plugin's name.
    let mut item = text::hint(catalog.get("conversation"), model.clone());
    let mut actions: Vec<ActionSpec> = Vec::new();
    match last_answer(chat) {
        Some(answer) => {
            item.resource = Resource::Text {
                text: answer.into(),
            };
            actions.push(ActionSpec {
                title: Some(catalog.get("copy").into()),
                ..ActionId::CopyText.into()
            });
            if session.replaceable() {
                actions.push(text::command("paste", catalog.get("paste").into()).with_item(answer));
            }
        }
        None => {
            item.resource = Resource::PluginItem {
                id: chat.id.clone(),
            }
        }
    }
    if chat.busy() {
        actions.push(text::command("stop", catalog.get("stop").into()));
    }
    actions.push(text::command("new", catalog.get("new").into()).with_item(""));
    if chat.attachment.is_some() {
        actions.push(text::command("detach", catalog.get("detach").into()));
    }
    for other in config
        .all_models()
        .into_iter()
        .filter(|other| *other != model)
    {
        let title = catalog.format("model", &[("model", &other)]);
        actions.push(text::command("model", title).with_item(other));
    }
    let recent = session.history().recent(RECENT + 1);
    for (id, title) in recent
        .into_iter()
        .filter(|(id, _)| *id != chat.id)
        .take(RECENT)
    {
        let title = catalog.format("open", &[("title", &title)]);
        actions.push(text::command("open", title).with_item(id));
    }
    item.actions = Some(actions);
    item
}

fn run(session: &Session, action: PluginActionRequest, catalog: &Catalog) -> SearchResponse {
    let item = action.item.unwrap_or_default();
    let refresh = PluginEffect {
        refresh: true,
        ..Default::default()
    };
    let result = match action.command.as_str() {
        "send" => session
            .send(&item)
            .map(|()| refresh)
            .map_err(|()| catalog.get("busy").to_owned()),
        "new" => {
            session.start(&item);
            Ok(refresh)
        }
        "stop" => {
            session.stop();
            Ok(refresh)
        }
        "detach" => {
            session.detach();
            Ok(refresh)
        }
        "open" => {
            session.open(&item);
            Ok(refresh)
        }
        "model" => {
            let config = config::Config::load().unwrap_or_default();
            session.use_model((item != config.model).then_some(item));
            Ok(refresh)
        }
        "paste" => Ok(PluginEffect {
            copy_text: Some(item),
            paste: true,
            ..Default::default()
        }),
        "ask-selection" => {
            session.start_on(&item, action.editable == Some(true));
            Ok(PluginEffect {
                open: true,
                ..Default::default()
            })
        }
        _ => Err(catalog.get("missing").to_owned()),
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
}

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
    if std::env::args().any(|argument| argument == web::ARGUMENT) {
        return web::serve();
    }
    let catalog: &'static Catalog = Box::leak(Box::new(
        Catalog::for_language(
            include_str!("../locales/en.json"),
            include_str!("../locales/zh-CN.json"),
        )
        .unwrap(),
    ));
    let notes = Notes {
        length: catalog.get("length").into(),
        stopped: catalog.get("stopped").into(),
    };
    let config = config::Config::load().unwrap_or_default();
    let model = answer::DeepSeek {
        catalog,
        toolbox: Mutex::new(tools::Toolbox::default()),
    };
    let session = Session::new(
        Arc::new(model),
        Arc::new(mytools_plugin_api::chat::push),
        notes,
        history::History::open(config.history),
    );
    mytools_plugin_api::serve_interactive(
        |_| search(&session, catalog),
        |action| run(&session, action, catalog),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mytools_plugin_api::chat::ChatMessage;
    use session::{Ending, Event, Model};

    fn catalog() -> Catalog {
        Catalog::from_json(
            include_str!("../locales/en.json"),
            include_str!("../locales/en.json"),
        )
        .unwrap()
    }

    struct Silent;
    impl Model for Silent {
        fn answer(
            &self,
            _: &[ChatMessage],
            _: Option<&str>,
            _: &mut dyn FnMut(Event) -> bool,
        ) -> Result<Ending, String> {
            Ok(Ending::Finished)
        }
    }

    fn session() -> Session {
        Session::new(
            Arc::new(Silent),
            Arc::new(|_| {}),
            Notes {
                length: String::new(),
                stopped: String::new(),
            },
            history::History::temporary(),
        )
    }

    fn message(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            role,
            text: text.into(),
            ..Default::default()
        }
    }

    fn commands(item: &SearchItem) -> Vec<Option<String>> {
        item.actions
            .as_ref()
            .unwrap()
            .iter()
            .map(|action| action.command.clone())
            .collect()
    }

    #[test]
    fn ctrl_k_copies_the_last_answer_and_starts_stops_or_switches() {
        let catalog = catalog();
        let session = session();
        let config = config::Config {
            models: vec!["deepseek-flash".into(), "other".into()],
            ..Default::default()
        };
        let mut chat = Chat {
            id: "1".into(),
            ..Default::default()
        };
        let empty = conversation_item(&session, &chat, &config, &catalog);
        assert_eq!(empty.description, "deepseek-flash");
        assert_eq!(commands(&empty), [Some("new".into()), Some("model".into())]);
        chat.messages = vec![
            message(Role::User, "q"),
            message(Role::Assistant, " answer "),
            message(Role::User, "q2"),
            ChatMessage {
                streaming: true,
                ..message(Role::Assistant, "partial")
            },
        ];
        chat.attachment = Some("text".into());
        let busy = conversation_item(&session, &chat, &config, &catalog);
        assert_eq!(
            busy.resource,
            Resource::Text {
                text: "answer".into()
            }
        );
        assert_eq!(busy.actions.as_ref().unwrap()[0].id, ActionId::CopyText);
        assert_eq!(
            commands(&busy)[1..],
            [
                Some("stop".into()),
                Some("new".into()),
                Some("detach".into()),
                Some("model".into())
            ]
        );
    }

    #[test]
    fn asking_about_a_selection_opens_the_conversation() {
        let catalog = catalog();
        let session = session();
        let response = run(
            &session,
            PluginActionRequest {
                command: "ask-selection".into(),
                item: Some("Hello".into()),
                editable: Some(true),
            },
            &catalog,
        );
        assert!(response.effect.unwrap().open);
        assert_eq!(session.chat().attachment.as_deref(), Some("Hello"));
        assert!(session.replaceable());
        let paste = run(
            &session,
            PluginActionRequest {
                command: "paste".into(),
                item: Some("Hi".into()),
                editable: None,
            },
            &catalog,
        )
        .effect
        .unwrap();
        assert_eq!(
            (paste.copy_text.as_deref(), paste.paste),
            (Some("Hi"), true)
        );
    }
}
