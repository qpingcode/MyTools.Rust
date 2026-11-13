//! Commands plugins declare in their manifests: the host finds them in search and binds their
//! shortcuts without asking the plugin, and runs one by sending its id as a plugin action.
use crate::{
    actions::Action, catalog::Entry, i18n, plugins::Plugin, search_process::SearchProcess,
};
use anyhow::{Result, bail};
use mytools_plugin_api::{
    Icon, Resource,
    commands::{self, CommandInput, CommandManifest},
    hotkeys::Hotkey,
    matching::rank,
    plugin_settings::Settings,
};
use std::{collections::HashSet, path::Path, sync::Arc};

#[derive(Clone, Debug)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub keywords: Vec<String>,
    pub icon: Icon,
    /// The shortcut in effect, as the user or the manifest wrote it.
    pub hotkey: Option<String>,
    pub input: CommandInput,
}

/// Reads a plugin's commands with the user's shortcut overrides applied.
pub fn load(
    plugin_id: &str,
    manifests: &[CommandManifest],
    settings: &Settings,
    directory: &Path,
    plugin_icon: &Icon,
) -> Result<Vec<Command>> {
    let language = mytools_plugin_api::i18n::language();
    let mut ids = HashSet::new();
    let mut loaded = Vec::new();
    for manifest in manifests {
        if !commands::valid_id(&manifest.id) || !ids.insert(manifest.id.as_str()) {
            bail!(i18n::format("error.command_id", &[("id", &manifest.id)]));
        }
        let icon = manifest
            .icon
            .as_deref()
            .filter(|icon| icon.starts_with("builtin:"))
            .map(Icon::from);
        let icon = mytools_plugin_api::icons::resolve_reference(icon.as_ref(), plugin_icon);
        loaded.push(Command {
            id: manifest.id.clone(),
            title: manifest.title.resolve(directory, language),
            keywords: manifest.keywords.clone(),
            icon,
            hotkey: settings
                .command_hotkey(plugin_id, &manifest.id, manifest.hotkey.as_deref())
                .map(str::to_owned),
            input: manifest.input,
        });
    }
    Ok(loaded)
}

/// How a command is shown beside its shortcut, e.g. in errors: "Clipboard · Paste as Text".
pub fn label(plugin: &Plugin, command: &Command) -> String {
    format!("{} · {}", plugin.name, command.title)
}

/// The action that runs a command; it starts a lazy plugin when needed.
pub fn action(plugin: &Plugin, command: &Command, process: &Arc<SearchProcess>) -> Action {
    let selection = (command.input == CommandInput::Selection)
        .then(|| plugin.aliases.first().map(|alias| format!("{alias} ")));
    Action::command(
        &command.id,
        &command.title,
        &command.icon,
        process,
        selection,
    )
}

fn matches(plugin: &Plugin, command: &Command, query: &str) -> bool {
    query.is_empty()
        || rank(&command.title, query) < 4
        || command.keywords.iter().any(|word| rank(word, query) < 4)
        || rank(&plugin.name, query) < 4
}

/// Search results for the commands that match `query`; every command when it is empty.
/// They answer global search only, never a plugin's alias scope.
pub fn entries(plugins: &[Plugin], query: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    for plugin in plugins {
        let Some(process) = &plugin.process else {
            continue;
        };
        for command in &plugin.commands {
            if matches(plugin, command, query) {
                entries.push(entry(plugin, command, process));
            }
        }
    }
    entries
}

fn entry(plugin: &Plugin, command: &Command, process: &Arc<SearchProcess>) -> Entry {
    let description = match command.hotkey.as_deref().and_then(Hotkey::parse) {
        Some(hotkey) => i18n::format(
            "command.description_hotkey",
            &[("plugin", &plugin.name), ("hotkey", &hotkey.to_string())],
        ),
        None => i18n::format("command.description", &[("plugin", &plugin.name)]),
    };
    let mut entry = Entry::new(
        command.title.clone(),
        description,
        command.icon.clone(),
        Resource::PluginItem {
            id: format!("command:{}", command.id),
        },
        Some(&[]),
    )
    .expect("an empty action list fits every resource");
    entry.set_source(&plugin.id);
    entry.actions.push(action(plugin, command, process));
    entry
}

/// What a global shortcut does.
#[derive(Clone, Debug)]
pub enum HotkeyTarget {
    /// Shows the launcher with this query, i.e. in a plugin's scope.
    Scope(String),
    /// Runs a command without showing the launcher.
    Command(Action),
}

#[derive(Clone, Debug)]
pub struct HotkeyBinding {
    /// What the shortcut belongs to, for messages: a plugin or "plugin · command".
    pub owner: String,
    /// As the user or the manifest wrote it.
    pub hotkey: String,
    pub target: HotkeyTarget,
}

/// Every shortcut the enabled plugins ask for, in catalog order.
pub fn bindings(plugins: &[Plugin]) -> Vec<HotkeyBinding> {
    let mut bindings = Vec::new();
    for plugin in plugins {
        let Some(process) = &plugin.process else {
            continue;
        };
        if let (Some(hotkey), Some(alias)) = (&plugin.hotkey, plugin.aliases.first()) {
            bindings.push(HotkeyBinding {
                owner: plugin.name.clone(),
                hotkey: hotkey.clone(),
                target: HotkeyTarget::Scope(format!("{alias} ")),
            });
        }
        for command in &plugin.commands {
            if let Some(hotkey) = &command.hotkey {
                bindings.push(HotkeyBinding {
                    owner: label(plugin, command),
                    hotkey: hotkey.clone(),
                    target: HotkeyTarget::Command(action(plugin, command, process)),
                });
            }
        }
    }
    bindings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_titles_icons_and_shortcut_overrides() {
        let manifests: Vec<CommandManifest> = serde_json::from_str(
            r#"[
                {"id":"paste-plain","title":{"defaultValue":"Paste as Plain Text"},
                 "icon":"builtin:type","hotkey":"Ctrl+Shift+Alt+V"},
                {"id":"other","title":{"defaultValue":"Other"},"icon":"C:\\x.ico"}
            ]"#,
        )
        .unwrap();
        let settings: Settings = serde_json::from_str(
            r#"{"plugins":{"clipboard":{"commands":{"other":{"hotkey":"Ctrl+F2"}}}}}"#,
        )
        .unwrap();
        let commands = load(
            "clipboard",
            &manifests,
            &settings,
            Path::new("."),
            &"builtin:clipboard".into(),
        )
        .unwrap();
        assert_eq!(commands[0].title, "Paste as Plain Text");
        assert_eq!(commands[0].hotkey.as_deref(), Some("Ctrl+Shift+Alt+V"));
        assert_eq!(commands[0].icon, "builtin:type".into());
        // Only builtin icons are taken from a command; others use the plugin's.
        assert_eq!(commands[1].icon, "builtin:clipboard".into());
        assert_eq!(commands[1].hotkey.as_deref(), Some("Ctrl+F2"));

        let duplicate = [manifests[0].clone(), manifests[0].clone()];
        assert!(
            load(
                "clipboard",
                &duplicate,
                &settings,
                Path::new("."),
                &"x".into()
            )
            .is_err()
        );
    }
}
