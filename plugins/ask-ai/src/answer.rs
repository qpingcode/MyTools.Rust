//! Answers with DeepSeek (or another OpenAI-compatible model), calling tools such as the web
//! search when the model asks for them.
use crate::{
    config::{self, Config},
    session::{Ending, Event, Model},
    tools::{Offered, Toolbox},
};
use mytools_ai::{Delta, Stopped, ToolCall, mcp::ServersFile};
use mytools_plugin_api::{
    chat::{ChatMessage, ChatSource, ChatStep, Role, StepState},
    i18n::Catalog,
};
use serde_json::{Value, json};
use std::sync::Mutex;

/// Rounds of tool calls before the model must answer.
const MAX_ROUNDS: usize = 6;

pub struct DeepSeek {
    pub catalog: &'static Catalog,
    pub toolbox: Mutex<Toolbox>,
}

impl Model for DeepSeek {
    fn answer(
        &self,
        history: &[ChatMessage],
        model: Option<&str>,
        on_event: &mut dyn FnMut(Event) -> bool,
    ) -> Result<Ending, String> {
        let catalog = self.catalog;
        let config =
            Config::load().map_err(|error| catalog.format("config.error", &[("error", &error)]))?;
        let provider = config.provider(model);
        if !provider.has_key() {
            return Err(catalog.get("key.missing").into());
        }
        let servers: ServersFile = config::read("MYTOOLS_ASK_AI_MCP", "mcp.json")
            .map_err(|error| catalog.format("mcp.error", &[("error", &error)]))?;
        let mut toolbox = self.toolbox.lock().unwrap();
        toolbox.configure(config.web_search, &servers);
        let offered = toolbox.offered();
        for problem in toolbox.problems.drain(..) {
            mytools_plugin_api::log::warn("mcp.start", problem);
        }
        let mut conversation = Conversation {
            catalog,
            toolbox: &mut toolbox,
            offered: &offered,
            sources: Vec::new(),
            steps: 0,
            on_event,
        };
        let mut messages = vec![json!({
            "role": "system", "content": system_prompt(&config, !offered.is_empty()),
        })];
        messages.extend(history.iter().filter_map(model_message));
        let tools: Vec<Value> = offered.iter().map(function).collect();
        let mut wrote = false;
        for round in 0..=MAX_ROUNDS {
            let mut body = json!({"max_tokens": config.max_tokens, "messages": messages});
            if !tools.is_empty() && round < MAX_ROUNDS {
                body["tools"] = tools.clone().into();
            }
            let (mut text, mut reasoning) = (String::new(), String::new());
            let stopped = mytools_ai::stream(&provider, &body, |delta| match delta {
                Delta::Text(piece) => {
                    // Text after a tool call starts a new paragraph.
                    let piece = if text.is_empty() && wrote {
                        format!("\n\n{piece}")
                    } else {
                        piece
                    };
                    text.push_str(&piece);
                    (conversation.on_event)(Event::Text(piece))
                }
                Delta::Reasoning(piece) => {
                    reasoning.push_str(&piece);
                    (conversation.on_event)(Event::Reasoning(piece))
                }
            })
            .map_err(|error| catalog.format("failed", &[("error", &error)]))?;
            wrote |= !text.trim().is_empty();
            let calls = match stopped {
                Stopped::ToolCalls(calls) => calls,
                Stopped::Finished => return Ok(Ending::Finished),
                Stopped::Length => return Ok(Ending::Length),
                Stopped::Cancelled => return Ok(Ending::Cancelled),
            };
            let mut assistant = json!({
                "role": "assistant", "content": text,
                "tool_calls": calls.iter().map(|call| json!({
                    "id": call.id, "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments},
                })).collect::<Vec<_>>(),
            });
            // Thinking models need their reasoning back within the same answer.
            if !reasoning.is_empty() {
                assistant["reasoning_content"] = reasoning.into();
            }
            messages.push(assistant);
            for call in &calls {
                let Some(content) = conversation.run(call) else {
                    return Ok(Ending::Cancelled);
                };
                messages.push(json!({"role": "tool", "tool_call_id": call.id, "content": content}));
            }
        }
        Ok(Ending::Finished)
    }
}

/// A question or answer as the model gets it; failed answers are left out.
fn model_message(message: &ChatMessage) -> Option<Value> {
    match message.role {
        Role::User => {
            let content = match &message.attachment {
                Some(attachment) => {
                    let question = if message.text.trim().is_empty() {
                        "Help me with this text."
                    } else {
                        message.text.trim()
                    };
                    format!("{question}\n\n<text>\n{attachment}\n</text>")
                }
                None => message.text.clone(),
            };
            Some(json!({"role": "user", "content": content}))
        }
        Role::Assistant if !message.error && !message.text.trim().is_empty() => {
            Some(json!({"role": "assistant", "content": message.text}))
        }
        Role::Assistant => None,
    }
}

fn function(offered: &Offered) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": offered.function,
            "description": offered.tool.description,
            "parameters": offered.tool.input_schema,
        },
    })
}

fn system_prompt(config: &Config, tools: bool) -> String {
    let mut prompt = config.system_prompt().to_owned();
    if tools {
        prompt.push_str(&format!(
            "\n\nToday is {}. You can call tools. Use web_search for recent events, prices, \
versions or facts you are unsure of, then answer from the results and cite them as [n] with \
their numbers; use fetch_page to read a page in full when the snippets are not enough. Never \
invent sources.",
            today()
        ));
    }
    prompt
}

/// Today's date in UTC as `YYYY-MM-DD`.
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs() / 86_400)
        .unwrap_or_default() as i64;
    let (year, month, day) = civil(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The calendar date `days` after 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// One answer's tool calls: steps shown to the user and sources numbered across them.
struct Conversation<'a> {
    catalog: &'static Catalog,
    toolbox: &'a mut Toolbox,
    offered: &'a [Offered],
    sources: Vec<ChatSource>,
    steps: usize,
    on_event: &'a mut dyn FnMut(Event) -> bool,
}

impl Conversation<'_> {
    /// Runs a call and returns what the model gets back; `None` when the user stopped.
    fn run(&mut self, call: &ToolCall) -> Option<String> {
        let catalog = self.catalog;
        let Some(offered) = self.offered.iter().find(|tool| tool.function == call.name) else {
            return Some(format!("Error: there is no tool named {}", call.name));
        };
        let arguments: Value = match serde_json::from_str(&call.arguments) {
            Ok(arguments) => arguments,
            Err(error) => return Some(format!("Error: invalid arguments: {error}")),
        };
        let query = arguments["query"].as_str().unwrap_or_default().to_owned();
        let url = arguments["url"].as_str().unwrap_or_default().to_owned();
        let label = if Toolbox::is_web_search(offered) {
            catalog.format("step.search", &[("query", &query)])
        } else if Toolbox::is_fetch(offered) {
            catalog.format("step.read", &[("page", &host(&url))])
        } else {
            catalog.format("step.tool", &[("tool", &call.name)])
        };
        let index = self.steps;
        self.steps += 1;
        let step = |text: String, state| ChatStep { text, state };
        if !(self.on_event)(Event::Step(index, step(label.clone(), StepState::Running))) {
            return None;
        }
        let result = self.toolbox.call(offered, arguments);
        let (content, done) = match result {
            Ok(text) if Toolbox::is_web_search(offered) => match self.number(&text) {
                Some((content, count)) => {
                    let text = catalog.format(
                        "step.searched",
                        &[("query", &query), ("count", &count.to_string())],
                    );
                    (content, step(text, StepState::Done))
                }
                None => (text, step(label, StepState::Done)),
            },
            Ok(text) => (text, step(label, StepState::Done)),
            Err(error) => {
                let text = catalog.format("step.failed", &[("step", &label), ("error", &error)]);
                (format!("Error: {error}"), step(text, StepState::Failed))
            }
        };
        (self.on_event)(Event::Step(index, done)).then_some(content)
    }

    /// Numbers the built-in search's results after those of earlier searches, reports them
    /// as sources, and writes them for the model; `None` when the text is not such a list.
    fn number(&mut self, text: &str) -> Option<(String, usize)> {
        let value: Value = serde_json::from_str(text).ok()?;
        let hits = value["results"].as_array()?;
        let mut lines = Vec::new();
        for hit in hits {
            let (Some(title), Some(url)) = (hit["title"].as_str(), hit["url"].as_str()) else {
                continue;
            };
            let source = ChatSource {
                title: title.into(),
                url: url.into(),
            };
            let number = match self
                .sources
                .iter()
                .position(|known| known.url == source.url)
            {
                Some(position) => position + 1,
                None => {
                    self.sources.push(source.clone());
                    (self.on_event)(Event::Source(source));
                    self.sources.len()
                }
            };
            let snippet = hit["snippet"].as_str().unwrap_or_default();
            lines.push(format!("[{number}] {title}\nURL: {url}\n{snippet}"));
        }
        Some((lines.join("\n\n"), hits.len()))
    }
}

fn host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> &'static Catalog {
        Box::leak(Box::new(
            Catalog::from_json(
                include_str!("../locales/en.json"),
                include_str!("../locales/en.json"),
            )
            .unwrap(),
        ))
    }

    #[test]
    fn dates_and_questions_with_a_selection() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_405), (2025, 11, 13));
        assert_eq!(civil(19_782), (2024, 2, 29));
        let message = ChatMessage {
            role: Role::User,
            text: String::new(),
            attachment: Some("Hello".into()),
            ..Default::default()
        };
        assert_eq!(
            model_message(&message).unwrap()["content"],
            "Help me with this text.\n\n<text>\nHello\n</text>"
        );
        let failed = ChatMessage {
            role: Role::Assistant,
            text: "no key".into(),
            error: true,
            ..Default::default()
        };
        assert!(model_message(&failed).is_none());
    }

    #[test]
    fn search_results_are_numbered_across_searches() {
        let mut toolbox = Toolbox::default();
        let mut events = Vec::new();
        let mut on_event = |event| {
            events.push(event);
            true
        };
        let mut conversation = Conversation {
            catalog: catalog(),
            toolbox: &mut toolbox,
            offered: &[],
            sources: Vec::new(),
            steps: 0,
            on_event: &mut on_event,
        };
        let first = r#"{"results":[{"title":"A","url":"https://a","snippet":"a"},{"title":"B","url":"https://b"}]}"#;
        let (text, count) = conversation.number(first).unwrap();
        assert_eq!(count, 2);
        assert_eq!(text, "[1] A\nURL: https://a\na\n\n[2] B\nURL: https://b\n");
        let again =
            r#"{"results":[{"title":"B","url":"https://b"},{"title":"C","url":"https://c"}]}"#;
        let (text, _) = conversation.number(again).unwrap();
        assert!(text.starts_with("[2] B") && text.contains("[3] C"));
        assert!(conversation.number("plain text").is_none());
        let unknown = conversation
            .run(&ToolCall {
                id: "1".into(),
                name: "nope".into(),
                arguments: "{}".into(),
            })
            .unwrap();
        assert!(unknown.starts_with("Error"));
        drop(conversation);
        let sources = events
            .iter()
            .filter(|event| matches!(event, Event::Source(_)))
            .count();
        assert_eq!(sources, 3);
    }
}
