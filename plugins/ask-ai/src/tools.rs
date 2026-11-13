//! The tools the model may call: the built-in web search and the MCP servers in
//! `mcp.json`. Servers start on the first question and keep running with the plugin.
use crate::web;
use mytools_ai::mcp::{Client, ServerConfig, ServersFile, Tool};
use serde_json::Value;

const BUILT_IN: &str = "web";

struct Server {
    name: String,
    config: ServerConfig,
    client: Option<Client>,
    tools: Vec<Tool>,
}

/// A tool as the model sees it: a unique function name, and where it goes.
#[derive(Clone, Debug)]
pub struct Offered {
    pub function: String,
    pub server: usize,
    pub tool: Tool,
}

#[derive(Default)]
pub struct Toolbox {
    servers: Vec<Server>,
    /// The configuration the servers were started with; a change restarts them.
    signature: String,
    /// Servers that could not start, for the log.
    pub problems: Vec<String>,
}

/// Function names allow letters, digits, `_` and `-`, up to 64 characters.
fn function_name(server: &str, tool: &str) -> String {
    let name: String = format!("{server}__{tool}")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    name.chars().take(64).collect()
}

impl Toolbox {
    /// Starts or restarts the servers when the configuration changed.
    pub fn configure(&mut self, web_search: bool, file: &ServersFile) {
        let mut wanted: Vec<(String, ServerConfig)> = Vec::new();
        if web_search && let Ok(executable) = std::env::current_exe() {
            wanted.push((
                BUILT_IN.into(),
                ServerConfig {
                    command: Some(executable.to_string_lossy().into_owned()),
                    args: vec![web::ARGUMENT.into()],
                    auto_approve: vec![web::SEARCH_TOOL.into(), web::FETCH_TOOL.into()],
                    ..Default::default()
                },
            ));
        }
        wanted.extend(
            file.mcp_servers
                .iter()
                .filter(|(_, config)| !config.disabled)
                .map(|(name, config)| (name.clone(), config.clone())),
        );
        let signature = format!("{wanted:?}");
        if signature == self.signature {
            return;
        }
        self.signature = signature;
        self.problems.clear();
        self.servers = wanted
            .into_iter()
            .map(|(name, config)| Server {
                name,
                config,
                client: None,
                tools: Vec::new(),
            })
            .collect();
    }

    /// The tools that may run without asking: those a server marks read-only or that its
    /// `autoApprove` lists. Servers that fail to start are left out this time.
    pub fn offered(&mut self) -> Vec<Offered> {
        let mut offered = Vec::new();
        for (index, server) in self.servers.iter_mut().enumerate() {
            if server.client.is_none() {
                match Client::connect(&server.config).and_then(|mut client| {
                    let tools = client.tools()?;
                    Ok((client, tools))
                }) {
                    Ok((client, tools)) => {
                        server.client = Some(client);
                        server.tools = tools;
                    }
                    Err(error) => {
                        self.problems.push(format!("{}: {error}", server.name));
                        continue;
                    }
                }
            }
            for tool in &server.tools {
                if !(tool.read_only || server.config.auto_approve.contains(&tool.name)) {
                    continue;
                }
                let function = if server.name == BUILT_IN {
                    tool.name.clone()
                } else {
                    function_name(&server.name, &tool.name)
                };
                offered.push(Offered {
                    function,
                    server: index,
                    tool: tool.clone(),
                });
            }
        }
        offered
    }

    /// Calls a tool; the text is what the model gets back. A server that stopped is
    /// started again on the next question.
    pub fn call(&mut self, offered: &Offered, arguments: Value) -> Result<String, String> {
        let server = &mut self.servers[offered.server];
        let client = server.client.as_mut().ok_or("the server stopped")?;
        match client.call(&offered.tool.name, arguments) {
            Ok(result) if result.is_error => Err(result.text),
            Ok(result) => Ok(result.text),
            Err(error) => {
                server.client = None;
                Err(error)
            }
        }
    }

    pub fn is_web_search(offered: &Offered) -> bool {
        offered.function == web::SEARCH_TOOL
    }

    pub fn is_fetch(offered: &Offered) -> bool {
        offered.function == web::FETCH_TOOL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_names_are_safe_and_unique_per_server() {
        assert_eq!(
            function_name("git hub", "list.issues"),
            "git_hub__list_issues"
        );
        assert_eq!(function_name("a", &"x".repeat(80)).len(), 64);
    }

    #[test]
    fn only_a_changed_configuration_restarts_servers() {
        let mut toolbox = Toolbox::default();
        let file: ServersFile = serde_json::from_str(
            r#"{"mcpServers":{"off":{"command":"x","disabled":true},"on":{"command":"y"}}}"#,
        )
        .unwrap();
        toolbox.configure(false, &file);
        assert_eq!(toolbox.servers.len(), 1);
        toolbox.servers[0].tools.push(Tool {
            name: "t".into(),
            description: String::new(),
            input_schema: Value::Null,
            read_only: true,
        });
        toolbox.configure(false, &file);
        assert_eq!(toolbox.servers[0].tools.len(), 1, "kept");
        toolbox.configure(true, &file);
        assert_eq!(toolbox.servers.len(), 2);
        assert_eq!(toolbox.servers[0].name, BUILT_IN);
    }
}
