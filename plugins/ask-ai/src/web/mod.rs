//! The built-in web search, served over MCP (stdio) when Ask AI runs with
//! `--mcp-web-search`; the conversation starts it like any other MCP server.
pub mod page;
pub mod search;

use serde_json::{Value, json};
use std::io::{BufRead, Write};

pub const ARGUMENT: &str = "--mcp-web-search";
pub const SEARCH_TOOL: &str = "web_search";
pub const FETCH_TOOL: &str = "fetch_page";

fn tools() -> Value {
    json!([
        {
            "name": SEARCH_TOOL,
            "description": "Search the web. Returns a JSON list of results with title, url and snippet. Use for recent events or facts you are unsure of.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "What to search for"},
                    "count": {"type": "integer", "description": "How many results, 1-10"}
                },
                "required": ["query"]
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": true}
        },
        {
            "name": FETCH_TOOL,
            "description": "Read a web page (http or https) as plain text, e.g. a search result, to answer from its content.",
            "inputSchema": {
                "type": "object",
                "properties": {"url": {"type": "string"}},
                "required": ["url"]
            },
            "annotations": {"readOnlyHint": true, "openWorldHint": true}
        }
    ])
}

/// Answers MCP requests on stdin until it closes.
pub fn serve() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(request) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        if let Some(response) = handle(&request, call) {
            writeln!(stdout, "{response}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// The response to one request, `None` for a notification.
fn handle(request: &Value, call: impl Fn(&str, &Value) -> Result<String, String>) -> Option<Value> {
    let id = request.get("id")?.clone();
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": request["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "mytools-web-search", "version": env!("CARGO_PKG_VERSION")},
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let params = &request["params"];
            let name = params["name"].as_str().unwrap_or_default();
            match call(name, &params["arguments"]) {
                Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
                Err(error) => {
                    json!({"content": [{"type": "text", "text": error}], "isError": true})
                }
            }
        }
        method => {
            return Some(json!({
                "jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unknown method {method}")},
            }));
        }
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn call(name: &str, arguments: &Value) -> Result<String, String> {
    match name {
        SEARCH_TOOL => {
            let query = arguments["query"].as_str().unwrap_or_default().trim();
            if query.is_empty() {
                return Err("query is empty".into());
            }
            let config = crate::config::Config::load()?.search;
            let count = arguments["count"]
                .as_u64()
                .map(|count| count as usize)
                .unwrap_or(config.max_results);
            let (provider, hits) = search::search(&config, query, count)?;
            Ok(json!({"provider": provider, "results": hits}).to_string())
        }
        FETCH_TOOL => page::fetch(arguments["url"].as_str().unwrap_or_default()),
        other => Err(format!("unknown tool {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_the_mcp_handshake_listing_and_calls() {
        let echo = |name: &str, arguments: &Value| -> Result<String, String> {
            match name {
                SEARCH_TOOL => Ok(arguments["query"].to_string()),
                _ => Err("boom".into()),
            }
        };
        let init = handle(
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}),
            echo,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(
            handle(
                &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                echo
            )
            .is_none()
        );
        let list = handle(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}), echo).unwrap();
        assert_eq!(list["result"]["tools"][0]["name"], SEARCH_TOOL);
        assert_eq!(
            list["result"]["tools"][1]["annotations"]["readOnlyHint"],
            true
        );
        let found = handle(
            &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"web_search","arguments":{"query":"x"}}}),
            echo,
        )
        .unwrap();
        assert_eq!(found["result"]["content"][0]["text"], "\"x\"");
        let failed = handle(
            &json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"fetch_page","arguments":{}}}),
            echo,
        )
        .unwrap();
        assert_eq!(failed["result"]["isError"], true);
        let unknown = handle(
            &json!({"jsonrpc":"2.0","id":5,"method":"resources/list"}),
            echo,
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);
    }
}
