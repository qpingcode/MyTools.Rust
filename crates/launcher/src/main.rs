#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod actions;
mod app;
mod autostart;
mod config;
mod desktop;
mod elide;
mod i18n;
mod icons;
mod notices;
mod search;
mod single_instance;
mod theme;
mod tray;
mod window_geometry;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    mytools_plugin_api::crash::install();
    // A restart or deploy starts the new MyTools while the old one is still exiting.
    let wait = if std::env::args().any(|argument| argument == "--restart") {
        single_instance::RESTART_WAIT
    } else {
        std::time::Duration::ZERO
    };
    let Some(instance) = single_instance::claim(wait) else {
        single_instance::show_running();
        return Ok(());
    };
    app::run(&instance)
        .inspect_err(|error| mytools_plugin_api::crash::report_error(&format!("{error:#}")))
}
