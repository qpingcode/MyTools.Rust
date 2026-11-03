use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

mod actions;
pub use actions::{ActionId, ActionSpec, Resource};
pub use icons::Icon;
pub use views::{PluginActionRequest, PluginEffect, Preview, View};
pub mod views;

#[derive(Deserialize, Serialize)]
pub struct SearchRequest {
    #[serde(default)]
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<PluginActionRequest>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchItem {
    /// Computed answer to an explicit expression; hosts choose which plugins to trust.
    #[serde(default, skip_serializing_if = "is_false")]
    pub intent_match: bool,
    pub title: String,
    /// Builtin/text string or `{ "native": "/absolute/path" }`. Omitted: inherit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Icon>,
    #[serde(default)]
    pub description: String,
    pub resource: Resource,
    /// Omitted: host defaults. Present (including []): exact ordered action list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ActionSpec>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<Preview>,
}

#[derive(Default, Deserialize, Serialize)]
pub struct SearchResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(default)]
    pub items: Vec<SearchItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<PluginEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A search plugin stays alive and handles one JSON request per line.
/// stdout is reserved for responses; diagnostics should use stderr.
pub fn serve(search: impl Fn(&str) -> Vec<SearchItem>) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve_io(stdin.lock(), stdout.lock(), search)
}

fn serve_io(
    input: impl BufRead,
    mut output: impl Write,
    search: impl Fn(&str) -> Vec<SearchItem>,
) -> io::Result<()> {
    for line in input.lines() {
        let request: SearchRequest = serde_json::from_str(&line?)?;
        let response = SearchResponse {
            items: search(&request.query),
            ..Default::default()
        };
        serde_json::to_writer(&mut output, &response)?;
        writeln!(output)?;
        output.flush()?;
    }
    Ok(())
}

pub mod i18n;
pub mod icons;
pub mod matching;
pub mod paths;
pub mod text;

fn is_false(value: &bool) -> bool {
    !value
}

/// A resident plugin may handle commands using the same ordered JSON transport.
pub fn serve_interactive(
    search: impl Fn(&str) -> SearchResponse,
    action: impl Fn(PluginActionRequest) -> SearchResponse,
) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let request: SearchRequest = serde_json::from_str(&line?)?;
        let response = match request.action {
            Some(command) => action(command),
            None => search(&request.query),
        };
        serde_json::to_writer(&mut stdout, &response)?;
        writeln!(stdout)?;
        stdout.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_multiple_queries_in_one_session() {
        let input = b"{\"query\":\"1 + 1\"}\n{\"query\":\"\"}\n";
        let mut output = Vec::new();
        serve_io(&input[..], &mut output, |query| {
            if query.is_empty() {
                vec![]
            } else {
                vec![SearchItem {
                    intent_match: false,
                    title: query.into(),
                    icon: None,
                    description: String::new(),
                    resource: Resource::Text { text: "2".into() },
                    actions: None,
                    preview: None,
                }]
            }
        })
        .unwrap();
        let lines: Vec<SearchResponse> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].items[0].title, "1 + 1");
        assert_eq!(
            lines[0].items[0].resource,
            Resource::Text { text: "2".into() }
        );
        assert!(lines[1].items.is_empty());
    }

    #[test]
    fn result_icons_are_optional_and_round_trip() {
        let mut item: SearchItem =
            serde_json::from_str(r#"{"title":"2","resource":{"type":"text","text":"2"}}"#).unwrap();
        assert!(item.icon.is_none());
        assert!(serde_json::to_value(&item).unwrap().get("icon").is_none());
        item.icon = Some("builtin:calculator".into());
        let json = serde_json::to_string(&item).unwrap();
        let decoded: SearchItem = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.icon, Some("builtin:calculator".into()));
    }

    #[test]
    fn distinguishes_default_actions_from_explicit_empty_override() {
        let default: SearchItem =
            serde_json::from_str(r#"{"title":"2","resource":{"type":"text","text":"2"}}"#).unwrap();
        let empty: SearchItem = serde_json::from_str(
            r#"{"title":"2","resource":{"type":"text","text":"2"},"actions":[]}"#,
        )
        .unwrap();
        assert!(default.actions.is_none());
        assert!(empty.actions.unwrap().is_empty());
    }
}
