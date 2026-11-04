mod client;
mod environment;
use mytools_plugin_api::{
    PluginActionRequest, PluginEffect, Resource, SearchResponse, i18n::Catalog, text,
};
use std::{collections::HashMap, sync::Mutex};

#[derive(Clone)]
struct Translation {
    input: String,
    target: &'static str,
}

#[derive(Default)]
struct State {
    candidates: HashMap<String, Translation>,
    results: HashMap<String, Result<String, String>>,
    revision: u64,
}

fn request(query: &str) -> Result<Translation, String> {
    let (language, value) = query
        .split_once(char::is_whitespace)
        .filter(|(language, _)| matches!(*language, "en" | "zh" | "auto"))
        .unwrap_or(("auto", query));
    let input = text::input(value)?;
    let chinese = input
        .chars()
        .any(|ch| matches!(ch, '\u{3400}'..='\u{9fff}' | '\u{20000}'..='\u{323af}'));
    let target = match language {
        "en" => "English",
        "zh" => "Simplified Chinese",
        _ if chinese => "English",
        _ => "Simplified Chinese",
    };
    Ok(Translation { input, target })
}

fn search(state: &Mutex<State>, query: &str, catalog: &Catalog) -> SearchResponse {
    if query.trim().is_empty() {
        return SearchResponse {
            items: vec![text::hint(
                catalog.get("usage"),
                catalog.get("usage.detail"),
            )],
            ..Default::default()
        };
    }
    let request = match request(query) {
        Ok(request) if !request.input.trim().is_empty() => request,
        _ => {
            return SearchResponse {
                error: Some(catalog.get("input.error").into()),
                ..Default::default()
            };
        }
    };
    let id = format!("{}:{}", request.target, request.input);
    let mut state = state.lock().unwrap();
    if state.candidates.len() >= 32 && !state.candidates.contains_key(&id) {
        state.candidates.clear();
    }
    state.candidates.insert(id.clone(), request);
    let mut items = vec![];
    if let Some(result) = state.results.get(&id) {
        match result {
            Ok(output) => items.push(text::item(
                catalog.get("result"),
                output.clone(),
                catalog.get("copy"),
            )),
            Err(error) => items.push(text::hint(catalog.get("failed"), error)),
        }
    }
    {
        let mut item = text::item(
            catalog.get("translate"),
            query.into(),
            catalog.get("translate.detail"),
        );
        item.actions = Some(vec![
            text::command("translate", catalog.get("translate").into()).with_item(id.clone()),
        ]);
        item.resource = Resource::PluginItem { id };
        items.push(item);
    }
    SearchResponse {
        items,
        revision: Some(state.revision),
        ..Default::default()
    }
}

fn start(
    state: &Mutex<State>,
    action: PluginActionRequest,
    catalog: &Catalog,
) -> Result<PluginEffect, String> {
    let config = client::Config::load()
        .map_err(|error| catalog.format("config.error", &[("error", &error)]))?;
    if config.api_key.trim().is_empty() {
        return Err(catalog.get("key.missing").into());
    }
    let item = action.item.unwrap_or_default();
    let locked = state.lock().unwrap();
    let request = locked
        .candidates
        .get(&item)
        .filter(|_| action.command == "translate")
        .cloned()
        .ok_or_else(|| catalog.get("missing").to_owned())?;
    drop(locked);
    // The host already executes actions off the UI thread. Reply only once the
    // translation has finished, then ask it to read the completed result once.
    let result = client::translate(&config, &request.input, request.target);
    let message = if result.is_ok() {
        "completed"
    } else {
        "failed"
    };
    let mut state = state.lock().unwrap();
    if state.results.len() >= 32 {
        state.results.clear();
    }
    state.results.insert(item, result);
    state.revision += 1;
    Ok(PluginEffect {
        refresh: true,
        message: catalog.get(message).into(),
        ..Default::default()
    })
}

fn main() -> std::io::Result<()> {
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    let state = Mutex::new(State::default());
    mytools_plugin_api::serve_interactive(
        |query| search(&state, query, &catalog),
        |action| match start(&state, action, &catalog) {
            Ok(effect) => SearchResponse {
                effect: Some(effect),
                ..Default::default()
            },
            Err(error) => SearchResponse {
                error: Some(error),
                ..Default::default()
            },
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_direction_and_keeps_requests_explicit() {
        assert_eq!(request("你好").unwrap().target, "English");
        assert_eq!(request("Hello").unwrap().target, "Simplified Chinese");
        assert_eq!(request("en Hello").unwrap().target, "English");
        let state = Mutex::new(State::default());
        let catalog = Catalog::from_json("{}", "{}").unwrap();
        let response = search(&state, "hello", &catalog);
        assert_eq!(response.items.len(), 1);
        assert_eq!(
            response.items[0].actions.as_ref().unwrap()[0]
                .command
                .as_deref(),
            Some("translate")
        );
    }
}
