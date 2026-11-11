use crate::i18n;
use crate::{
    actions::{Action, resolve_actions},
    plugins::{DisabledPlugin, LoadContext, Loaded, Plugin, manifest_paths},
    search_process::{SEARCH_TIMEOUT, STARTUP_TIMEOUT, SearchProcess},
    status::{Board, Notice},
};
use anyhow::Result;
use mytools_plugin_api::{ActionId, ActionSpec, Icon, Resource, SearchItem, plugin_status::State};
use std::{collections::HashSet, sync::Arc, time::Instant};

#[derive(Clone, Debug)]
pub struct Entry {
    rank_intent: bool,
    source_id: String,
    pub name: String,
    pub description: String,
    pub icon: Icon,
    pub resource: Resource,
    pub actions: Vec<Action>,
    pub preview: Option<mytools_plugin_api::Preview>,
}

impl Entry {
    pub fn new(
        name: String,
        description: String,
        icon: Icon,
        resource: Resource,
        specified: Option<&[ActionSpec]>,
    ) -> Result<Self> {
        let actions = resolve_actions(&resource, specified)?;
        let fallback = match &resource {
            Resource::Application { .. } => "builtin:app-window",
            Resource::File { .. } => "builtin:file",
            Resource::Folder { .. } => "builtin:folder",
            Resource::Url { .. } => "builtin:globe",
            Resource::Text { .. } => "builtin:file-text",
            Resource::PluginItem { .. } => "builtin:clipboard",
        };
        let icon = mytools_plugin_api::icons::resolve_reference(Some(&icon), &fallback.into());
        Ok(Self {
            rank_intent: false,
            source_id: String::new(),
            name,
            description,
            icon,
            resource,
            actions,
            preview: None,
        })
    }
}

#[derive(Clone)]
pub struct Catalog {
    plugins: Vec<Plugin>,
    /// Turned off in the settings; their aliases explain why nothing answers.
    disabled: Vec<DisabledPlugin>,
    errors: Vec<String>,
    /// When the plugin settings file was last changed as of this load.
    settings_stamp: Option<std::time::SystemTime>,
    board: Arc<Board>,
}

/// Host settings that apply to every plugin.
pub struct LoadOptions {
    /// How long a plugin may take to start unless its manifest says otherwise.
    pub startup_timeout: std::time::Duration,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            startup_timeout: STARTUP_TIMEOUT,
        }
    }
}

fn settings_stamp() -> Option<std::time::SystemTime> {
    std::fs::metadata(crate::plugins::settings_path())
        .and_then(|metadata| metadata.modified())
        .ok()
}

pub struct SearchResults {
    pub entries: Vec<Entry>,
    pub errors: Vec<String>,
    pub plugin_name: Option<String>,
    pub view: mytools_plugin_api::View,
    pub revision: Option<u64>,
    /// A newer query arrived while waiting; the results are incomplete and stale.
    pub cancelled: bool,
    /// A plugin this search needs is still starting: search again once it is ready.
    pub waiting: bool,
    /// For a global search while some plugins are not ready: (ready, expected).
    pub partial: Option<(usize, usize)>,
}

/// How one search runs: retries wait longer, and a newer query cancels the wait.
pub struct SearchOptions<'a> {
    /// 0 for a normal search; each retry doubles the alias timeout (capped).
    pub attempt: u32,
    pub cancelled: &'a dyn Fn() -> bool,
}

impl Default for SearchOptions<'_> {
    fn default() -> Self {
        Self {
            attempt: 0,
            cancelled: &|| false,
        }
    }
}

/// Global searches share one second; a search through an alias uses that plugin's own
/// limit, doubled per retry, never above the maximum.
fn search_timeout(plugin: &Plugin, targeted: bool, attempt: u32) -> std::time::Duration {
    if !targeted {
        return SEARCH_TIMEOUT;
    }
    plugin
        .alias_timeout
        .saturating_mul(1 << attempt.min(4))
        .min(crate::plugins::MAX_ALIAS_TIMEOUT)
}

enum Waited {
    Response(Result<mytools_plugin_api::SearchResponse>),
    TimedOut,
    Cancelled,
}

/// Waits in short slices so a newer query can stop the wait early.
fn wait(
    reply: &std::sync::mpsc::Receiver<Result<mytools_plugin_api::SearchResponse>>,
    deadline: Instant,
    cancelled: &dyn Fn() -> bool,
) -> Waited {
    use std::sync::mpsc::RecvTimeoutError;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Waited::TimedOut;
        }
        match reply.recv_timeout(left.min(std::time::Duration::from_millis(50))) {
            Ok(response) => return Waited::Response(response),
            Err(RecvTimeoutError::Timeout) if cancelled() => return Waited::Cancelled,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Waited::Response(Err(anyhow::anyhow!(i18n::t("error.process_exited"))));
            }
        }
    }
}

fn seconds(duration: std::time::Duration) -> String {
    format!("{}", duration.as_millis() as f64 / 1000.0)
}

fn retry_entry(plugin: &Plugin, input: &str, waited: std::time::Duration, attempt: u32) -> Entry {
    let next = search_timeout(plugin, true, attempt + 1);
    let mut entry = Entry::new(
        i18n::format("search.timed_out", &[("seconds", &seconds(waited))]),
        i18n::format(
            "search.timed_out_detail",
            &[("name", &plugin.name), ("seconds", &seconds(next))],
        ),
        "builtin:refresh-cw".into(),
        Resource::Text { text: input.into() },
        Some(&[]),
    )
    .expect("an empty action list fits every resource");
    entry.source_id = plugin.id.clone();
    entry.actions.push(Action::retry(attempt + 1));
    entry
}

impl Catalog {
    pub fn load() -> Self {
        Self::load_with(&LoadOptions::default())
    }

    /// Loads every plugin and starts the eager ones without waiting for them.
    pub fn load_with(options: &LoadOptions) -> Self {
        let settings_stamp = settings_stamp();
        let (settings, settings_error) = crate::plugins::load_settings();
        let board = Arc::new(Board::new(Some(mytools_plugin_api::plugin_status::path(
            &crate::plugins::settings_path(),
        ))));
        let mut catalog = Self {
            plugins: vec![],
            disabled: vec![],
            errors: settings_error.into_iter().collect(),
            settings_stamp,
            board: board.clone(),
        };
        let context = LoadContext {
            settings: &settings,
            board: &board,
            startup_timeout: options.startup_timeout,
        };
        let root = crate::plugins::directory();
        match manifest_paths(&root) {
            Ok(paths) => {
                let mut ids = HashSet::new();
                let mut aliases = HashSet::new();
                for path in paths {
                    match Plugin::load(&path, &context) {
                        Ok(Loaded::Disabled(plugin)) => catalog.disabled.push(plugin),
                        Ok(Loaded::Enabled(plugin)) => {
                            if !ids.insert(plugin.id.clone()) {
                                catalog.errors.push(i18n::format(
                                    "error.duplicate_id",
                                    &[("id", &plugin.id)],
                                ));
                            } else if plugin
                                .aliases
                                .iter()
                                .any(|alias| aliases.contains(&alias.to_lowercase()))
                            {
                                let error = i18n::format(
                                    "error.duplicate_alias",
                                    &[("name", &plugin.name)],
                                );
                                board.set(&plugin.id, State::Failed, Some(error.clone()));
                                catalog.errors.push(error);
                            } else {
                                aliases.extend(
                                    plugin.aliases.iter().map(|alias| alias.to_lowercase()),
                                );
                                catalog.plugins.push(plugin);
                            }
                        }
                        Err(err) => catalog.errors.push(format!("{}：{err:#}", path.display())),
                    }
                }
            }
            Err(err) => catalog.errors.push(i18n::format(
                "error.read_plugins",
                &[("error", &err.to_string())],
            )),
        }
        // Crashed, hung or killed resident plugins are restarted in the background.
        crate::search_process::watch(
            catalog
                .plugins
                .iter()
                .filter_map(|plugin| plugin.process.as_ref().map(std::sync::Arc::downgrade))
                .collect(),
        );
        catalog
    }

    /// Whether the plugin settings file changed since this catalog was loaded.
    pub fn settings_changed(&self) -> bool {
        settings_stamp() != self.settings_stamp
    }

    pub fn shutdown(&self) {
        for plugin in &self.plugins {
            if let Some(process) = &plugin.process {
                process.shutdown();
            }
        }
        self.board.retract();
    }

    /// Changes whenever any plugin changes state.
    pub fn revision(&self) -> u64 {
        self.board.revision()
    }

    /// Problems found since the last call, each reported once.
    pub fn take_notices(&self) -> Vec<Notice> {
        self.board.take_notices()
    }

    /// Restarts a plugin, e.g. one that failed to start; `false` if it has no process.
    pub fn restart(&self, id: &str) -> bool {
        let process = self
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .and_then(|plugin| plugin.process.as_ref());
        if let Some(process) = process {
            process.restart();
        }
        process.is_some()
    }

    /// Carries out what the settings window asked for, such as restarting a plugin.
    pub fn apply_requests(&self) {
        let settings = crate::plugins::settings_path();
        if let Some(requests) = mytools_plugin_api::plugin_status::take_requests(&settings) {
            for id in requests.restart {
                self.restart(&id);
            }
        }
    }

    /// Plugins with a process that should be running now, and how many of them are ready.
    fn loaded(&self) -> (usize, usize) {
        let states: Vec<State> = self
            .plugins
            .iter()
            .filter_map(|plugin| plugin.process.as_ref().map(|process| process.state().0))
            .filter(|state| *state != State::Idle)
            .collect();
        let ready = states
            .iter()
            .filter(|state| **state == State::Ready)
            .count();
        (ready, states.len())
    }

    pub fn alias_table(&self) -> AliasTable {
        AliasTable(
            self.plugins
                .iter()
                .map(|plugin| (plugin.aliases.clone(), plugin.name.clone()))
                .collect(),
        )
    }

    pub fn search(&self, input: &str) -> SearchResults {
        self.search_with(input, &SearchOptions::default())
    }

    pub fn search_with(&self, input: &str, options: &SearchOptions) -> SearchResults {
        let (target, query) = route(&self.plugins, input);
        let mut output = SearchResults {
            entries: vec![],
            errors: self.errors.clone(),
            plugin_name: target.map(|i| self.plugins[i].name.clone()),
            view: target.map(|i| self.plugins[i].view).unwrap_or_default(),
            revision: None,
            cancelled: false,
            waiting: false,
            partial: None,
        };
        if target.is_none()
            && let (Some(index), _) = route_by(
                self.disabled.iter().map(|plugin| plugin.aliases.as_slice()),
                input,
            )
        {
            let plugin = &self.disabled[index];
            output.plugin_name = Some(plugin.name.clone());
            output.entries.push(status_entry(
                &plugin.id,
                i18n::format("plugin.disabled_title", &[("name", &plugin.name)]),
                i18n::t("plugin.disabled_detail"),
                "builtin:power",
                input,
            ));
            return output;
        }
        let timeout = |plugin: &Plugin| search_timeout(plugin, target.is_some(), options.attempt);
        let mut pending = vec![];
        // Submit to every eligible plugin before waiting, so plugins run concurrently.
        for (index, plugin) in self.plugins.iter().enumerate() {
            if !eligible(index, plugin.global, target) {
                continue;
            }
            if let Some(process) = &plugin.process {
                if !ready(process, plugin, input, target.is_some(), &mut output) {
                    continue;
                }
                match process.begin(query, timeout(plugin), target.is_some()) {
                    Ok(reply) => pending.push((plugin, reply)),
                    Err(err) => output.errors.push(format!("{}：{err}", plugin.name)),
                }
            } else if target.is_some() || matches_query(&plugin.name, &plugin.aliases, query) {
                match Entry::new(
                    plugin.name.clone(),
                    format!("v{} · {}", plugin.version, plugin.executable.display()),
                    plugin.icon.clone(),
                    Resource::Application {
                        path: plugin.executable.clone(),
                    },
                    Some(&[
                        ActionId::Launch.into(),
                        ActionId::CopyPath.into(),
                        ActionId::Reveal.into(),
                    ]),
                ) {
                    Ok(mut entry) => {
                        entry.source_id = plugin.id.clone();
                        output.entries.push(entry);
                    }
                    Err(error) => output.errors.push(format!("{}：{error}", plugin.name)),
                }
            }
        }
        let started = Instant::now();
        for (plugin, reply) in pending {
            match wait(&reply, started + timeout(plugin), options.cancelled) {
                Waited::Cancelled => {
                    output.cancelled = true;
                    return output;
                }
                Waited::Response(Ok(response)) => {
                    if target.is_some() {
                        output.revision = response.revision;
                    }
                    if let Some(error) = response.error {
                        output.errors.push(format!("{}：{error}", plugin.name));
                    }
                    for item in response.items {
                        let title = item.title.clone();
                        match plugin_entry(plugin, item) {
                            Ok(entry) => output.entries.push(entry),
                            Err(error) => output
                                .errors
                                .push(format!("{} · {title}：{error}", plugin.name)),
                        }
                    }
                }
                // The plugin is not killed: it skips or finishes the abandoned search, and
                // the heartbeat restarts it only if it really stops responding.
                Waited::TimedOut if target.is_some() => output.entries.push(retry_entry(
                    plugin,
                    input,
                    timeout(plugin),
                    options.attempt,
                )),
                Waited::TimedOut => {
                    output
                        .errors
                        .push(format!("{}：{}", plugin.name, i18n::t("error.timeout")))
                }
                Waited::Response(Err(error)) => {
                    output.errors.push(format!("{}：{error}", plugin.name));
                }
            }
        }
        if target.is_none() {
            let (ready, expected) = self.loaded();
            output.partial = (ready < expected).then_some((ready, expected));
        }
        if target.is_none() && !query.is_empty() {
            rank_global(
                &mut output.entries,
                query,
                &crate::history::History::load(),
                crate::history::now(),
            );
        }
        output.entries.truncate(50);
        self.attach_contributed_actions(&mut output.entries);
        output
    }

    /// Appends actions other plugins contribute through `contributes.actions` rules, such
    /// as "Open in Rider" on every path.
    fn attach_contributed_actions(&self, entries: &mut [Entry]) {
        for entry in entries {
            if entry.actions.is_empty() {
                continue; // An explicit `[]` means the result is informational only.
            }
            let item = contributed_item(&entry.resource);
            for plugin in &self.plugins {
                let Some(process) = &plugin.process else {
                    continue;
                };
                for rule in &plugin.action_rules {
                    if !rule.when.matches(&entry.resource, &entry.source_id) {
                        continue;
                    }
                    for action in &rule.actions {
                        entry
                            .actions
                            .push(Action::contributed(action, item.clone(), process));
                    }
                }
            }
        }
    }
}

/// Whether a plugin can be searched now. A lazy plugin is started on first need; for a
/// search through its alias, a plugin that is not ready explains itself with one result.
fn ready(
    process: &SearchProcess,
    plugin: &Plugin,
    input: &str,
    targeted: bool,
    output: &mut SearchResults,
) -> bool {
    let (mut state, detail) = process.state();
    if state == State::Idle {
        process.activate();
        state = process.state().0;
    }
    match state {
        State::Ready => return true,
        State::Idle | State::Starting => output.waiting = true,
        State::Failed | State::Disabled => {}
    }
    if targeted {
        output.entries.push(match state {
            State::Failed | State::Disabled => status_entry(
                &plugin.id,
                i18n::format("plugin.failed_title", &[("name", &plugin.name)]),
                i18n::format(
                    "plugin.failed_detail",
                    &[("reason", &detail.unwrap_or_default())],
                ),
                "builtin:triangle-alert",
                input,
            ),
            _ => status_entry(
                &plugin.id,
                i18n::format("plugin.starting_title", &[("name", &plugin.name)]),
                detail.unwrap_or_else(|| i18n::t("plugin.starting_detail")),
                "builtin:clock",
                input,
            ),
        });
    }
    false
}

/// An informational result describing a plugin rather than coming from it.
fn status_entry(id: &str, title: String, description: String, icon: &str, input: &str) -> Entry {
    let mut entry = Entry::new(
        title,
        description,
        icon.into(),
        Resource::Text { text: input.into() },
        Some(&[]),
    )
    .expect("an empty action list fits every resource");
    entry.source_id = id.into();
    entry
}

/// The value sent as `item` when a contributed action runs.
fn contributed_item(resource: &Resource) -> String {
    match resource {
        Resource::File { path } | Resource::Folder { path } | Resource::Application { path } => {
            mytools_plugin_api::paths::display(path)
        }
        Resource::Url { url } => url.clone(),
        Resource::Text { text } => text.clone(),
        Resource::PluginItem { id } => id.clone(),
    }
}

fn rank_global(entries: &mut [Entry], query: &str, history: &crate::history::History, time: u64) {
    use crate::history::Selection;
    use mytools_plugin_api::matching::{normalize, rank};
    for entry in entries.iter_mut() {
        let selection = Selection::new(&entry.source_id, &entry.resource, query);
        if let Some(action) = entry.actions.first_mut() {
            action.selection = Some(selection);
        }
    }
    let reused = history.reused(
        query,
        entries.iter().filter_map(|entry| {
            entry
                .actions
                .first()?
                .selection
                .as_ref()
                .map(|s| s.key.as_str())
        }),
        time,
    );
    let query_length = normalize(query).chars().count() as u64;
    entries.sort_by_cached_key(|entry| {
        let selection = Selection::new(&entry.source_id, &entry.resource, query);
        let length = normalize(&entry.name).chars().count().max(1) as u64;
        let quality = (query_length * 1000 / length).min(1000);
        let score = quality * 70 + u64::from(history.frequency(&selection.key, time)) * 5;
        (
            !entry.rank_intent,
            rank(&entry.name, query),
            std::cmp::Reverse(score),
            normalize(&entry.name),
            selection.key,
        )
    });
    // Promote exactly one recalled, executable result, after explicit intent answers.
    let first_ordinary = entries.iter().take_while(|entry| entry.rank_intent).count();
    if let Some(index) = entries.iter().position(|entry| {
        !entry.rank_intent
            && entry
                .actions
                .first()
                .and_then(|action| action.selection.as_ref())
                .is_some_and(|selection| Some(selection.key.as_str()) == reused.as_deref())
    }) {
        entries[first_ordinary..=index].rotate_right(1);
    }
}

fn plugin_entry(plugin: &Plugin, item: SearchItem) -> Result<Entry> {
    let intent = plugin.id == "calculator" && item.intent_match;
    let mut entry = Entry::new(
        item.title,
        format!("{} · {}", plugin.name, item.description),
        mytools_plugin_api::icons::resolve_reference(item.icon.as_ref(), &plugin.icon),
        item.resource,
        item.actions.as_deref(),
    )?;
    entry.source_id = plugin.id.clone();
    entry.rank_intent = intent && !entry.actions.is_empty();
    entry.preview = item.preview;
    if let Some(process) = &plugin.process {
        for action in &mut entry.actions {
            action.bind_plugin(process);
        }
    }
    Ok(entry)
}

fn route<'a>(plugins: &[Plugin], input: &'a str) -> (Option<usize>, &'a str) {
    route_by(
        plugins.iter().map(|plugin| plugin.aliases.as_slice()),
        input,
    )
}

/// `alias + whitespace + query` selects the plugin with that alias, ignoring ASCII case.
fn route_by<'p, 'a>(
    aliases: impl Iterator<Item = &'p [String]>,
    input: &'a str,
) -> (Option<usize>, &'a str) {
    let input = input.trim_start();
    if let Some((alias, query)) = input.split_once(char::is_whitespace) {
        let mut aliases = aliases;
        if let Some(index) = aliases.position(|candidates| {
            candidates
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(alias))
        }) {
            return (Some(index), query.trim());
        }
    }
    (None, input.trim())
}

/// Plugin aliases and names, so the UI thread can name the plugin a query will search
/// without waiting for the worker that owns the catalog.
#[derive(Clone, Debug, Default)]
pub struct AliasTable(Vec<(Vec<String>, String)>);

impl AliasTable {
    pub fn plugin_for(&self, input: &str) -> Option<&str> {
        let (index, _) = route_by(self.0.iter().map(|(aliases, _)| aliases.as_slice()), input);
        index.map(|index| self.0[index].1.as_str())
    }
}

fn eligible(index: usize, global: bool, target: Option<usize>) -> bool {
    target.map_or(global, |selected| selected == index)
}

fn matches_query(name: &str, aliases: &[String], query: &str) -> bool {
    query.to_lowercase().split_whitespace().all(|word| {
        name.to_lowercase().contains(word)
            || aliases
                .iter()
                .any(|alias| alias.to_lowercase().contains(word))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_icon_overrides_plugin_and_invalid_icons_inherit() {
        let mut plugin = plugin();
        plugin.icon = "builtin:calculator".into();
        for (icon, expected) in [
            (None, "builtin:calculator"),
            (Some("builtin:circle-check"), "builtin:circle-check"),
            (Some("builtin:missing"), "builtin:calculator"),
            (Some(""), "builtin:calculator"),
            (Some("＝"), "＝"),
        ] {
            let item = SearchItem {
                intent_match: false,
                title: "2".into(),
                description: String::new(),
                icon: icon.map(Icon::from),
                resource: Resource::Text { text: "2".into() },
                actions: None,
                preview: None,
            };
            assert_eq!(plugin_entry(&plugin, item).unwrap().icon, expected.into());
        }
    }

    #[test]
    fn host_results_default_to_resource_icons() {
        for (resource, expected) in [
            (Resource::Text { text: "2".into() }, "builtin:file-text"),
            (
                Resource::Url {
                    url: "https://example.com".into(),
                },
                "builtin:globe",
            ),
            (
                Resource::Folder {
                    path: std::env::temp_dir(),
                },
                "builtin:folder",
            ),
            (
                Resource::File {
                    path: std::env::temp_dir().join("note.txt"),
                },
                "builtin:file",
            ),
            (
                Resource::Application {
                    path: std::env::temp_dir().join("app.exe"),
                },
                "builtin:app-window",
            ),
        ] {
            let entry = Entry::new(
                "result".into(),
                String::new(),
                "builtin:missing".into(),
                resource,
                None,
            )
            .unwrap();
            assert_eq!(entry.icon, expected.into());
        }
    }

    fn ranked_entry(name: &str) -> Entry {
        let mut entry = Entry::new(
            name.into(),
            String::new(),
            "builtin:file".into(),
            Resource::PluginItem { id: name.into() },
            Some(&[ActionSpec {
                id: ActionId::Plugin,
                title: None,
                command: Some("copy".into()),
                item: Some(name.into()),
            }]),
        )
        .unwrap();
        entry.source_id = "fixture".into();
        entry
    }

    #[test]
    fn reuse_overrides_text_but_never_explicit_intent_and_promotes_only_one() {
        let weak = ranked_entry("development environment");
        let key = crate::history::Selection::new("fixture", &weak.resource, "dev").key;
        let history = serde_json::from_value(
            serde_json::json!({"records":[{"query":"dev","key":key,"last":100,"uses":1.0}]}),
        )
        .unwrap();
        let mut answer = ranked_entry("2");
        answer.rank_intent = true;
        let mut entries = vec![ranked_entry("dev"), weak.clone(), weak, answer];
        rank_global(&mut entries, "dev", &history, 101);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec![
                "2",
                "development environment",
                "dev",
                "development environment"
            ]
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.actions[0].selection.as_ref().unwrap().query == "dev")
        );
        rank_global(&mut entries, "dev", &history, 15 * 86_400_000);
        assert_eq!(entries[1].name, "dev");
    }

    #[test]
    fn frequency_adjusts_within_tier_and_cannot_override_stronger_match() {
        let frequent = ranked_entry("z dev");
        let key = crate::history::Selection::new("fixture", &frequent.resource, "other").key;
        let history = serde_json::from_value(
            serde_json::json!({"records":[{"query":"other","key":key,"last":100,"uses":1000.0}]}),
        )
        .unwrap();
        let mut entries = vec![ranked_entry("a dev"), frequent, ranked_entry("dev")];
        rank_global(&mut entries, "dev", &history, 101);
        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["dev", "z dev", "a dev"]
        );
        rank_global(&mut entries, "dev", &history, 1000 * 86_400_000);
        assert_eq!(entries[1].name, "a dev");
    }

    fn plugin() -> Plugin {
        Plugin {
            id: "calculator".into(),
            name: "计算器".into(),
            version: "0.1.0".into(),
            icon: "=".into(),
            aliases: vec!["calc".into(), "计算".into()],
            global: false,
            alias_timeout: crate::plugins::ALIAS_TIMEOUT,
            executable: std::env::temp_dir().join("mytools-fixture.exe"),
            process: None,
            view: Default::default(),
            action_rules: vec![],
        }
    }

    fn catalog(plugins: Vec<Plugin>) -> Catalog {
        Catalog {
            plugins,
            disabled: vec![],
            errors: vec![],
            settings_stamp: None,
            board: Default::default(),
        }
    }

    #[test]
    fn alias_requires_whitespace_and_preserves_expression() {
        let plugins = vec![plugin()];
        assert_eq!(route(&plugins, "calc"), (None, "calc"));
        assert_eq!(route(&plugins, "calc "), (Some(0), ""));
        assert_eq!(route(&plugins, " CALC  1 + 1 "), (Some(0), "1 + 1"));
        assert_eq!(route(&plugins, "计算 2+2"), (Some(0), "2+2"));
        assert_eq!(route(&plugins, "other 1+1"), (None, "other 1+1"));
        assert_eq!(
            route(&plugins, "calcSomething 1+1"),
            (None, "calcSomething 1+1")
        );
    }

    #[test]
    fn alias_table_names_the_routed_plugin_like_search_does() {
        let table = AliasTable(vec![(vec!["calc".into(), "计算".into()], "计算器".into())]);
        assert_eq!(table.plugin_for(" CALC 1+1"), Some("计算器"));
        assert_eq!(table.plugin_for("计算 2"), Some("计算器"));
        assert_eq!(table.plugin_for("calc"), None);
        assert_eq!(table.plugin_for("other 1"), None);
    }

    #[test]
    fn global_false_is_excluded_unless_selected_by_alias() {
        assert!(!eligible(0, false, None));
        assert!(eligible(0, true, None));
        assert!(eligible(0, false, Some(0)));
        assert!(!eligible(1, true, Some(0)));
        let mut rider = plugin();
        rider.name = "Rider".into();
        rider.aliases = vec!["rider".into()];
        rider.global = true;
        let catalog = catalog(vec![plugin(), rider]);
        assert!(catalog.search("计算器").entries.is_empty());
        let scoped = catalog.search("calc 1+1");
        assert_eq!(scoped.entries.len(), 1);
        assert_eq!(scoped.entries[0].name, "计算器");
        assert_eq!(scoped.plugin_name.as_deref(), Some("计算器"));
        assert_eq!(catalog.search("RIDER").entries[0].name, "Rider");
    }

    #[test]
    fn disabled_alias_explains_instead_of_searching_everything() {
        let mut catalog = catalog(vec![plugin()]);
        catalog.disabled.push(DisabledPlugin {
            id: "translator".into(),
            name: "翻译".into(),
            aliases: vec!["tr".into()],
        });
        let results = catalog.search("tr hello");
        assert_eq!(results.entries.len(), 1);
        assert_eq!(results.plugin_name.as_deref(), Some("翻译"));
        assert!(results.entries[0].actions.is_empty());
        // An enabled plugin's alias wins over a disabled one's.
        catalog.disabled[0].aliases = vec!["calc".into()];
        assert_eq!(catalog.search("calc 1").entries[0].name, "计算器");
    }

    #[cfg(windows)]
    #[test]
    fn aggregates_persistent_processes_and_routes_only_selected_plugin() {
        use crate::search_process::SearchProcess;
        use std::sync::Arc;
        let mut first = plugin();
        first.global = true;
        first.process = Some(Arc::new(SearchProcess::fixture()));
        let mut second = first.clone();
        second.id = "second".into();
        second.name = "第二个插件".into();
        second.aliases = vec!["second".into()];
        second.process = Some(Arc::new(SearchProcess::fixture()));
        let mut catalog = catalog(vec![first, second]);
        let results = catalog.search("1+1");
        assert!(results.errors.is_empty(), "{:?}", results.errors);
        assert_eq!(results.entries.len(), 2);
        assert!(
            results
                .entries
                .iter()
                .all(|entry| entry.name == "1+1" && entry.description.ends_with('2'))
        );
        catalog.plugins[1].global = false;
        let results = catalog.search("2+2");
        assert_eq!(results.entries.len(), 1);
        assert!(results.entries[0].description.ends_with('3'));
        let scoped = catalog.search("second 3+3");
        assert!(scoped.errors.is_empty(), "{:?}", scoped.errors);
        assert_eq!(scoped.entries.len(), 1);
        assert_eq!(scoped.entries[0].name, "3+3");
        // Second plugin was not invoked for global search while disabled.
        assert!(scoped.entries[0].description.ends_with('3'));
        let first_again = catalog.search("calc 4+4");
        assert_eq!(first_again.entries.len(), 1);
        assert!(first_again.entries[0].description.ends_with('4'));
    }
}
