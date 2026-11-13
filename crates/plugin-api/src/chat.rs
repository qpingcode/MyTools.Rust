//! Data of the `chat` view: a conversation the host draws, with an input under it.
//!
//! The plugin answers every search in its scope with the whole conversation in
//! [`crate::SearchResponse::chat`]; the query does not matter. The host sends what the user
//! types as actions:
//!
//! - `{"command":"send","item":"<text>"}`: a new message in the current conversation;
//! - `{"command":"new","item":"<text>"}`: a new conversation, starting with the text unless
//!   it is empty (e.g. Tab in the main search box);
//! - `{"command":"stop"}`: stop the answer being written.
//!
//! Actions return at once; the plugin writes the answer in the background and sends each
//! change of a message with [`push`], which the host shows without searching again. The
//! single result the plugin returns next to the conversation carries the actions shown by
//! Ctrl+K, such as copying the last answer.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    User,
    Assistant,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    /// Plain text from the user; Markdown from the assistant.
    pub text: String,
    /// The answer is still being written.
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub streaming: bool,
    /// The answer failed; `text` says why.
    #[serde(default, skip_serializing_if = "crate::is_false")]
    pub error: bool,
    /// What the model thought before answering, for models that share it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning: String,
    /// Text the question is about, e.g. the selection it was asked on; shown folded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<String>,
    /// What was done to answer, e.g. web searches, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<ChatStep>,
    /// Pages the answer cites as `[1]`, `[2]`… in this order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ChatSource>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepState {
    #[default]
    Running,
    Done,
    Failed,
}

/// One thing done for an answer, e.g. "Searched: rust slint · 5 results".
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatStep {
    pub text: String,
    #[serde(default)]
    pub state: StepState,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatSource {
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Chat {
    /// Changes with each new conversation, so late updates of an old one are dropped.
    pub id: String,
    pub messages: Vec<ChatMessage>,
    /// Text that goes with the next question, shown above the input until it is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<String>,
}

impl Chat {
    /// Whether an answer is being written, so the input stops it rather than sending.
    pub fn busy(&self) -> bool {
        self.messages.iter().any(|message| message.streaming)
    }
}

/// One message of a conversation, new or changed, sent between requests.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChatUpdate {
    /// The conversation's [`Chat::id`].
    pub chat: String,
    /// The message's position; the length of the conversation adds a message.
    pub index: usize,
    pub message: ChatMessage,
}

/// Sends `update` to the host now, from any thread, e.g. while an answer streams in.
pub fn push(update: ChatUpdate) {
    crate::transport::push(crate::SearchResponse {
        update: Some(Box::new(update)),
        ..Default::default()
    });
}
