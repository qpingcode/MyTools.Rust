#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod actions;
mod app;
mod autostart;
mod config;
mod desktop;
mod elide;
mod i18n;
mod icons;
mod search;
mod theme;
mod tray;
mod window_position;

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    app::run()
}
