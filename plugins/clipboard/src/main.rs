use mytools_plugin_api::{PluginActionRequest, PluginEffect, SearchResponse};
use std::sync::{Arc, Mutex};

mod clipboard;
mod i18n;
mod presentation;
mod store;

fn main() -> anyhow::Result<()> {
    let store = Arc::new(Mutex::new(store::Store::open(store::directory()?)?));
    if std::env::var_os("MYTOOLS_CLIPBOARD_NO_MONITOR").is_none() {
        clipboard::start(store.clone());
    }
    let cleaning = store.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
            let mut store = cleaning.lock().unwrap_or_else(|error| error.into_inner());
            if let Err(error) = store.cleanup() {
                store.last_error = Some(error.to_string());
            }
        }
    });
    let searching = store.clone();
    mytools_plugin_api::serve_interactive(
        move |query| {
            presentation::search(
                &searching.lock().unwrap_or_else(|error| error.into_inner()),
                query,
            )
        },
        move |request| match execute(&store, request) {
            Ok(effect) => SearchResponse {
                effect: Some(effect),
                ..Default::default()
            },
            Err(error) => SearchResponse {
                error: Some(error.to_string()),
                ..Default::default()
            },
        },
    )?;
    Ok(())
}

fn execute(
    store: &Mutex<store::Store>,
    request: PluginActionRequest,
) -> anyhow::Result<PluginEffect> {
    let mut state = store.lock().unwrap_or_else(|error| error.into_inner());
    match request.command.as_str() {
        "delete" => {
            state.delete(&request.item)?;
            Ok(PluginEffect {
                refresh: true,
                message: i18n::text("deleted"),
                ..Default::default()
            })
        }
        "clear" => {
            state.clear()?;
            Ok(PluginEffect {
                refresh: true,
                message: i18n::text("cleared"),
                ..Default::default()
            })
        }
        "paste" | "plain" | "copy" => {
            let entry = state
                .entries
                .iter()
                .find(|entry| entry.id == request.item)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!(i18n::text("missing")))?;
            anyhow::ensure!(
                request.command != "plain" || matches!(entry.content, store::Content::Text { .. }),
                i18n::text("unsupported")
            );
            let root = state.root.clone();
            drop(state);
            clipboard::restore(&entry, &root, request.command == "plain")?;
            Ok(PluginEffect {
                paste: request.command != "copy",
                message: i18n::text("copied"),
                ..Default::default()
            })
        }
        _ => anyhow::bail!(i18n::text("unsupported")),
    }
}
