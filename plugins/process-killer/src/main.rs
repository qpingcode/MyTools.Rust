mod ports;
use mytools_plugin_api::{
    PluginActionRequest, PluginEffect, Resource, SearchItem, SearchResponse, i18n::Catalog, text,
};
use std::cell::RefCell;
use sysinfo::{Pid, ProcessesToUpdate, System};

struct State {
    system: System,
    issued: std::collections::HashMap<String, (Pid, u64)>,
}

impl State {
    fn search(&mut self, query: &str, catalog: &Catalog) -> SearchResponse {
        let port = match ports::query_port(query) {
            Ok(port) => port,
            Err(()) => {
                return SearchResponse {
                    error: Some(catalog.get("port.invalid").into()),
                    ..Default::default()
                };
            }
        };
        let owners = match port.map(ports::owners).transpose() {
            Ok(owners) => owners,
            Err(error) => {
                return SearchResponse {
                    error: Some(catalog.format("query.error", &[("error", &error)])),
                    ..Default::default()
                };
            }
        };
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        let query = query.trim().to_lowercase();
        let mut processes: Vec<_> = self
            .system
            .processes()
            .iter()
            .filter(|(pid, process)| {
                owners
                    .as_ref()
                    .map(|owners| owners.contains(&pid.as_u32()))
                    .unwrap_or_else(|| {
                        process
                            .name()
                            .to_string_lossy()
                            .to_lowercase()
                            .contains(&query)
                    })
            })
            .collect();
        processes.sort_by_key(|(pid, process)| {
            (
                process.name().to_string_lossy().to_lowercase(),
                pid.as_u32(),
            )
        });
        self.issued.clear();
        let items = processes
            .into_iter()
            .take(50)
            .map(|(pid, process)| {
                let id = format!("{}:{}", pid.as_u32(), process.start_time());
                self.issued.insert(id.clone(), (*pid, process.start_time()));
                // Resource is an opaque process identity, so actions return to this plugin.
                SearchItem {
                    intent_match: false,
                    title: process.name().to_string_lossy().into(),
                    description: catalog.format(
                        "process.description",
                        &[
                            ("pid", &pid.to_string()),
                            ("memory", &(process.memory() / 1024 / 1024).to_string()),
                        ],
                    ),
                    icon: None,
                    actions: Some(vec![
                        text::command("kill", catalog.get("kill").into()).with_item(id.clone()),
                    ]),
                    resource: Resource::PluginItem { id },
                    preview: None,
                }
            })
            .collect();
        SearchResponse {
            items,
            ..Default::default()
        }
    }

    fn kill(&mut self, request: PluginActionRequest, catalog: &Catalog) -> SearchResponse {
        let item = request.item.unwrap_or_default();
        let Some(&(pid, start)) = self.issued.get(&item) else {
            return SearchResponse {
                error: Some(catalog.get("missing").into()),
                ..Default::default()
            };
        };
        self.system
            .refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        let valid =
            request.command == "kill" && pid.as_u32() != std::process::id() && pid.as_u32() != 0;
        let result = self
            .system
            .process(pid)
            .filter(|process| valid && process.start_time() == start)
            .is_some_and(|process| process.kill());
        if result {
            self.issued.remove(&item);
            SearchResponse {
                effect: Some(PluginEffect {
                    refresh: true,
                    message: catalog.get("killed").into(),
                    ..Default::default()
                }),
                ..Default::default()
            }
        } else {
            SearchResponse {
                error: Some(catalog.get("kill.error").into()),
                ..Default::default()
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    let state = RefCell::new(State {
        system: System::new(),
        issued: Default::default(),
    });
    mytools_plugin_api::serve_interactive(
        |query| state.borrow_mut().search(query, &catalog),
        |request| state.borrow_mut().kill(request, &catalog),
    )
}
