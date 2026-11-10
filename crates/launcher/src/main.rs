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
mod theme;
mod tray;
mod window_geometry;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    mytools_plugin_api::crash::install();
    app::run().inspect_err(|error| mytools_plugin_api::crash::report_error(&format!("{error:#}")))
}
