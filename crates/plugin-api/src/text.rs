//! Helpers for text-oriented search plugins using the host's existing layouts.
use crate::{ActionId, ActionSpec, Preview, Resource, SearchItem};
use std::{io::Read, path::Path};

pub const INPUT_LIMIT: usize = 16 * 1024;

/// `@path` loads UTF-8 text, allowing multiline input without a custom editor.
/// Read at most one byte beyond the limit, including for files that grow.
pub fn input(value: &str) -> Result<String, String> {
    let value = if let Some(path) = value.strip_prefix('@') {
        let file = std::fs::File::open(Path::new(path.trim().trim_matches('"')))
            .map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        file.take(INPUT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        String::from_utf8(bytes).map_err(|error| error.to_string())?
    } else {
        value.to_owned()
    };
    if value.len() > INPUT_LIMIT {
        return Err(format!("{INPUT_LIMIT} bytes"));
    }
    Ok(value)
}

pub fn item(title: impl Into<String>, text: String, description: impl Into<String>) -> SearchItem {
    let title = title.into();
    SearchItem {
        intent_match: false,
        title: title.clone(),
        icon: None,
        description: description.into(),
        resource: Resource::Text { text: text.clone() },
        actions: None,
        preview: Some(Preview {
            title,
            text,
            ..Default::default()
        }),
    }
}

pub fn hint(title: impl Into<String>, description: impl Into<String>) -> SearchItem {
    let mut item = item(title, String::new(), description);
    item.actions = Some(vec![]);
    item
}

/// The result of a text tool in the `editor` view: the right side shows `shown` alone, and
/// Ctrl+Enter copies `copied` (the default action of a text result). `title` is not shown
/// there but names the result, e.g. for screen readers.
pub fn editor_result(title: impl Into<String>, copied: String, shown: String) -> SearchItem {
    let title = title.into();
    SearchItem {
        intent_match: false,
        title: title.clone(),
        icon: None,
        description: String::new(),
        resource: Resource::Text { text: copied },
        actions: None,
        preview: Some(Preview {
            title,
            text: shown,
            ..Default::default()
        }),
    }
}

/// A prompt or an error in the `editor` view: without a preview, the right side shows
/// `title` and `description`, e.g. "Translate / Ctrl+Enter sends the text". It has no
/// actions; add the one Ctrl+Enter should run.
pub fn editor_prompt(title: impl Into<String>, description: impl Into<String>) -> SearchItem {
    let mut item = item(title, String::new(), description);
    item.preview = None;
    item.actions = Some(vec![]);
    item
}

pub fn command(command: &str, title: String) -> ActionSpec {
    ActionSpec {
        id: ActionId::Plugin,
        command: Some(command.into()),
        title: Some(title),
        item: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_results_show_text_and_prompts_show_title_and_description() {
        let result = editor_result("Translation", "你好".into(), "你好\n\n/həˈləʊ/".into());
        assert_eq!(result.preview.as_ref().unwrap().text, "你好\n\n/həˈləʊ/");
        assert_eq!(
            result.resource,
            Resource::Text {
                text: "你好".into()
            }
        );
        assert!(result.actions.is_none(), "Ctrl+Enter copies by default");
        let prompt = editor_prompt("Translate", "Ctrl+Enter sends the text");
        assert!(prompt.preview.is_none());
        assert_eq!(prompt.description, "Ctrl+Enter sends the text");
        assert!(prompt.actions.is_some_and(|actions| actions.is_empty()));
    }

    #[test]
    fn literal_input_preserves_whitespace_and_rejects_oversized_text() {
        assert_eq!(input("  中文\n ").unwrap(), "  中文\n ");
        assert!(input(&"a".repeat(INPUT_LIMIT + 1)).is_err());
    }
}
