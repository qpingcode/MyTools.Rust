//! Conversations of plugins with the `chat` view: what the host shows and the actions it
//! sends for the input under it (see `mytools_plugin_api::chat`).
use crate::{actions::Action, search_process::SearchProcess};
use mytools_plugin_api::chat::{Chat, ChatUpdate};
use std::sync::Arc;

/// Receives chat updates as plugins send them, with the plugin's id; called on a reader
/// thread, so it should only hand them over to the UI.
pub type ChatUpdates = Arc<dyn Fn(&str, ChatUpdate) + Send + Sync>;

/// A conversation as a scoped search returned it, with the plugin to send input to.
#[derive(Clone, Debug)]
pub struct ChatSession {
    pub chat: Chat,
    /// The plugin's id, which updates carry.
    pub plugin: String,
    process: Arc<SearchProcess>,
}

impl ChatSession {
    pub(crate) fn new(chat: Chat, plugin: &str, process: &Arc<SearchProcess>) -> Self {
        Self {
            chat,
            plugin: plugin.into(),
            process: process.clone(),
        }
    }

    fn action(&self, command: &str, item: &str) -> Action {
        Action::command(
            command,
            command,
            &"builtin:send".into(),
            &self.process,
            None,
        )
        .with_item(item.into())
    }

    /// Sends `text` as the next message of this conversation.
    pub fn send(&self, text: &str) -> Action {
        self.action("send", text)
    }

    /// Starts a new conversation, with `text` as its first message unless it is empty.
    pub fn start(&self, text: &str) -> Action {
        self.action("new", text)
    }

    /// Stops the answer being written.
    pub fn stop(&self) -> Action {
        self.action("stop", "")
    }
}
