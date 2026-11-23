//! Global search names plugins that only answer through their alias, so a plugin can be
//! found without remembering its alias: Enter opens its scope as if the alias was typed.
use crate::{actions::Action, catalog::Entry, i18n, plugins::Plugin};
use mytools_plugin_api::{Resource, hotkeys::Hotkey, matching::rank};

/// Entries for the scoped plugins whose name or an alias matches `query`.
pub fn entries(plugins: &[Plugin], query: &str) -> Vec<Entry> {
    plugins
        .iter()
        .filter(|plugin| scoped(plugin) && matches(plugin, query))
        .map(entry)
        .collect()
}

/// Global plugins already answer in global search, and without an alias there is no scope.
fn scoped(plugin: &Plugin) -> bool {
    plugin.process.is_some() && !plugin.global && !plugin.aliases.is_empty()
}

fn matches(plugin: &Plugin, query: &str) -> bool {
    let best = plugin
        .aliases
        .iter()
        .map(|alias| rank(alias, query))
        .fold(rank(&plugin.name, query), u8::min);
    !query.trim().is_empty() && best < 4
}

fn entry(plugin: &Plugin) -> Entry {
    let mut entry = Entry::new(
        plugin.name.clone(),
        description(plugin),
        plugin.icon.clone(),
        Resource::PluginItem {
            id: format!("scope:{}", plugin.id),
        },
        Some(&[]),
    )
    .expect("an empty action list fits every resource");
    entry.set_source(&plugin.id);
    entry.actions.push(Action::open_scope(&plugin.id));
    entry
}

/// "Alias gemini, glic · Shortcut Ctrl+Alt+G"; the shortcut part only when one is set.
fn description(plugin: &Plugin) -> String {
    let aliases = plugin
        .aliases
        .join(i18n::t("scope.alias_separator").as_str());
    match plugin.hotkey.as_deref().and_then(Hotkey::parse) {
        Some(hotkey) => i18n::format(
            "scope.description_hotkey",
            &[("aliases", &aliases), ("hotkey", &hotkey.to_string())],
        ),
        None => i18n::format("scope.description", &[("aliases", &aliases)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn plugin(id: &str, name: &str, aliases: &[&str]) -> Plugin {
        Plugin {
            id: id.into(),
            name: name.into(),
            version: "0.1.0".into(),
            icon: "builtin:sparkles".into(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
            global: false,
            tab: false,
            alias_timeout: crate::plugins::ALIAS_TIMEOUT,
            executable: std::env::temp_dir().join("mytools-fixture.exe"),
            process: Some(Arc::new(crate::search_process::SearchProcess::idle())),
            view: Default::default(),
            action_rules: vec![],
            hotkey: None,
            commands: vec![],
        }
    }

    fn names(plugins: &[Plugin], query: &str) -> Vec<String> {
        entries(plugins, query)
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }

    #[test]
    fn finds_scoped_plugins_by_name_or_alias() {
        let plugins = [
            plugin("gemini-chrome", "Gemini in Chrome", &["gemini", "glic"]),
            plugin("uuid", "UUID Generator", &["uuid"]),
        ];
        assert_eq!(names(&plugins, "gem"), ["Gemini in Chrome"]);
        assert_eq!(names(&plugins, "glic"), ["Gemini in Chrome"]);
        assert_eq!(names(&plugins, "generator"), ["UUID Generator"]);
        assert!(names(&plugins, "").is_empty());
        assert!(names(&plugins, "zzz").is_empty());
    }

    #[test]
    fn skips_plugins_that_have_no_scope_or_already_answer_globally() {
        let mut global = plugin("bookmarks", "Chrome Bookmarks", &["bm"]);
        global.global = true;
        let unaliased = plugin("awake", "Keep Awake", &[]);
        let mut application = plugin("app", "Some App", &["app"]);
        application.process = None;
        assert!(entries(&[global, unaliased, application], "e").is_empty());
    }

    #[test]
    fn describes_aliases_and_the_shortcut_only_when_set() {
        let mut gemini = plugin("gemini-chrome", "Gemini in Chrome", &["gemini", "glic"]);
        let plain = entries(std::slice::from_ref(&gemini), "gemini").remove(0);
        assert!(plain.description.contains("gemini") && plain.description.contains("glic"));
        assert!(!plain.description.contains("Ctrl"));
        gemini.hotkey = Some("Ctrl+Alt+G".into());
        let shortcut = entries(&[gemini], "gemini").remove(0);
        assert!(shortcut.description.contains("Ctrl+Alt+G"));
        assert_eq!(shortcut.actions.len(), 1);
    }
}
