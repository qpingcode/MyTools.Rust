use std::{io::Read, time::Duration};

/// Where requests go: an OpenAI-compatible endpoint, its model and the API key.
#[derive(Clone, Debug, Default)]
pub struct Provider {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

impl Provider {
    /// DeepSeek at `base_url` with `model`; the key comes from `DEEPSEEK_API_KEY`, and
    /// `DEEPSEEK_MODEL` overrides the model.
    pub fn deepseek(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.into(),
            model: crate::environment::variable(crate::DEEPSEEK_MODEL)
                .unwrap_or_else(|| model.into()),
            api_key: crate::environment::variable(crate::DEEPSEEK_API_KEY).unwrap_or_default(),
        }
    }

    pub fn has_key(&self) -> bool {
        !self.api_key.trim().is_empty()
    }

    pub(crate) fn client(&self, timeout: Duration) -> Result<reqwest::blocking::Client, String> {
        reqwest::blocking::Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| describe(&e))
    }

    /// Posts `body` (with this provider's model) to `/chat/completions`.
    pub(crate) fn post(
        &self,
        client: &reqwest::blocking::Client,
        body: &serde_json::Value,
    ) -> Result<reqwest::blocking::Response, String> {
        let mut body = body.clone();
        if body.get("model").is_none() {
            body["model"] = self.model.clone().into();
        }
        let response = client
            .post(format!(
                "{}/chat/completions",
                self.base_url.trim_end_matches('/')
            ))
            .bearer_auth(self.api_key.trim())
            .json(&body)
            .send()
            .map_err(|e| match environment_proxy() {
                // reqwest picks the proxy up silently; name it so a refusal is explainable.
                Some(proxy) => format!("{} (via proxy {proxy})", describe(&e)),
                None => describe(&e),
            })?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status().as_u16()));
        }
        Ok(response)
    }
}

/// Sends one chat completion (`body` without `model` gets the provider's) and returns the
/// reply text; a reply cut short, e.g. by `max_tokens`, is an error.
pub fn complete(
    provider: &Provider,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<String, String> {
    let client = provider.client(timeout)?;
    let response = provider.post(&client, body)?;
    let mut bytes = Vec::new();
    response
        .take(256 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| describe(&e))?;
    if bytes.len() > 256 * 1024 {
        return Err("256 KiB".into());
    }
    parse_response(&bytes)
}

/// The HTTPS proxy reqwest takes from the environment, without any credentials.
fn environment_proxy() -> Option<String> {
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .map(|value| without_credentials(value.trim()))
}

fn without_credentials(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(scheme), Some(at)) if at > scheme => {
            format!("{}{}", &url[..scheme + 3], &url[at + 1..])
        }
        (None, Some(at)) => url[at + 1..].to_owned(),
        _ => url.to_owned(),
    }
}

/// The error and every underlying cause, e.g. "error sending request for url (...): client
/// error (Connect): proxy refused". reqwest's own message names only the outermost layer.
pub fn describe(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        // Wrappers often repeat their inner message verbatim.
        if !parts.iter().any(|part| part.contains(&text)) {
            parts.push(text);
        }
        source = cause.source();
    }
    parts.join(": ")
}

fn parse_response(bytes: &[u8]) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let choice = &value["choices"][0];
    if choice["finish_reason"] != "stop" {
        return Err("finish_reason != stop".into());
    }
    choice["message"]["content"]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "choices[0].message.content".into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    /// Serves one request per reply on a local port and hands each request body to `check`.
    pub fn serve(
        replies: Vec<(u16, &'static str, String)>,
        check: impl Fn(serde_json::Value) + Send + 'static,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for (status, content_type, body) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(line.starts_with("POST /chat/completions "));
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut request = vec![0; length];
                reader.read_exact(&mut request).unwrap();
                check(serde_json::from_slice(&request).unwrap());
                write!(
                    stream,
                    "HTTP/1.1 {status} Mock\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (address, server)
    }

    pub fn provider(base_url: String) -> Provider {
        Provider {
            base_url,
            model: "test-model".into(),
            api_key: "test-key".into(),
        }
    }

    #[test]
    fn completes_with_the_providers_model_and_reports_http_errors() {
        let reply = r#"{"choices":[{"finish_reason":"stop","message":{"content":"Hello"}}]}"#;
        let (address, server) = serve(
            vec![
                (200, "application/json", reply.into()),
                (401, "application/json", reply.into()),
            ],
            |request| {
                assert_eq!(request["model"], "test-model");
                assert_eq!(request["messages"][0]["content"], "你好");
            },
        );
        let provider = provider(address);
        let body = serde_json::json!({"messages": [{"role": "user", "content": "你好"}]});
        assert_eq!(
            complete(&provider, &body, Duration::from_secs(5)).unwrap(),
            "Hello"
        );
        assert_eq!(
            complete(&provider, &body, Duration::from_secs(5)).unwrap_err(),
            "HTTP 401"
        );
        server.join().unwrap();
    }

    #[test]
    fn proxy_names_drop_credentials() {
        assert_eq!(
            without_credentials("http://user:secret@proxy.local:3128"),
            "http://proxy.local:3128"
        );
        assert_eq!(without_credentials("user@proxy:8080"), "proxy:8080");
        assert_eq!(
            without_credentials("http://proxy:3128"),
            "http://proxy:3128"
        );
    }

    #[test]
    fn describe_includes_each_distinct_cause() {
        #[derive(Debug)]
        struct Layer(&'static str, Option<Box<Layer>>);
        impl std::fmt::Display for Layer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        impl std::error::Error for Layer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                self.1.as_deref().map(|layer| layer as _)
            }
        }
        let error = Layer(
            "error sending request",
            Some(Box::new(Layer(
                "client error (Connect)",
                Some(Box::new(Layer(
                    "client error (Connect)",
                    Some(Box::new(Layer("proxy refused", None))),
                ))),
            ))),
        );
        assert_eq!(
            describe(&error),
            "error sending request: client error (Connect): proxy refused"
        );
    }

    #[test]
    fn requires_complete_nonempty_reply() {
        assert_eq!(
            parse_response(
                br#"{"choices":[{"finish_reason":"stop","message":{"content":"Hello"}}]}"#
            )
            .unwrap(),
            "Hello"
        );
        assert!(
            parse_response(
                br#"{"choices":[{"finish_reason":"length","message":{"content":"partial"}}]}"#
            )
            .is_err()
        );
        assert!(parse_response(br#"{"choices":[]}"#).is_err());
    }
}
