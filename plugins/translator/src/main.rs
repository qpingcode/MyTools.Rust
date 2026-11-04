mod cache;
mod client;
mod environment;
mod jsonc;
mod speech;
use cache::{Cache, Entry};
use mytools_plugin_api::{
    ActionId, PluginActionRequest, PluginEffect, Resource, SearchItem, SearchResponse,
    i18n::Catalog, text,
};
use std::{collections::HashMap, sync::Mutex};

#[derive(Clone)]
struct Translation {
    input: String,
    target: &'static str,
}

struct State {
    candidates: HashMap<String, Translation>,
    /// Failures stay in memory only, so a retry is always one Enter away.
    errors: HashMap<String, String>,
    cache: Cache,
    revision: u64,
}

impl State {
    fn new(cache: Cache) -> Self {
        Self {
            candidates: HashMap::new(),
            errors: HashMap::new(),
            cache,
            revision: 0,
        }
    }
}

fn request(query: &str) -> Result<Translation, String> {
    let (language, value) = query
        .split_once(char::is_whitespace)
        .filter(|(language, _)| matches!(*language, "en" | "zh" | "auto"))
        .unwrap_or(("auto", query));
    let input = text::input(value)?;
    let target = match language {
        "en" => "English",
        "zh" => "Simplified Chinese",
        _ if input.chars().any(is_han) => "English",
        _ => "Simplified Chinese",
    };
    Ok(Translation { input, target })
}

fn is_han(ch: char) -> bool {
    matches!(ch, '\u{3400}'..='\u{9fff}' | '\u{20000}'..='\u{323af}')
}

/// One English word such as "hello", "well-known" or "don't".
fn english_word(text: &str) -> bool {
    let text = text.trim();
    text.chars().count() <= 40
        && text.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphabetic() || matches!(ch, '-' | '\''))
}

/// Inputs that are likely a dictionary lookup, so the reply should carry the IPA too: an
/// English word, or a short run of Chinese characters that may translate to one.
fn is_lookup(input: &str) -> bool {
    let input = input.trim();
    english_word(input) || (input.chars().count() <= 8 && input.chars().all(is_han))
}

/// The English word to show the IPA for and read aloud.
fn spoken_word(entry: &Entry) -> Option<&str> {
    [entry.input.trim(), entry.translation.trim()]
        .into_iter()
        .find(|text| english_word(text))
}

fn speak_action(word: &str, catalog: &Catalog) -> mytools_plugin_api::ActionSpec {
    text::command("speak", catalog.get("speak").into()).with_item(word)
}

/// The translation (Enter copies it) and, for a single word, its IPA (Enter reads it).
fn entry_items(entry: &Entry, id: &str, description: String, catalog: &Catalog) -> Vec<SearchItem> {
    let word = spoken_word(entry);
    let mut translation = text::item(
        catalog.get("result"),
        entry.translation.clone(),
        description,
    );
    let mut actions = vec![ActionId::CopyText.into()];
    actions.extend(word.map(|word| speak_action(word, catalog)));
    actions.push(text::command("translate", catalog.get("retranslate").into()).with_item(id));
    translation.title = first_line(&entry.translation);
    translation.actions = Some(actions);
    let mut items = vec![translation];
    if let (Some(word), Some(phonetic)) = (word, &entry.phonetic) {
        let mut item = text::item(
            phonetic.clone(),
            String::new(),
            catalog.format("speak.detail", &[("word", word)]),
        );
        item.icon = Some("builtin:volume-2".into());
        item.resource = Resource::PluginItem {
            id: format!("speak:{word}"),
        };
        item.preview = None;
        item.actions = Some(vec![speak_action(word, catalog)]);
        items.push(item);
    }
    // Enter copies the definition or the example sentence; examples can also be read aloud.
    if let Some(definition) = &entry.definition {
        let mut item = text::item(
            definition.clone(),
            definition.clone(),
            catalog.get("definition"),
        );
        item.icon = Some("builtin:info".into());
        items.push(item);
    }
    for example in &entry.examples {
        let mut item = text::item(
            example.sentence.clone(),
            example.sentence.clone(),
            example.translation.clone(),
        );
        item.icon = Some("builtin:text-initial".into());
        item.actions = Some(vec![
            ActionId::CopyText.into(),
            speak_action(&example.sentence, catalog),
        ]);
        items.push(item);
    }
    items
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(text)
        .trim()
        .to_owned()
}

fn id(target: &str, input: &str) -> String {
    format!("{target}:{input}")
}

/// An empty query lists the recent translations, newest first, then the usage hint.
fn recent(state: &State, catalog: &Catalog) -> Vec<SearchItem> {
    let mut items: Vec<SearchItem> = state
        .cache
        .recent()
        .iter()
        .map(|entry| {
            let description = match &entry.phonetic {
                Some(phonetic) if spoken_word(entry).is_some() => {
                    format!("{} · {phonetic}", entry.input)
                }
                _ => entry.input.clone(),
            };
            let mut items = entry_items(
                entry,
                &id(&entry.target, &entry.input),
                description,
                catalog,
            );
            items.truncate(1); // The list stays one line per translation.
            items.remove(0)
        })
        .collect();
    items.push(text::hint(
        catalog.get("usage"),
        catalog.get("usage.detail"),
    ));
    items
}

fn search(state: &Mutex<State>, query: &str, catalog: &Catalog) -> SearchResponse {
    let mut state = state.lock().unwrap();
    if query.trim().is_empty() {
        return SearchResponse {
            items: recent(&state, catalog),
            revision: Some(state.revision),
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
    let id = id(request.target, &request.input);
    if state.candidates.len() >= 32 && !state.candidates.contains_key(&id) {
        state.candidates.clear();
    }
    state.candidates.insert(id.clone(), request.clone());
    // The list never holds a stale "Translate" next to a result, so after translating
    // the result is the selected item.
    let items = if let Some(entry) = state.cache.get(&request.input, request.target) {
        entry_items(entry, &id, catalog.get("copy").into(), catalog)
    } else if let Some(error) = state.errors.get(&id) {
        // The full error stays readable in the detail pane; Enter retries.
        let mut item = text::item(catalog.get("failed"), error.clone(), error.clone());
        item.actions = Some(vec![
            text::command("translate", catalog.get("retry").into()).with_item(id.clone()),
        ]);
        item.resource = Resource::PluginItem { id };
        vec![item]
    } else {
        let mut item = text::item(
            catalog.get("translate"),
            query.into(),
            catalog.get("translate.detail"),
        );
        item.actions = Some(vec![
            text::command("translate", catalog.get("translate").into()).with_item(id.clone()),
        ]);
        item.resource = Resource::PluginItem { id };
        vec![item]
    };
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
    state.lock().unwrap().cache.set_limit(config.cache_size);
    let item = action.item.unwrap_or_default();
    match action.command.as_str() {
        "speak" => {
            speech::speak(&item, &config.speech)
                .map_err(|error| catalog.format("speak.failed", &[("error", &error)]))?;
            Ok(PluginEffect {
                message: catalog.format("spoken", &[("word", &item)]),
                ..Default::default()
            })
        }
        "translate" => translate(state, &config, &item, catalog),
        _ => Err(catalog.get("missing").into()),
    }
}

fn translate(
    state: &Mutex<State>,
    config: &client::Config,
    item: &str,
    catalog: &Catalog,
) -> Result<PluginEffect, String> {
    if config.api_key.trim().is_empty() {
        return Err(catalog.get("key.missing").into());
    }
    let request = state
        .lock()
        .unwrap()
        .candidates
        .get(item)
        .cloned()
        .ok_or_else(|| catalog.get("missing").to_owned())?;
    // The host already executes actions off the UI thread. Reply only once the
    // translation has finished, then ask it to read the completed result once.
    let result = client::translate(
        config,
        &request.input,
        request.target,
        is_lookup(&request.input),
    );
    let mut state = state.lock().unwrap();
    let message = match result {
        Ok(output) => {
            state.errors.remove(item);
            state.cache.put(Entry {
                input: request.input,
                target: request.target.into(),
                translation: output.text,
                phonetic: output.phonetic,
                definition: output.definition,
                examples: output.examples,
            });
            "completed"
        }
        Err(error) => {
            if state.errors.len() >= 32 {
                state.errors.clear();
            }
            state.errors.insert(item.into(), error);
            "failed"
        }
    };
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
    let cache_size = client::Config::load()
        .map(|config| config.cache_size)
        .unwrap_or_else(|_| client::Config::default().cache_size);
    let state = Mutex::new(State::new(Cache::open(cache_size)));
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

    fn catalog() -> Catalog {
        Catalog::from_json(
            include_str!("../locales/en.json"),
            include_str!("../locales/en.json"),
        )
        .unwrap()
    }

    fn state() -> Mutex<State> {
        Mutex::new(State::new(Cache::at(None, 10)))
    }

    fn cached(state: &Mutex<State>, input: &str, translation: &str, phonetic: Option<&str>) {
        let target = request(input).unwrap().target;
        state.lock().unwrap().cache.put(Entry {
            input: input.into(),
            target: target.into(),
            translation: translation.into(),
            phonetic: phonetic.map(str::to_owned),
            definition: None,
            examples: vec![],
        });
    }

    #[test]
    fn detects_direction_and_keeps_requests_explicit() {
        assert_eq!(request("你好").unwrap().target, "English");
        assert_eq!(request("Hello").unwrap().target, "Simplified Chinese");
        assert_eq!(request("en Hello").unwrap().target, "English");
        let response = search(&state(), "hello", &catalog());
        assert_eq!(response.items.len(), 1);
        assert_eq!(
            response.items[0].actions.as_ref().unwrap()[0]
                .command
                .as_deref(),
            Some("translate")
        );
    }

    #[test]
    fn single_words_are_lookups_and_sentences_are_not() {
        assert!(is_lookup("hello"));
        assert!(is_lookup("well-known"));
        assert!(is_lookup("苹果"));
        assert!(!is_lookup("hello world"));
        assert!(!is_lookup("这是一个很长的中文句子啊"));
    }

    #[test]
    fn cached_word_shows_translation_then_phonetic_to_speak() {
        let state = state();
        cached(&state, "hello", "你好", Some("/həˈləʊ/"));
        let items = search(&state, "hello", &catalog()).items;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "你好");
        let actions = items[0].actions.as_ref().unwrap();
        assert_eq!(actions[0].id, ActionId::CopyText);
        assert_eq!(actions[1].command.as_deref(), Some("speak"));
        assert_eq!(actions[2].command.as_deref(), Some("translate"));
        assert_eq!(items[1].title, "/həˈləʊ/");
        let speak = &items[1].actions.as_ref().unwrap()[0];
        assert_eq!(speak.command.as_deref(), Some("speak"));
        assert_eq!(speak.item.as_deref(), Some("hello"));
    }

    #[test]
    fn word_lookups_add_definition_and_example_sentences() {
        let state = state();
        cached(&state, "apple", "苹果", Some("/ˈæpl/"));
        // One lock at a time: holding it while locking again would deadlock.
        let base = state.lock().unwrap().cache.recent()[0].clone();
        state.lock().unwrap().cache.put(Entry {
            definition: Some("a round fruit".into()),
            examples: vec![cache::Example {
                sentence: "I ate an apple.".into(),
                translation: "我吃了一个苹果。".into(),
            }],
            ..base
        });
        let items = search(&state, "apple", &catalog()).items;
        assert_eq!(
            items
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            ["苹果", "/ˈæpl/", "a round fruit", "I ate an apple."]
        );
        assert_eq!(
            items[2].resource,
            Resource::Text {
                text: "a round fruit".into()
            }
        );
        assert_eq!(items[3].description, "我吃了一个苹果。");
        let example_actions = items[3].actions.as_ref().unwrap();
        assert_eq!(example_actions[0].id, ActionId::CopyText);
        assert_eq!(example_actions[1].item.as_deref(), Some("I ate an apple."));
    }

    #[test]
    fn chinese_word_speaks_its_english_translation_and_sentences_have_no_phonetic() {
        let state = state();
        cached(&state, "苹果", "apple", Some("/ˈæpl/"));
        cached(&state, "good morning", "早上好", None);
        let items = search(&state, "苹果", &catalog()).items;
        assert_eq!(
            items[1].actions.as_ref().unwrap()[0].item.as_deref(),
            Some("apple")
        );
        assert_eq!(search(&state, "good morning", &catalog()).items.len(), 1);
    }

    #[test]
    fn empty_query_lists_recent_translations_newest_first() {
        let state = state();
        let items = search(&state, " ", &catalog()).items;
        assert_eq!(items.len(), 1, "only the usage hint without history");
        cached(&state, "hello", "你好", Some("/həˈləʊ/"));
        cached(&state, "苹果", "apple", None);
        let items = search(&state, "", &catalog()).items;
        assert_eq!(
            items
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            ["apple", "你好", "tr [en|zh|auto] <text or @file>"]
        );
        assert_eq!(items[1].description, "hello · /həˈləʊ/");
        assert_eq!(
            items[1].resource,
            Resource::Text {
                text: "你好".into()
            }
        );
    }

    #[test]
    fn failure_is_a_single_retry_item() {
        let state = state();
        let id = search(&state, "hello", &catalog()).items[0]
            .actions
            .as_ref()
            .unwrap()[0]
            .item
            .clone()
            .unwrap();
        state.lock().unwrap().errors.insert(id, "HTTP 401".into());
        let items = search(&state, "hello", &catalog()).items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].description, "HTTP 401");
        assert_eq!(
            items[0].actions.as_ref().unwrap()[0].command.as_deref(),
            Some("translate")
        );
    }
}
