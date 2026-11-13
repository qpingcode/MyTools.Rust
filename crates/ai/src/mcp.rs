//! A small Model Context Protocol client: starts a server (a local process speaking JSON-RPC
//! over stdin/stdout, or a remote one over Streamable HTTP), lists its tools and calls them.
//! Servers are configured like Claude Desktop's `mcpServers`.
use crate::describe;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

const PROTOCOL_VERSION: &str = "2025-06-18";
/// How long a request may take, e.g. a slow search.
const TIMEOUT: Duration = Duration::from_secs(60);

/// One entry of `mcpServers`: either `command` (with `args` and `env`) or `url` (with
/// `headers`). `autoApprove` lists tools that may run without asking, in addition to those
/// the server marks read-only.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub url: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub auto_approve: Vec<String>,
    pub disabled: bool,
}

/// A file of servers by name, as Claude Desktop and VS Code write it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct ServersFile {
    pub mcp_servers: BTreeMap<String, ServerConfig>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments.
    pub input_schema: Value,
    /// The server says the tool only reads (`annotations.readOnlyHint`).
    pub read_only: bool,
}

/// What a tool returned: its text content, and whether it reports a failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallResult {
    pub text: String,
    pub is_error: bool,
}

pub struct Client {
    transport: Transport,
    next_id: u64,
}

enum Transport {
    Stdio {
        child: Child,
        stdin: ChildStdin,
        lines: mpsc::Receiver<String>,
    },
    Http {
        client: reqwest::blocking::Client,
        url: String,
        headers: BTreeMap<String, String>,
        session: Option<String>,
    },
}

impl Client {
    /// Starts or reaches the server and completes the handshake.
    pub fn connect(config: &ServerConfig) -> Result<Self, String> {
        let transport = match (&config.command, &config.url) {
            (Some(command), _) => spawn(command, &config.args, &config.env)?,
            (None, Some(url)) => Transport::Http {
                client: reqwest::blocking::Client::builder()
                    .timeout(TIMEOUT)
                    .build()
                    .map_err(|e| describe(&e))?,
                url: url.clone(),
                headers: config.headers.clone(),
                session: None,
            },
            (None, None) => return Err("no command or url".into()),
        };
        let mut client = Self {
            transport,
            next_id: 1,
        };
        client.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "MyTools", "version": env!("CARGO_PKG_VERSION")},
            }),
        )?;
        client.notify("notifications/initialized")?;
        Ok(client)
    }

    /// Every tool of the server, following pagination.
    pub fn tools(&mut self) -> Result<Vec<Tool>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(cursor) => json!({"cursor": cursor}),
                None => json!({}),
            };
            let result = self.request("tools/list", params)?;
            for tool in result["tools"].as_array().into_iter().flatten() {
                let Some(name) = tool["name"].as_str() else {
                    continue;
                };
                tools.push(Tool {
                    name: name.into(),
                    description: tool["description"].as_str().unwrap_or_default().into(),
                    input_schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                    read_only: tool["annotations"]["readOnlyHint"].as_bool() == Some(true),
                });
            }
            cursor = result["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() || tools.len() > 500 {
                return Ok(tools);
            }
        }
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Result<CallResult, String> {
        let result = self.request("tools/call", json!({"name": name, "arguments": arguments}))?;
        let text = result["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(CallResult {
            text,
            is_error: result["isError"].as_bool() == Some(true),
        })
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let response = match &mut self.transport {
            Transport::Stdio { stdin, lines, .. } => {
                write_line(stdin, &message)?;
                read_stdio(stdin, lines, id)?
            }
            Transport::Http {
                client,
                url,
                headers,
                session,
            } => post(client, url, headers, session, &message, Some(id))?
                .ok_or_else(|| format!("{method}: no response"))?,
        };
        if let Some(error) = response.get("error") {
            return Err(error["message"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| error.to_string()));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notify(&mut self, method: &str) -> Result<(), String> {
        let message = json!({"jsonrpc": "2.0", "method": method});
        match &mut self.transport {
            Transport::Stdio { stdin, .. } => write_line(stdin, &message),
            Transport::Http {
                client,
                url,
                headers,
                session,
            } => post(client, url, headers, session, &message, None).map(|_| ()),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Transport::Stdio { child, .. } = &mut self.transport {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<Transport, String> {
    let build = |program: &str, args: &[String]| {
        let mut process = Command::new(program);
        process
            .args(args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            process.creation_flags(CREATE_NO_WINDOW);
        }
        process
    };
    let mut child = match build(command, args).spawn() {
        Ok(child) => child,
        // `npx`, `uvx` and the like are batch files on Windows, which only cmd runs.
        #[cfg(windows)]
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut wrapped = vec!["/C".to_owned(), command.to_owned()];
            wrapped.extend(args.iter().cloned());
            build("cmd", &wrapped)
                .spawn()
                .map_err(|e| format!("{command}: {e}"))?
        }
        Err(error) => return Err(format!("{command}: {error}")),
    };
    let stdin = child.stdin.take().ok_or("stdin")?;
    let stdout = child.stdout.take().ok_or("stdout")?;
    let (sender, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout.take(64 * 1024 * 1024)).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    Ok(Transport::Stdio {
        child,
        stdin,
        lines,
    })
}

fn write_line(stdin: &mut ChildStdin, message: &Value) -> Result<(), String> {
    let mut line = message.to_string();
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|e| format!("the server stopped: {e}"))
}

/// Waits for the response to request `id`, answering requests from the server meanwhile.
fn read_stdio(
    stdin: &mut ChildStdin,
    lines: &mpsc::Receiver<String>,
    id: u64,
) -> Result<Value, String> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        let line = lines.recv_timeout(wait).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => "the server did not answer in time".to_owned(),
            mpsc::RecvTimeoutError::Disconnected => "the server stopped".to_owned(),
        })?;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue; // Servers sometimes log to stdout.
        };
        if let Some(answer) = matching(&message, id) {
            return Ok(answer);
        }
        if let Some(reply) = refuse(&message) {
            write_line(stdin, &reply)?;
        }
    }
}

/// `message` when it answers request `id`.
fn matching(message: &Value, id: u64) -> Option<Value> {
    (message.get("method").is_none() && message["id"].as_u64() == Some(id)).then(|| message.clone())
}

/// The error reply to a request from the server (e.g. sampling), which this client offers
/// none of; `None` for notifications.
fn refuse(message: &Value) -> Option<Value> {
    let id = message.get("id")?;
    message.get("method")?;
    Some(json!({
        "jsonrpc": "2.0", "id": id,
        "error": {"code": -32601, "message": "not supported"},
    }))
}

fn post(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: &BTreeMap<String, String>,
    session: &mut Option<String>,
    message: &Value,
    id: Option<u64>,
) -> Result<Option<Value>, String> {
    let mut request = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .json(message);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(session) = session.as_deref() {
        request = request.header("Mcp-Session-Id", session);
    }
    let response = request.send().map_err(|e| describe(&e))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    if let Some(value) = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
    {
        *session = Some(value.to_owned());
    }
    let Some(id) = id else {
        return Ok(None);
    };
    let events = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|kind| kind.starts_with("text/event-stream"));
    if !events {
        let body: Value = response.json().map_err(|e| describe(&e))?;
        return Ok(Some(body));
    }
    for line in BufReader::new(response).lines() {
        let line = line.map_err(|e| describe(&e))?;
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        if let Ok(message) = serde_json::from_str::<Value>(data.trim())
            && let Some(answer) = matching(&message, id)
        {
            return Ok(Some(answer));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_claude_desktop_style_configuration() {
        let file: ServersFile = serde_json::from_str(
            r#"{"mcpServers":{
                "fs":{"command":"npx","args":["-y","server"],"env":{"A":"1"},"autoApprove":["read"]},
                "remote":{"url":"https://example.com/mcp","headers":{"Authorization":"Bearer x"}}
            }}"#,
        )
        .unwrap();
        assert_eq!(file.mcp_servers["fs"].command.as_deref(), Some("npx"));
        assert_eq!(file.mcp_servers["fs"].auto_approve, ["read"]);
        assert_eq!(
            file.mcp_servers["remote"].headers["Authorization"],
            "Bearer x"
        );
    }

    #[test]
    fn matches_responses_and_refuses_server_requests() {
        let response = json!({"jsonrpc":"2.0","id":3,"result":{}});
        assert!(matching(&response, 3).is_some());
        assert!(matching(&response, 4).is_none());
        let request = json!({"jsonrpc":"2.0","id":3,"method":"sampling/createMessage"});
        assert!(matching(&request, 3).is_none());
        assert_eq!(refuse(&request).unwrap()["error"]["code"], -32601);
        assert!(refuse(&json!({"jsonrpc":"2.0","method":"notifications/progress"})).is_none());
    }

    /// A shell script playing a server: it answers initialize, lists one tool and echoes
    /// the call, with a log line and a server request in between.
    #[cfg(unix)]
    #[test]
    fn talks_to_a_stdio_server() {
        let script = r#"
while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*) echo 'not json'; echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}}}}';;
    *'"tools/list"'*) echo '{"jsonrpc":"2.0","id":9,"method":"roots/list"}'; echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"Echo","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}}';;
    *'"tools/call"'*) echo '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"hi"}],"isError":false}}';;
  esac
done"#;
        let config = ServerConfig {
            command: Some("sh".into()),
            args: vec!["-c".into(), script.into()],
            ..Default::default()
        };
        let mut client = Client::connect(&config).unwrap();
        let tools = client.tools().unwrap();
        assert_eq!(tools[0].name, "echo");
        assert!(tools[0].read_only);
        assert_eq!(
            client.call("echo", json!({})).unwrap(),
            CallResult {
                text: "hi".into(),
                is_error: false
            }
        );
    }
}
