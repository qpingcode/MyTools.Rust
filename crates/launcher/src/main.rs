#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod actions;
mod app;
mod autostart;
mod config;
mod desktop;
mod i18n;
mod icons;
mod search;
mod tray;
mod window_position;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    app::run()
}
