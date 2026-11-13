use crate::{
    i18n, runner,
    store::{Script, Store},
    terminal,
};
use mytools_plugin_api::{
    PluginActionRequest, PluginEffect, Resource, SearchItem, SearchResponse, matching, text,
};

/// Scripts matching `query`. After an alias (`scoped`), e.g. `> podman ps`, the query can
/// also be run as a command in a terminal; see [`terminal_item`].
pub fn search(store: &Store, query: &str, scoped: bool) -> anyhow::Result<SearchResponse> {
    // "Manage Scripts" is the manifest command `manage`, which the launcher lists itself.
    let mut items = vec![];
    let mut scripts: Vec<_> = store
        .load()?
        .into_iter()
        .filter(|script| {
            query.trim().is_empty()
                || matching::rank(&script.name, query) < 4
                || (!script.alias.is_empty() && matching::rank(&script.alias, query) < 4)
        })
        .collect();
    scripts.sort_by_key(|script| {
        (
            matching::normalize(&script.alias) != matching::normalize(query),
            matching::rank(&script.name, query),
            script.name.clone(),
            script.id.clone(),
        )
    });
    let exact = scripts
        .iter()
        .take_while(|script| exact_match(script, query))
        .count();
    for script in scripts.into_iter().take(49) {
        items.push(item(&script));
    }
    if scoped && let Some(command) = terminal_item(query) {
        items.insert(exact.min(items.len()), command);
    }
    let mut bytes = 0;
    items.retain(|item| {
        bytes += serde_json::to_vec(item).map_or(1_048_576, |json| json.len());
        bytes < 900_000
    });
    Ok(SearchResponse {
        items,
        ..Default::default()
    })
}

/// A script whose alias or name is exactly the query stays above the terminal command, so
/// `> dev` still runs the script `dev` first.
fn exact_match(script: &Script, query: &str) -> bool {
    let query = matching::normalize(query);
    (!script.alias.is_empty() && matching::normalize(&script.alias) == query)
        || matching::normalize(&script.name) == query
}

/// Runs the typed text, e.g. `podman ps`, in a terminal window that stays open.
fn terminal_item(query: &str) -> Option<SearchItem> {
    let command = query.trim();
    if command.is_empty() {
        return None;
    }
    let mut item = text::item(command, command.to_owned(), i18n::t("terminal.description"));
    item.icon = Some("builtin:chevron-right".into());
    item.resource = Resource::PluginItem {
        id: format!("terminal:{command}"),
    };
    let action = |name: &str, title: &str| text::command(name, i18n::t(title)).with_item(command);
    let mut actions = vec![action("terminal", "terminal.run")];
    if cfg!(windows) {
        actions.push(action("terminal-admin", "admin"));
    }
    item.actions = Some(actions);
    Some(item)
}

fn item(script: &Script) -> SearchItem {
    let mut item = text::item(
        &script.name,
        script.content.clone(),
        // Scripts without an alias show only their type, without a dangling separator.
        match script.alias.as_str() {
            "" => script.kind.extension().to_owned(),
            alias => format!("{} · {alias}", script.kind.extension()),
        },
    );
    item.resource = Resource::PluginItem {
        id: script.id.clone(),
    };
    let command = |name: &str| text::command(name, i18n::t(name)).with_item(script.id.clone());
    let mut actions = vec![command("run")];
    if cfg!(windows) {
        actions.push(command("admin"));
    }
    actions.push(command("edit"));
    item.actions = Some(actions);
    item
}

pub fn execute(store: &Store, request: PluginActionRequest) -> anyhow::Result<PluginEffect> {
    let item = request.item.as_deref().unwrap_or_default();
    if request.command == "manage" && request.item.is_none() {
        open_manager(None)?;
        return Ok(PluginEffect {
            dismiss: true,
            ..Default::default()
        });
    } else if let Some(admin) = terminal_command(&request.command) {
        terminal::run(item, admin)?;
    } else {
        let script = store
            .load()?
            .into_iter()
            .find(|script| script.id == item)
            .ok_or_else(|| anyhow::anyhow!(i18n::t("missing")))?;
        match request.command.as_str() {
            "run" | "admin" => runner::run(store, &script, request.command == "admin")?,
            "edit" => open_manager(Some(&script.id))?,
            _ => anyhow::bail!(i18n::t("missing")),
        }
    }
    Ok(PluginEffect {
        message: i18n::t("run.started"),
        ..Default::default()
    })
}
/// Whether `command` runs a terminal command, and if so whether elevated.
fn terminal_command(command: &str) -> Option<bool> {
    match command {
        "terminal" => Some(false),
        "terminal-admin" => Some(true),
        _ => None,
    }
}

fn open_manager(id: Option<&str>) -> anyhow::Result<()> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.arg("--manage");
    if let Some(id) = id {
        command.arg(id);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_has_stable_resource_identity_and_distinct_run_actions() {
        let store = Store {
            root: std::env::temp_dir().join(format!("scripts-search-{}", uuid::Uuid::new_v4())),
        };
        let script = Script {
            id: uuid::Uuid::new_v4().to_string(),
            name: "开发环境".into(),
            alias: "dev".into(),
            kind: crate::store::Kind::Ps1,
            content: "Write-Output test".into(),
            directory: String::new(),
            mode: Default::default(),
        };
        store.update(script.clone(), None).unwrap();
        let items = search(&store, "dev", false).unwrap().items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].resource, Resource::PluginItem { id: script.id });
        assert_eq!(
            items[0].actions.as_ref().unwrap()[0].command.as_deref(),
            Some("run")
        );
        assert_eq!(
            items[0]
                .actions
                .as_ref()
                .unwrap()
                .iter()
                .any(|action| action.command.as_deref() == Some("admin")),
            cfg!(windows)
        );
        assert_eq!(search(&store, "开发", false).unwrap().items.len(), 1);
        // The manager is a manifest command now, not a result of the plugin.
        assert!(search(&store, "scripts", false).unwrap().items.is_empty());
        assert!(search(&store, "no match", false).unwrap().items.is_empty());
        std::fs::remove_dir_all(store.root).unwrap();
    }

    #[test]
    fn after_the_alias_the_query_runs_in_a_terminal_unless_a_script_is_named() {
        let store = Store {
            root: std::env::temp_dir().join(format!("scripts-terminal-{}", uuid::Uuid::new_v4())),
        };
        let script = Script {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Podman 状态".into(),
            alias: "dev".into(),
            kind: crate::store::Kind::Ps1,
            content: "podman ps".into(),
            directory: String::new(),
            mode: Default::default(),
        };
        store.update(script.clone(), None).unwrap();
        let resource = |id: &str| Resource::PluginItem { id: id.into() };
        let terminal = resource("terminal:podman");
        // `> ` lists every script and offers nothing to run.
        let all = search(&store, "", true).unwrap().items;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].resource, resource(&script.id));
        // `> podman` runs the command first; the script it also matches follows.
        let items = search(&store, " podman ", true).unwrap().items;
        assert_eq!(items[0].resource, terminal);
        assert_eq!(items[0].title, "podman");
        let action = &items[0].actions.as_ref().unwrap()[0];
        assert_eq!(action.command.as_deref(), Some("terminal"));
        assert_eq!(action.item.as_deref(), Some("podman"));
        assert_eq!(items[1].resource, resource(&script.id));
        // A script's exact alias keeps it first.
        let items = search(&store, "dev", true).unwrap().items;
        assert_eq!(items[0].resource, resource(&script.id));
        assert_eq!(items[1].resource, resource("terminal:dev"));
        // Global search never offers to run what was typed.
        assert!(
            search(&store, "podman", false)
                .unwrap()
                .items
                .iter()
                .all(|item| item.resource != terminal)
        );
        assert_eq!(terminal_command("terminal-admin"), Some(true));
        assert_eq!(terminal_command("run"), None);
        std::fs::remove_dir_all(store.root).unwrap();
    }
}
