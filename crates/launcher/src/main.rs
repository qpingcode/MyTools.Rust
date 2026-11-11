#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod actions;
mod app;
mod autostart;
mod commands;
mod config;
mod desktop;
mod elide;
mod highlight;
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
    use mytools_plugin_api::log;
    mytools_plugin_api::crash::install();
    // MyTools writes the log for itself and its plugins; `log_level` in config.json sets
    // the least severe level kept.
    let level = config::load()
        .ok()
        .and_then(|config| config.log_level)
        .and_then(|level| log::Level::parse(&level))
        .unwrap_or(log::Level::Info);
    log::init_file(mytools_plugin_api::crash::directory(), level);
    // A restart or deploy starts the new MyTools while the old one is still exiting.
    let wait = if std::env::args().any(|argument| argument == "--restart") {
        single_instance::RESTART_WAIT
    } else {
        std::time::Duration::ZERO
    };
    let Some(instance) = single_instance::claim(wait) else {
        log::info(
            "app.already_running",
            "MyTools is already running; showing its window instead",
        );
        single_instance::show_running();
        return Ok(());
    };
    log::info(
        "app.start",
        format!(
            "MyTools {} started (PID {})",
            env!("CARGO_PKG_VERSION"),
            std::process::id()
        ),
    );
    let result = app::run(&instance)
        .inspect_err(|error| mytools_plugin_api::crash::report_error(&format!("{error:#}")));
    log::info("app.exit", "MyTools exited");
    result
}
