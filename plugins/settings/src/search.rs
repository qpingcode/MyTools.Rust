use crate::i18n;
use mytools_plugin_api::{PluginActionRequest, PluginEffect, SearchResponse};

/// The window is opened through the manifest command `open`, which the launcher finds and
/// lists itself; the plugin has no results of its own.
pub fn search(_query: &str) -> SearchResponse {
    SearchResponse::default()
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
    fn only_the_open_command_is_understood() {
        assert!(search("settings").items.is_empty());
        let unknown = PluginActionRequest {
            command: "delete".into(),
            item: None,
            editable: None,
        };
        assert!(execute(unknown).is_err());
    }
}
