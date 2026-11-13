#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#[path = "../../../ui/fonts.rs"]
mod fonts;
mod hotkeys;
mod i18n;
mod log_page;
mod logs;
mod manager;
mod plugins;
mod search;
slint::include_modules!();
/// Shows a folder in the file manager, creating it first.
pub fn open_folder(directory: &std::path::Path) {
    let _ = std::fs::create_dir_all(directory);
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(directory).spawn();
}

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
