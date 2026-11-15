use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Host-owned layouts. Plugins provide data, never executable UI source.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum View {
    #[default]
    List,
    ListDetail,
    /// A multi-line input on the left and the first result on the right, for text tools
    /// such as translator. The input mirrors the query after the alias; editing it searches
    /// without changing the search box.
    Editor,
    /// A conversation with an input under it, e.g. Ask AI; see [`crate::chat`].
    Chat,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Preview {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub metadata: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_file: Option<PathBuf>,
}

/// A snapshot supplied by the launcher when an attachment command is executed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandArgument {
    Text { text: String },
    Files { paths: Vec<PathBuf> },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PluginActionRequest {
    pub command: String,
    /// The `item` the action declared; absent when it declared none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// For a command on the selection: whether the selection can be replaced, i.e. it is
    /// in a text field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<CommandArgument>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PluginEffect {
    /// Ask the host to copy text using its UI-thread clipboard integration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_text: Option<String>,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub paste: bool,
    #[serde(default)]
    pub message: String,
    /// Text to display in the launcher's main area after executing a command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_text: Option<String>,
    /// Open this plugin's search on this literal input (an empty string opens its initial view).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_query: Option<String>,
    /// Hide the launcher after the action succeeded (e.g. an external editor was opened).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dismiss: bool,
    /// Show the plugin's scope (its first alias), e.g. a conversation a command started.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
}
