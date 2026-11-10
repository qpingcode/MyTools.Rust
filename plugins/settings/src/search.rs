use crate::i18n;
use mytools_plugin_api::{
    PluginActionRequest, PluginEffect, Resource, SearchResponse, matching, text,
};

/// Offers the settings window for an empty alias query or a query naming settings.
pub fn search(query: &str) -> SearchResponse {
    let query = query.trim();
    let matches = query.is_empty()
        || ["settings", "setting", "设置", "插件"]
            .iter()
            .any(|word| matching::rank(word, query) < 4)
        || matching::rank(&i18n::t("name"), query) < 4;
    let mut items = vec![];
    if matches {
        let mut item = text::item(i18n::t("open"), String::new(), i18n::t("open.detail"));
        item.resource = Resource::PluginItem {
            id: "window".into(),
        };
        item.actions = Some(vec![
            text::command("open", i18n::t("open")).with_item("window"),
        ]);
        items.push(item);
    }
    SearchResponse {
        items,
        ..Default::default()
    }
}

pub fn execute(request: PluginActionRequest) -> anyhow::Result<PluginEffect> {
    anyhow::ensure!(request.command == "open", i18n::t("error.command"));
    // The window is its own process so the search protocol stays responsive.
    std::process::Command::new(std::env::current_exe()?)
        .arg("--manage")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(PluginEffect {
        dismiss: true,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offers_the_window_for_settings_words_only() {
        for query in ["", "settings", "SET", "设置"] {
            let items = search(query).items;
            assert_eq!(items.len(), 1, "{query}");
            assert_eq!(
                items[0].resource,
                Resource::PluginItem {
                    id: "window".into()
                }
            );
        }
        assert!(search("calculator").items.is_empty());
    }
}
