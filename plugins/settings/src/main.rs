#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod hotkeys;
mod i18n;
mod log_page;
mod logs;
mod manager;
mod plugins;
mod search;
slint::include_modules!();
fn main() -> anyhow::Result<()> {
    mytools_plugin_api::crash::install();
    mytools_plugin_api::i18n::language();
    if std::env::args().nth(1).as_deref() == Some("--manage") {
        return manager::run();
    }
    mytools_plugin_api::serve_interactive(search::search, |request| {
        search::execute(request)
            .map(|effect| mytools_plugin_api::SearchResponse {
                effect: Some(effect),
                ..Default::default()
            })
            .unwrap_or_else(|error| mytools_plugin_api::SearchResponse {
                error: Some(i18n::format(
                    "error.detail",
                    &[("error", &format!("{error:#}"))],
                )),
                ..Default::default()
            })
    })?;
    Ok(())
}
