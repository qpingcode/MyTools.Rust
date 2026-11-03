use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Explicit resource types drive host actions; display strings are never guessed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Resource {
    Url { url: String },
    File { path: PathBuf },
    Folder { path: PathBuf },
    Text { text: String },
    Application { path: PathBuf },
    PluginItem { id: String },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ActionId {
    Open,
    OpenInBrowser,
    CopyText,
    CopyPath,
    CopyName,
    Reveal,
    Launch,
    RunAsAdmin,
    Plugin,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ActionSpec {
    pub id: ActionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl From<ActionId> for ActionSpec {
    fn from(id: ActionId) -> Self {
        Self {
            id,
            title: None,
            command: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_and_actions_have_explicit_wire_types() {
        let resource: Resource =
            serde_json::from_str(r#"{"type":"url","url":"https://example.com"}"#).unwrap();
        assert_eq!(
            resource,
            Resource::Url {
                url: "https://example.com".into()
            }
        );
        let action: ActionSpec =
            serde_json::from_str(r#"{"id":"copy_text","title":"复制链接"}"#).unwrap();
        assert_eq!(action.id, ActionId::CopyText);
        assert_eq!(action.title.as_deref(), Some("复制链接"));
        assert!(serde_json::from_str::<ActionSpec>(r#"{"id":"unknown"}"#).is_err());
    }
}
