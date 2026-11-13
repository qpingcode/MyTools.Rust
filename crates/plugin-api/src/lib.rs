use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

mod transport;
pub use transport::{cancelled, deadline, on_init, progress};

mod actions;
pub use actions::{ActionId, ActionSpec, Resource};
pub use icons::Icon;
pub use views::{PluginActionRequest, PluginEffect, Preview, View};
pub mod chat;
pub mod views;

#[derive(Deserialize, Serialize)]
pub struct SearchRequest {
    #[serde(default)]
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<PluginActionRequest>,
    /// How long the host waits for this search; see [`deadline`].
    #[serde(default, rename = "timeoutMs", skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Heartbeat: answered at once with `pong` by the transport, never by the plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping: Option<u64>,
    /// The host's first request to a new process; answered with `ready` (see [`on_init`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init: Option<InitRequest>,
    /// The user picked this plugin by its alias, so every query is meant for it. Absent in
    /// global search, where a plugin should answer only what it recognizes.
    #[serde(default, skip_serializing_if = "is_false")]
    pub scoped: bool,
}

/// Sent once as `{"init":{}}` before any search or action.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct InitRequest {}

impl SearchRequest {
    pub fn init() -> Self {
        Self {
            query: String::new(),
            action: None,
            timeout_ms: None,
            ping: None,
            init: Some(InitRequest {}),
            scoped: false,
        }
    }
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
    /// Heartbeat reply carrying the `ping` value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pong: Option<u64>,
    /// With `pong`: how long the current request has been running, 0 when idle.
    #[serde(default, rename = "busyMs", skip_serializing_if = "Option::is_none")]
    pub busy_ms: Option<u64>,
    /// Answer to `init`: the plugin can take searches now. `error` instead means it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
    /// A status line sent while `init` runs; not a response to any request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
    /// The conversation of a plugin with the `chat` view; see [`chat`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<Box<chat::Chat>>,
    /// A changed chat message sent between requests; not a response to any request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Box<chat::ChatUpdate>>,
}

/// A search plugin stays alive and handles one JSON request per line.
/// stdout is reserved for responses; diagnostics should use stderr.
/// Heartbeats and cancellation are handled by the transport (see [`cancelled`]).
pub fn serve(search: impl Fn(&str) -> Vec<SearchItem>) -> io::Result<()> {
    serve_io(io::BufReader::new(io::stdin()), io::stdout(), search)
}

fn serve_io(
    input: impl BufRead + Send + 'static,
    output: impl Write + Send + 'static,
    search: impl Fn(&str) -> Vec<SearchItem>,
) -> io::Result<()> {
    transport::run(input, output, |request| SearchResponse {
        items: search(&request.query),
        ..Default::default()
    })
}

pub mod commands;
pub mod crash;
pub mod dirs;
pub mod highlight;
pub mod hotkeys;
pub mod i18n;
pub mod icons;
pub mod log;
pub mod matching;
pub mod paths;
pub mod plugin_settings;
pub mod plugin_status;
pub mod text;

fn is_false(value: &bool) -> bool {
    !value
}

/// A resident plugin may handle commands using the same ordered JSON transport.
pub fn serve_interactive(
    search: impl Fn(&str) -> SearchResponse,
    action: impl Fn(PluginActionRequest) -> SearchResponse,
) -> io::Result<()> {
    transport::run(io::BufReader::new(io::stdin()), io::stdout(), |request| {
        transport::dispatch(request, &search, &action)
    })
}

/// Like [`serve_interactive`], for a search that needs the whole request, e.g. `scoped` to
/// tell input after the plugin's alias from global search.
pub fn serve_requests(
    search: impl Fn(&SearchRequest) -> SearchResponse,
    action: impl Fn(PluginActionRequest) -> SearchResponse,
) -> io::Result<()> {
    transport::run(
        io::BufReader::new(io::stdin()),
        io::stdout(),
        |mut request| match request.action.take() {
            Some(command) => action(command),
            None => search(&request),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SharedSink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl Write for SharedSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn handles_multiple_queries_in_one_session() {
        // Every request gets one response, in order. A search queued behind a newer one
        // is cancelled (empty), so the newest query is sent last.
        let input = b"{\"query\":\"\"}\n{\"query\":\"1 + 1\"}\n";
        let sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        serve_io(
            std::io::Cursor::new(input.to_vec()),
            SharedSink(sink.clone()),
            |query| {
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
            },
        )
        .unwrap();
        let output = sink.lock().unwrap();
        let lines: Vec<SearchResponse> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].items.is_empty());
        assert_eq!(lines[1].items[0].title, "1 + 1");
        assert_eq!(
            lines[1].items[0].resource,
            Resource::Text { text: "2".into() }
        );
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
