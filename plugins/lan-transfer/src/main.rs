use std::{cell::RefCell, io};

use mytools_plugin_api::{
    ActionId, ActionSpec, PluginEffect, Resource, SearchItem, SearchResponse, i18n::Catalog, text,
};

mod server;
mod web;

const ENGLISH: &str = include_str!("../locales/en.json");
const CHINESE: &str = include_str!("../locales/zh-CN.json");
const ICON: &str = "builtin:send";

#[derive(Default)]
enum Transfer {
    #[default]
    Unopened,
    Stopped,
    Running(server::Session),
    Failed(String),
}

impl Transfer {
    fn start(&mut self) {
        *self = match server::Session::start() {
            Ok(session) => Self::Running(session),
            Err(error) => Self::Failed(error.to_string()),
        };
    }

    fn items(&self, catalog: &Catalog) -> Vec<SearchItem> {
        match self {
            Self::Running(session) => vec![
                SearchItem {
                    intent_match: false,
                    title: catalog.get("launcher.open").into(),
                    description: catalog.get("launcher.open.detail").into(),
                    icon: Some(ICON.into()),
                    resource: Resource::Url { url: session.url() },
                    actions: Some(vec![ActionId::OpenInBrowser.into()]),
                    preview: None,
                },
                command(catalog, "stop", "launcher.stop", "launcher.stop.detail"),
            ],
            Self::Failed(error) => vec![
                text::hint(
                    catalog.get("launcher.failed"),
                    catalog.format("launcher.failed.detail", &[("error", error)]),
                ),
                command(catalog, "start", "launcher.retry", "launcher.start.detail"),
            ],
            Self::Stopped | Self::Unopened => {
                vec![command(
                    catalog,
                    "start",
                    "launcher.start",
                    "launcher.start.detail",
                )]
            }
        }
    }
}

fn command(catalog: &Catalog, id: &str, title: &str, detail: &str) -> SearchItem {
    let title = catalog.get(title);
    SearchItem {
        intent_match: false,
        title: title.into(),
        description: catalog.get(detail).into(),
        icon: Some(ICON.into()),
        resource: Resource::PluginItem { id: id.into() },
        actions: Some(vec![ActionSpec {
            id: ActionId::Plugin,
            title: Some(title.into()),
            command: Some(id.into()),
            item: None,
        }]),
        preview: None,
    }
}

fn main() -> io::Result<()> {
    mytools_plugin_api::crash::install();
    let catalog = Catalog::for_language(ENGLISH, CHINESE).map_err(io::Error::other)?;
    let transfer = RefCell::new(Transfer::default());
    mytools_plugin_api::serve_requests(
        |request| {
            if !request.scoped {
                return SearchResponse::default();
            }
            let mut transfer = transfer.borrow_mut();
            if matches!(*transfer, Transfer::Unopened) {
                transfer.start();
            }
            SearchResponse {
                items: transfer.items(&catalog),
                ..Default::default()
            }
        },
        |request| {
            let mut transfer = transfer.borrow_mut();
            match request.command.as_str() {
                "start" if !matches!(*transfer, Transfer::Running(_)) => transfer.start(),
                "stop" => *transfer = Transfer::Stopped,
                _ => return SearchResponse::default(),
            }
            SearchResponse {
                effect: Some(PluginEffect {
                    refresh: true,
                    ..PluginEffect::stay()
                }),
                ..Default::default()
            }
        },
    )
}
