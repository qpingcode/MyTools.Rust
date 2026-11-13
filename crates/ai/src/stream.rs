//! Streamed replies: `"stream": true` makes the endpoint send server-sent events, one
//! `data: {json}` line per chunk and `data: [DONE]` at the end.
use crate::{Provider, describe};
use std::{
    io::{BufRead, BufReader},
    time::Duration,
};

/// A piece of the reply as it arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delta {
    /// Text of the answer.
    Text(String),
    /// The model's reasoning before the answer, for models that share it.
    Reasoning(String),
}

/// Why a stream ended without an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// The model finished its answer.
    Finished,
    /// The reply reached the token limit.
    Length,
    /// The callback asked to stop.
    Cancelled,
    /// The model asks for these tools to be called before it goes on.
    ToolCalls(Vec<ToolCall>),
}

/// A function the model wants called, with its arguments as JSON text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// How long the stream may stay silent before it counts as broken.
const IDLE: Duration = Duration::from_secs(90);

/// Sends `body` with `"stream": true` and calls `on_delta` with each piece of the reply;
/// returning `false` from it stops reading. Blocks until the reply ends.
pub fn stream(
    provider: &Provider,
    body: &serde_json::Value,
    mut on_delta: impl FnMut(Delta) -> bool,
) -> Result<Stopped, String> {
    let mut body = body.clone();
    body["stream"] = true.into();
    let client = provider.client(IDLE)?;
    let response = provider.post(&client, &body)?;
    let mut reader = BufReader::new(response);
    let mut line = String::new();
    // Tool calls arrive in pieces, keyed by their index.
    let mut calls: Vec<ToolCall> = Vec::new();
    let finished = |calls: Vec<ToolCall>| {
        if calls.is_empty() {
            Stopped::Finished
        } else {
            Stopped::ToolCalls(calls)
        }
    };
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| describe(&e))? == 0 {
            // The connection closed without `[DONE]`; what arrived is the answer.
            return Ok(finished(calls));
        }
        let Some(data) = line.trim_end().strip_prefix("data:") else {
            continue; // Blank separators, comments and keep-alives.
        };
        let data = data.trim();
        if data == "[DONE]" {
            return Ok(finished(calls));
        }
        let chunk: serde_json::Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
        if let Some(message) = chunk["error"]["message"].as_str() {
            return Err(message.to_owned());
        }
        let choice = &chunk["choices"][0];
        for (field, make) in [
            ("reasoning_content", Delta::Reasoning as fn(String) -> Delta),
            ("content", Delta::Text),
        ] {
            if let Some(text) = choice["delta"][field].as_str().filter(|t| !t.is_empty())
                && !on_delta(make(text.to_owned()))
            {
                return Ok(Stopped::Cancelled);
            }
        }
        for piece in choice["delta"]["tool_calls"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let index = piece["index"].as_u64().unwrap_or(calls.len() as u64) as usize;
            if index >= calls.len() {
                calls.resize(index + 1, ToolCall::default());
            }
            let call = &mut calls[index];
            if let Some(id) = piece["id"].as_str() {
                call.id = id.into();
            }
            if let Some(name) = piece["function"]["name"].as_str() {
                call.name.push_str(name);
            }
            if let Some(arguments) = piece["function"]["arguments"].as_str() {
                call.arguments.push_str(arguments);
            }
        }
        match choice["finish_reason"].as_str() {
            Some("length") => return Ok(Stopped::Length),
            Some(_) => return Ok(finished(calls)),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tests::{provider, serve};

    fn events(chunks: &[&str]) -> String {
        chunks
            .iter()
            .map(|chunk| format!("data: {chunk}\n\n"))
            .collect()
    }

    #[test]
    fn delivers_reasoning_and_text_in_order() {
        let body = events(&[
            r#"{"choices":[{"delta":{"reasoning_content":"think"}}]}"#,
            r#"{"choices":[{"delta":{"content":"Hel"}}]}"#,
            r#"{"choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]);
        let (address, server) = serve(vec![(200, "text/event-stream", body)], |request| {
            assert_eq!(request["stream"], true);
        });
        let mut deltas = Vec::new();
        let stopped = stream(
            &provider(address),
            &serde_json::json!({"messages": []}),
            |delta| {
                deltas.push(delta);
                true
            },
        )
        .unwrap();
        assert_eq!(stopped, Stopped::Finished);
        assert_eq!(
            deltas,
            [
                Delta::Reasoning("think".into()),
                Delta::Text("Hel".into()),
                Delta::Text("lo".into())
            ]
        );
        server.join().unwrap();
    }

    #[test]
    fn collects_tool_calls_split_over_chunks() {
        let body = events(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"web_search","arguments":"{\"query\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"rust\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"c2","function":{"name":"fetch_page","arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#,
        ]);
        let (address, server) = serve(vec![(200, "text/event-stream", body)], |_| {});
        let stopped = stream(&provider(address), &serde_json::json!({}), |_| true).unwrap();
        assert_eq!(
            stopped,
            Stopped::ToolCalls(vec![
                ToolCall {
                    id: "c1".into(),
                    name: "web_search".into(),
                    arguments: r#"{"query":"rust"}"#.into()
                },
                ToolCall {
                    id: "c2".into(),
                    name: "fetch_page".into(),
                    arguments: "{}".into()
                },
            ])
        );
        server.join().unwrap();
    }

    #[test]
    fn stops_when_asked_and_reports_errors_and_limits() {
        let body = events(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            r#"{"choices":[{"delta":{"content":"b"}}]}"#,
        ]);
        let limit =
            events(&[r#"{"choices":[{"delta":{"content":"a"},"finish_reason":"length"}]}"#]);
        let error = events(&[r#"{"error":{"message":"overloaded"}}"#]);
        let (address, server) = serve(
            vec![
                (200, "text/event-stream", body),
                (200, "text/event-stream", limit),
                (200, "text/event-stream", error),
            ],
            |_| {},
        );
        let provider = provider(address);
        let request = serde_json::json!({"messages": []});
        let mut count = 0;
        let stopped = stream(&provider, &request, |_| {
            count += 1;
            false
        });
        assert_eq!((stopped, count), (Ok(Stopped::Cancelled), 1));
        assert_eq!(stream(&provider, &request, |_| true), Ok(Stopped::Length));
        assert_eq!(
            stream(&provider, &request, |_| true),
            Err("overloaded".into())
        );
        server.join().unwrap();
    }
}
