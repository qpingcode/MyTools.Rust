mod cache;
mod client;
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
        _ => automatic_target(&input),
    };
    Ok(Translation { input, target })
}

/// Chinese becomes English and anything else Chinese.
fn automatic_target(input: &str) -> &'static str {
    if input.chars().any(is_han) {
        "English"
    } else {
        "Simplified Chinese"
    }
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

/// The whole translation as the single result of the editor view: the right side shows the
/// translation and, for a word, its IPA, definition and examples, all selectable. Ctrl+Enter
/// copies the translation alone; Ctrl+K reads the word aloud or translates again.
fn entry_item(entry: &Entry, id: &str, catalog: &Catalog) -> SearchItem {
    let word = spoken_word(entry);
    let mut item = text::editor_result(
        catalog.get("result"),
        entry.translation.clone(),
        details(entry, word),
    );
    let mut actions = vec![ActionId::CopyText.into()];
    actions.extend(word.map(|word| speak_action(word, catalog)));
    actions.push(text::command("translate", catalog.get("retranslate").into()).with_item(id));
    item.actions = Some(actions);
    item
}

/// The translation, then the IPA, definition and example sentences, separated by blank lines.
fn details(entry: &Entry, word: Option<&str>) -> String {
    let mut sections = vec![entry.translation.clone()];
    if word.is_some() {
        sections.extend(entry.phonetic.clone());
    }
    sections.extend(entry.definition.clone());
    sections.extend(
        entry
            .examples
            .iter()
            .map(|example| format!("{}\n{}", example.sentence, example.translation)),
    );
    sections.join("\n\n")
}

fn id(target: &str, input: &str) -> String {
    format!("{target}:{input}")
}

fn search(state: &Mutex<State>, query: &str, catalog: &Catalog) -> SearchResponse {
    let mut state = state.lock().unwrap();
    if query.trim().is_empty() {
        return SearchResponse {
            items: vec![text::hint(
                catalog.get("usage"),
                catalog.get("usage.detail"),
            )],
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
    // One result at a time: the translation replaces "Translate" once it is done.
    let items = if let Some(entry) = state.cache.get(&request.input, request.target) {
        vec![entry_item(entry, &id, catalog)]
    } else if let Some(error) = state.errors.get(&id) {
        // The editor shows the title and the full error; Ctrl+Enter retries.
        let mut item = text::editor_prompt(catalog.get("failed"), error.clone());
        item.actions = Some(vec![
            text::command("translate", catalog.get("retry").into()).with_item(id.clone()),
        ]);
        item.resource = Resource::PluginItem { id };
        vec![item]
    } else {
        let mut item =
            text::editor_prompt(catalog.get("translate"), catalog.get("translate.detail"));
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
        "translate-selection" => translate_selection(state, &config, &item, catalog),
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
    let message = match fetch(state, config, request) {
        Ok(_) => {
            state.lock().unwrap().errors.remove(item);
            "completed"
        }
        Err(error) => {
            let mut state = state.lock().unwrap();
            if state.errors.len() >= 32 {
                state.errors.clear();
            }
            state.errors.insert(item.into(), error);
            "failed"
        }
    };
    state.lock().unwrap().revision += 1;
    Ok(PluginEffect {
        refresh: true,
        message: catalog.get(message).into(),
        ..Default::default()
    })
}

/// Translates `request`, or takes it from the cache, and keeps the result there.
fn fetch(
    state: &Mutex<State>,
    config: &client::Config,
    request: Translation,
) -> Result<String, String> {
    if let Some(entry) = state
        .lock()
        .unwrap()
        .cache
        .get(&request.input, request.target)
    {
        return Ok(entry.translation.clone());
    }
    let output = client::translate(
        config,
        &request.input,
        request.target,
        is_lookup(&request.input),
    )?;
    let translation = output.text.clone();
    state.lock().unwrap().cache.put(Entry {
        input: request.input,
        target: request.target.into(),
        translation: output.text,
        phonetic: output.phonetic,
        definition: output.definition,
        examples: output.examples,
    });
    Ok(translation)
}

/// The `translate-selection` command: the host sends the selected text and pastes the
/// translation over it, or opens `tr` with the text when the selection cannot be edited
/// (the translation is cached by then, so it shows at once).
fn translate_selection(
    state: &Mutex<State>,
    config: &client::Config,
    selection: &str,
    catalog: &Catalog,
) -> Result<PluginEffect, String> {
    let input = selection.trim();
    if input.is_empty() || input.len() > text::INPUT_LIMIT {
        return Err(catalog.get("input.error").into());
    }
    if config.api_key.trim().is_empty() {
        return Err(catalog.get("key.missing").into());
    }
    let request = Translation {
        input: input.to_owned(),
        target: automatic_target(input),
    };
    let translation = fetch(state, config, request)
        .map_err(|error| catalog.format("selection.failed", &[("error", &error)]))?;
    state.lock().unwrap().revision += 1;
    Ok(PluginEffect {
        copy_text: Some(translation),
        paste: true,
        message: catalog.get("completed").into(),
        ..Default::default()
    })
}

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
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

    fn details_of(item: &SearchItem) -> &str {
        &item.preview.as_ref().unwrap().text
    }

    #[test]
    fn cached_word_is_one_result_that_copies_and_speaks() {
        let state = state();
        cached(&state, "hello", "你好", Some("/həˈləʊ/"));
        let items = search(&state, "hello", &catalog()).items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Translation");
        assert_eq!(details_of(&items[0]), "你好\n\n/həˈləʊ/");
        assert_eq!(
            items[0].resource,
            Resource::Text {
                text: "你好".into()
            }
        );
        let actions = items[0].actions.as_ref().unwrap();
        assert_eq!(actions[0].id, ActionId::CopyText);
        assert_eq!(actions[1].command.as_deref(), Some("speak"));
        assert_eq!(actions[1].item.as_deref(), Some("hello"));
        assert_eq!(actions[2].command.as_deref(), Some("translate"));
    }

    #[test]
    fn word_lookups_show_definition_and_example_sentences_below_the_translation() {
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
        assert_eq!(items.len(), 1);
        assert_eq!(
            details_of(&items[0]),
            "苹果\n\n/ˈæpl/\n\na round fruit\n\nI ate an apple.\n我吃了一个苹果。"
        );
    }

    #[test]
    fn chinese_word_speaks_its_english_translation_and_sentences_have_no_phonetic() {
        let state = state();
        cached(&state, "苹果", "apple", Some("/ˈæpl/"));
        cached(&state, "good morning", "早上好", None);
        let items = search(&state, "苹果", &catalog()).items;
        assert_eq!(
            items[0].actions.as_ref().unwrap()[1].item.as_deref(),
            Some("apple")
        );
        let sentence = search(&state, "good morning", &catalog()).items;
        assert_eq!(details_of(&sentence[0]), "早上好");
        assert_eq!(sentence[0].actions.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn empty_query_shows_only_the_usage() {
        let state = state();
        cached(&state, "hello", "你好", Some("/həˈləʊ/"));
        for query in ["", " "] {
            let items = search(&state, query, &catalog()).items;
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].title, "tr [en|zh|auto] <text or @file>");
        }
    }

    #[test]
    fn selection_translation_comes_from_the_cache_and_is_pasted() {
        let state = state();
        cached(&state, "hello", "你好", None);
        let config = client::Config {
            api_key: "key".into(),
            ..Default::default()
        };
        let effect = translate_selection(&state, &config, " hello\r\n", &catalog()).unwrap();
        assert_eq!(effect.copy_text.as_deref(), Some("你好"));
        assert!(effect.paste);
        assert!(translate_selection(&state, &config, " \n", &catalog()).is_err());
        let missing = client::Config::default();
        assert!(translate_selection(&state, &missing, "hello", &catalog()).is_err());
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
