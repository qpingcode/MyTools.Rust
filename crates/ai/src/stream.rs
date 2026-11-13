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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// The model finished its answer.
    Finished,
    /// The reply reached the token limit.
    Length,
    /// The callback asked to stop.
    Cancelled,
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
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| describe(&e))? == 0 {
            // The connection closed without `[DONE]`; what arrived is the answer.
            return Ok(Stopped::Finished);
        }
        let Some(data) = line.trim_end().strip_prefix("data:") else {
            continue; // Blank separators, comments and keep-alives.
        };
        let data = data.trim();
        if data == "[DONE]" {
            return Ok(Stopped::Finished);
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
        match choice["finish_reason"].as_str() {
            Some("length") => return Ok(Stopped::Length),
            Some(_) => return Ok(Stopped::Finished),
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
