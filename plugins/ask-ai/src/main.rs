//! Ask AI: a conversation with a chat model (DeepSeek by default) in the `chat` view. Tab
//! in the main search box starts one with what was typed.
mod config;
mod session;

use mytools_plugin_api::{
    ActionId, PluginActionRequest, PluginEffect, Resource, SearchItem, SearchResponse,
    chat::{Chat, ChatMessage, Role},
    i18n::Catalog,
    text,
};
use session::{Ending, Model, Notes, Session};
use std::sync::Arc;

/// Asks the configured model, reading the configuration and key for every question.
struct DeepSeek {
    catalog: &'static Catalog,
}

impl Model for DeepSeek {
    fn answer(
        &self,
        history: &[ChatMessage],
        on_text: &mut dyn FnMut(&str) -> bool,
    ) -> Result<Ending, String> {
        let catalog = self.catalog;
        let config = config::Config::load()
            .map_err(|error| catalog.format("config.error", &[("error", &error)]))?;
        let provider = config.provider();
        if !provider.has_key() {
            return Err(catalog.get("key.missing").into());
        }
        let mut messages = vec![serde_json::json!({
            "role": "system", "content": config.system_prompt()
        })];
        messages.extend(
            history
                .iter()
                .filter(|message| !message.error && !message.text.trim().is_empty())
                .map(|message| {
                    let role = match message.role {
                        Role::User => "user",
                        Role::Assistant => "assistant",
                    };
                    serde_json::json!({"role": role, "content": message.text})
                }),
        );
        let body = serde_json::json!({"max_tokens": config.max_tokens, "messages": messages});
        let stopped = mytools_ai::stream(&provider, &body, |delta| match delta {
            mytools_ai::Delta::Text(text) => on_text(&text),
            mytools_ai::Delta::Reasoning(_) => on_text(""),
        })
        .map_err(|error| catalog.format("failed", &[("error", &error)]))?;
        Ok(match stopped {
            mytools_ai::Stopped::Finished => Ending::Finished,
            mytools_ai::Stopped::Length => Ending::Length,
            mytools_ai::Stopped::Cancelled => Ending::Cancelled,
        })
    }
}

/// The conversation, and one result whose actions Ctrl+K shows.
fn search(session: &Session, catalog: &Catalog) -> SearchResponse {
    let chat = session.chat();
    SearchResponse {
        items: vec![conversation_item(&chat, catalog)],
        chat: Some(Box::new(chat)),
        ..Default::default()
    }
}

fn conversation_item(chat: &Chat, catalog: &Catalog) -> SearchItem {
    let answer = chat
        .messages
        .iter()
        .rev()
        .filter(|message| message.role == Role::Assistant && !message.error)
        .map(|message| message.text.trim())
        .find(|text| !text.is_empty());
    let mut item = text::hint(catalog.get("conversation"), "");
    let mut actions = Vec::new();
    match answer {
        Some(answer) => {
            item.resource = Resource::Text {
                text: answer.into(),
            };
            actions.push(ActionId::CopyText.into());
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
    item.actions = Some(actions);
    item
}

fn run(session: &Session, action: PluginActionRequest, catalog: &Catalog) -> SearchResponse {
    let item = action.item.unwrap_or_default();
    let result = match action.command.as_str() {
        "send" => session
            .send(&item)
            .map_err(|()| catalog.get("busy").to_owned()),
        "new" => {
            session.start(&item);
            Ok(())
        }
        "stop" => {
            session.stop();
            Ok(())
        }
        _ => Err(catalog.get("missing").to_owned()),
    };
    match result {
        Ok(()) => SearchResponse {
            effect: Some(PluginEffect {
                refresh: true,
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
    mytools_plugin_api::crash::install();
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
    let session = Session::new(
        Arc::new(DeepSeek { catalog }),
        Arc::new(mytools_plugin_api::chat::push),
        notes,
    );
    mytools_plugin_api::serve_interactive(
        |_| search(&session, catalog),
        |action| run(&session, action, catalog),
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

    fn message(role: Role, text: &str) -> ChatMessage {
        ChatMessage {
            role,
            text: text.into(),
            ..Default::default()
        }
    }

    #[test]
    fn ctrl_k_copies_the_last_answer_and_starts_or_stops() {
        let catalog = catalog();
        let mut chat = Chat {
            id: "1".into(),
            messages: vec![],
        };
        let empty = conversation_item(&chat, &catalog);
        let commands = |item: &SearchItem| -> Vec<Option<String>> {
            item.actions
                .as_ref()
                .unwrap()
                .iter()
                .map(|action| action.command.clone())
                .collect()
        };
        assert_eq!(commands(&empty), [Some("new".to_owned())]);
        chat.messages = vec![
            message(Role::User, "q"),
            message(Role::Assistant, " answer "),
            message(Role::User, "q2"),
            ChatMessage {
                streaming: true,
                ..message(Role::Assistant, "")
            },
        ];
        let busy = conversation_item(&chat, &catalog);
        assert_eq!(
            busy.resource,
            Resource::Text {
                text: "answer".into()
            }
        );
        assert_eq!(busy.actions.as_ref().unwrap()[0].id, ActionId::CopyText);
        assert_eq!(
            commands(&busy)[1..],
            [Some("stop".to_owned()), Some("new".to_owned())]
        );
    }

    #[test]
    fn missing_key_is_explained_in_the_answer() {
        // SAFETY: tests in this binary do not read the environment concurrently.
        unsafe {
            std::env::set_var(mytools_ai::DEEPSEEK_API_KEY, "");
            std::env::set_var("MYTOOLS_ASK_AI_CONFIG", "missing-config.json");
        }
        let catalog: &'static Catalog = Box::leak(Box::new(catalog()));
        let result = DeepSeek { catalog }.answer(&[message(Role::User, "hi")], &mut |_| true);
        if mytools_ai::environment::variable(mytools_ai::DEEPSEEK_API_KEY).is_none() {
            assert_eq!(result, Err(catalog.get("key.missing").to_owned()));
        }
    }
}
