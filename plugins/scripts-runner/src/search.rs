use crate::{
    i18n, runner,
    store::{Script, Store},
};
use mytools_plugin_api::{
    PluginActionRequest, PluginEffect, Resource, SearchItem, SearchResponse, matching, text,
};

pub fn search(store: &Store, query: &str) -> anyhow::Result<SearchResponse> {
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
    for script in scripts.into_iter().take(49) {
        items.push(item(&script));
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
        let items = search(&store, "dev").unwrap().items;
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
        assert_eq!(search(&store, "开发").unwrap().items.len(), 1);
        // The manager is a manifest command now, not a result of the plugin.
        assert!(search(&store, "scripts").unwrap().items.is_empty());
        assert!(search(&store, "no match").unwrap().items.is_empty());
        std::fs::remove_dir_all(store.root).unwrap();
    }
}
