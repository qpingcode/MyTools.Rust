//! The `chat` view: shows a plugin's conversation, sends the input under it as the next
//! question, applies the answer as the plugin streams it, and opens it with Tab.
#[path = "chat/markdown.rs"]
mod markdown;

use crate::{
    ChatBlock, ChatMessageData, ChatSource, ChatStep, MainWindow, app::State, i18n,
    search::QuerySender,
};
use mytools_core::{Action, ActionEffect, ChatSession, SearchResults};
use mytools_plugin_api::chat::{ChatMessage, ChatUpdate, Role, StepState};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, rc::Rc};

#[derive(Default)]
pub struct ChatState {
    session: Option<ChatSession>,
    messages: Rc<VecModel<ChatMessageData>>,
    /// Tab was pressed: start a conversation with this once the plugin answers.
    pending: Option<String>,
}

/// A channel for the updates plugins push, to hand to the catalog, and its receiving end.
pub fn updates() -> (
    mytools_core::ChatUpdates,
    async_channel::Receiver<(String, ChatUpdate)>,
) {
    let (sender, receiver) = async_channel::unbounded();
    let updates: mytools_core::ChatUpdates = std::sync::Arc::new(move |plugin, update| {
        let _ = sender.send_blocking((plugin.to_owned(), update));
    });
    (updates, receiver)
}

pub fn wire(ui: &MainWindow, state: &Rc<RefCell<State>>, requests: &QuerySender) {
    ui.set_chat_messages(ModelRc::from(state.borrow().chat.messages.clone()));
    let weak = ui.as_weak();
    let tab_state = state.clone();
    let tab_requests = requests.clone();
    ui.on_tab_pressed(move || {
        let Some(ui) = weak.upgrade() else {
            return false;
        };
        let query = ui.get_query();
        if tab_requests.scope_prefix(&query).is_some() {
            return false;
        }
        let Some(scope) = tab_requests.tab_scope() else {
            return false;
        };
        tab_state.borrow_mut().chat.pending = Some(query.trim().to_owned());
        ui.set_query(scope.clone().into());
        ui.invoke_search(scope.into());
        // The cursor stays where it was in the replaced text; the chat input takes over
        // once the conversation shows.
        ui.invoke_focus_search_at_end();
        true
    });
    let weak = ui.as_weak();
    let send_state = state.clone();
    ui.on_chat_send(move |text| {
        if let Some(ui) = weak.upgrade() {
            send(&ui, &send_state.borrow(), &text);
        }
    });
    let weak = ui.as_weak();
    let stop_state = state.clone();
    ui.on_chat_stop(move || {
        if let Some(ui) = weak.upgrade() {
            stop(&ui, &stop_state.borrow());
        }
    });
    let weak = ui.as_weak();
    let detach_state = state.clone();
    ui.on_chat_detach(move || {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(session) = &detach_state.borrow().chat.session {
            run(&ui, session.detach());
        }
    });
    let weak = ui.as_weak();
    ui.on_copy_text(move |text| {
        let copied = arboard::Clipboard::new().and_then(|mut board| board.set_text(text.as_str()));
        if let Some(ui) = weak.upgrade() {
            status(
                &ui,
                copied.map(|()| i18n::t("chat.copied")).map_err(Into::into),
            );
        }
    });
    let weak = ui.as_weak();
    ui.on_open_link(move |url| {
        let opened = mytools_core::open_url(&url);
        if let Some(ui) = weak.upgrade()
            && let Err(error) = opened
        {
            status(&ui, Err(error));
        }
    });
}

/// Shows the conversation of a chat search, and starts the one Tab asked for.
pub fn apply(ui: &MainWindow, state: &mut State, results: &mut SearchResults) {
    let chat_view = results.view == mytools_plugin_api::View::Chat;
    // Typing goes to the conversation's input as soon as it shows.
    let appeared = chat_view && !ui.get_chat_view();
    ui.set_chat_view(chat_view);
    if appeared {
        ui.invoke_focus_chat();
    }
    let chat = &mut state.chat;
    match results.chat.take() {
        Some(session) => {
            if let Some(text) = chat.pending.take() {
                // The previous conversation would flash until the new one arrives.
                chat.messages.set_vec(Vec::new());
                run(ui, session.start(&text));
                chat.session = Some(*session);
                ui.invoke_focus_chat();
                return;
            }
            show(ui, chat, *session);
        }
        // The plugin may still be starting; a pending question waits for it.
        None if chat_view => {}
        None => {
            chat.pending = None;
            if chat.session.take().is_some() {
                chat.messages.set_vec(Vec::new());
            }
        }
    }
}

/// In the chat view the primary button sends, or stops an answer being written.
pub fn sync_primary(ui: &MainWindow, state: &State) {
    if ui.get_chat_view() {
        ui.set_primary_available(true);
        ui.set_primary_title(primary_title(&state.chat).into());
    }
}

fn primary_title(chat: &ChatState) -> String {
    if chat
        .session
        .as_ref()
        .is_some_and(|session| session.chat.busy())
    {
        i18n::t("chat.stop")
    } else {
        i18n::t("chat.send")
    }
}

/// Replaces the shown conversation, touching only the messages that changed.
fn show(ui: &MainWindow, chat: &mut ChatState, session: ChatSession) {
    let same = chat
        .session
        .as_ref()
        .is_some_and(|shown| shown.chat.id == session.chat.id);
    let old: &[ChatMessage] = match (&chat.session, same) {
        (Some(shown), true) => &shown.chat.messages,
        _ => &[],
    };
    let changed = !same
        || old.len() != session.chat.messages.len()
        || old.iter().zip(&session.chat.messages).any(|(a, b)| a != b);
    if changed {
        if !same {
            chat.messages.set_vec(Vec::new());
        }
        for (index, message) in session.chat.messages.iter().enumerate() {
            if old.get(index) == Some(message) && index < chat.messages.row_count() {
                continue;
            }
            put(&chat.messages, index, message);
        }
        while chat.messages.row_count() > session.chat.messages.len() {
            chat.messages.remove(chat.messages.row_count() - 1);
        }
    }
    let session_grew = same && session.chat.messages.len() > old.len();
    ui.set_chat_busy(session.chat.busy());
    ui.set_chat_attachment(session.chat.attachment.clone().unwrap_or_default().into());
    chat.session = Some(session);
    if changed {
        // A new question or conversation shows its end even after scrolling up.
        ui.invoke_chat_scroll_to_end(!same || session_grew);
    }
}

fn put(model: &VecModel<ChatMessageData>, index: usize, message: &ChatMessage) {
    let data = message_data(message);
    if index < model.row_count() {
        model.set_row_data(index, data);
    } else {
        model.push(data);
    }
}

/// Applies a message the plugin pushed while the conversation is shown. Returns whether
/// to search again once `state` is released: a finished answer changes what Ctrl+K
/// offers, such as copying it.
pub fn update(ui: &MainWindow, state: &mut State, plugin: &str, update: ChatUpdate) -> bool {
    let chat = &mut state.chat;
    let Some(session) = chat.session.as_mut() else {
        return false;
    };
    if session.plugin != plugin || session.chat.id != update.chat {
        return false;
    }
    let messages = &mut session.chat.messages;
    match update.index.cmp(&messages.len()) {
        std::cmp::Ordering::Less => messages[update.index] = update.message.clone(),
        std::cmp::Ordering::Equal => messages.push(update.message.clone()),
        std::cmp::Ordering::Greater => return false,
    }
    put(&chat.messages, update.index, &update.message);
    let busy = session.chat.busy();
    ui.set_chat_busy(busy);
    ui.invoke_chat_scroll_to_end(false);
    if ui.get_chat_view() {
        ui.set_primary_title(primary_title(chat).into());
    }
    !update.message.streaming && !busy
}

fn message_data(message: &ChatMessage) -> ChatMessageData {
    let blocks: Vec<markdown::Block> = if message.role == Role::User || message.error {
        vec![markdown::Block {
            kind: markdown::Kind::Text,
            text: message.text.trim().into(),
            language: String::new(),
            rows: Vec::new(),
        }]
    } else {
        markdown::blocks(&message.text)
    };
    let blocks: Vec<ChatBlock> = blocks
        .into_iter()
        .filter(|block| !block.text.is_empty() || !block.rows.is_empty())
        .map(|block| {
            let widths = markdown::column_shares(&block.rows);
            let rows: Vec<ModelRc<SharedString>> = block
                .rows
                .into_iter()
                .map(|row| {
                    let cells: Vec<SharedString> = row.into_iter().map(Into::into).collect();
                    ModelRc::new(VecModel::from(cells))
                })
                .collect();
            ChatBlock {
                kind: block.kind as i32,
                plain: block.text.into(),
                language: block.language.into(),
                rows: ModelRc::new(VecModel::from(rows)),
                widths: ModelRc::new(VecModel::from(widths)),
            }
        })
        .collect();
    let steps: Vec<ChatStep> = message
        .steps
        .iter()
        .map(|step| ChatStep {
            text: step.text.clone().into(),
            state: match step.state {
                StepState::Running => 0,
                StepState::Done => 1,
                StepState::Failed => 2,
            },
        })
        .collect();
    let sources: Vec<ChatSource> = message
        .sources
        .iter()
        .enumerate()
        .map(|(index, source)| ChatSource {
            number: index as i32 + 1,
            title: source.title.clone().into(),
            url: source.url.clone().into(),
            host: host(&source.url).into(),
        })
        .collect();
    ChatMessageData {
        user: message.role == Role::User,
        blocks: ModelRc::new(VecModel::from(blocks)),
        streaming: message.streaming,
        error: message.error,
        reasoning: message.reasoning.trim().into(),
        attachment: message.attachment.clone().unwrap_or_default().into(),
        steps: ModelRc::new(VecModel::from(steps)),
        sources: ModelRc::new(VecModel::from(sources)),
    }
}

/// `example.com` of `https://www.example.com/page`.
fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.strip_prefix("www.").unwrap_or(host)
}

/// Sends the input as the next question; while an answer is written, an empty input
/// stops it instead.
fn send(ui: &MainWindow, state: &State, text: &str) {
    let Some(session) = &state.chat.session else {
        return;
    };
    if session.chat.busy() {
        if text.trim().is_empty() {
            run(ui, session.stop());
        } else {
            status(ui, Err(anyhow::anyhow!(i18n::t("chat.busy"))));
        }
        return;
    }
    // The attached text alone is a question too.
    if text.trim().is_empty() && session.chat.attachment.is_none() {
        return;
    }
    run(ui, session.send(text));
    ui.set_editor_text(Default::default());
    if let Some(prefix) = scope_of(&ui.get_query()) {
        ui.set_query(prefix.into());
    }
}

/// The query up to and including the alias, e.g. `"ai "` of `"ai hello"`.
fn scope_of(query: &str) -> Option<&str> {
    let start = query.len() - query.trim_start().len();
    let end = start + query[start..].find(char::is_whitespace)?;
    let rest = query[end..].trim_start();
    Some(&query[..query.len() - rest.len()])
}

fn stop(ui: &MainWindow, state: &State) {
    if let Some(session) = &state.chat.session
        && session.chat.busy()
    {
        run(ui, session.stop());
    }
}

/// In the chat view, Esc stops an answer being written before it closes the window;
/// `true` when it stopped one.
pub fn dismiss(ui: &MainWindow, state: &State) -> bool {
    let busy = ui.get_chat_view()
        && state
            .chat
            .session
            .as_ref()
            .is_some_and(|session| session.chat.busy());
    if busy {
        stop(ui, state);
    }
    busy
}

/// The status bar's primary button in the chat view: send, or stop while answering.
pub fn execute(ui: &MainWindow, state: &State) -> bool {
    if !ui.get_chat_view() {
        return false;
    }
    if !dismiss(ui, state) {
        send(ui, state, &ui.get_editor_text());
    }
    true
}

/// Runs a chat action off the UI thread; the plugin answers at once and asks for the
/// conversation to be shown again.
fn run(ui: &MainWindow, action: Action) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = action.execute();
        let _ = weak.upgrade_in_event_loop(move |ui| match result {
            Ok(ActionEffect::Plugin(effect)) if effect.refresh => ui.invoke_refresh(),
            Ok(_) => {}
            Err(error) => status(&ui, Err(error)),
        });
    });
}

fn status(ui: &MainWindow, result: anyhow::Result<String>) {
    match result {
        Ok(message) => {
            ui.set_failed(false);
            ui.set_status(message.into());
        }
        Err(error) => {
            ui.set_failed(true);
            ui.set_status(
                i18n::format("status.failed", &[("error", &format!("{error:#}"))]).into(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_drop_the_scheme_path_and_www() {
        assert_eq!(host("https://www.example.com/a?b"), "example.com");
        assert_eq!(host("http://docs.rs"), "docs.rs");
    }

    #[test]
    fn scope_keeps_the_alias_and_its_whitespace() {
        assert_eq!(scope_of("ai hello there"), Some("ai "));
        assert_eq!(scope_of(" ai  "), Some(" ai  "));
        assert_eq!(scope_of("ai"), None);
    }

    #[test]
    fn answers_become_blocks_and_questions_stay_plain() {
        let question = message_data(&ChatMessage {
            role: Role::User,
            text: "## not a heading".into(),
            ..Default::default()
        });
        assert!(question.user);
        assert_eq!(question.blocks.row_count(), 1);
        assert_eq!(
            question.blocks.row_data(0).unwrap().plain,
            "## not a heading"
        );
        let answer = message_data(&ChatMessage {
            role: Role::Assistant,
            text: "Hi **there**\n\n```sh\nls\n```".into(),
            streaming: true,
            ..Default::default()
        });
        assert_eq!(answer.blocks.row_count(), 2);
        assert_eq!(answer.blocks.row_data(1).unwrap().kind, 2);
        assert_eq!(answer.blocks.row_data(1).unwrap().plain, "ls");
        assert!(answer.streaming);
    }
}
