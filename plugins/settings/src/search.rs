use crate::i18n;
use mytools_plugin_api::{PluginActionRequest, PluginEffect, SearchResponse};

/// The window is opened through the manifest command `open`, and the data folder through
/// `data-folder`; the launcher finds and lists them itself, so the plugin has no results of
/// its own.
pub fn search(_query: &str) -> SearchResponse {
    SearchResponse::default()
}

pub fn execute(request: PluginActionRequest) -> anyhow::Result<PluginEffect> {
    match request.command.as_str() {
        "open" => {
            // The window is its own process so the search protocol stays responsive.
            std::process::Command::new(std::env::current_exe()?)
                .arg("--manage")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
        }
        "data-folder" => crate::open_folder(&mytools_plugin_api::dirs::root()),
        _ => anyhow::bail!(i18n::t("error.command")),
    }
    Ok(PluginEffect {
        dismiss: true,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_commands_are_refused() {
        assert!(search("settings").items.is_empty());
        let unknown = PluginActionRequest {
            command: "delete".into(),
            item: None,
            editable: None,
        };
        assert!(execute(unknown).is_err());
    }
}
