#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
mod i18n;
mod manager;
mod runner;
mod search;
mod store;
slint::include_modules!();
fn main() -> anyhow::Result<()> {
    mytools_plugin_api::i18n::language();
    if std::env::args().nth(1).as_deref() == Some("--manage") {
        return manager::run(std::env::args().nth(2));
    }
    let store = store::Store::new()?;
    mytools_plugin_api::serve_interactive(
        |query| search::search(&store, query).unwrap_or_else(response_error),
        |request| {
            search::execute(&store, request)
                .map(|effect| mytools_plugin_api::SearchResponse {
                    effect: Some(effect),
                    ..Default::default()
                })
                .unwrap_or_else(response_error)
        },
    )?;
    Ok(())
}
fn response_error(error: anyhow::Error) -> mytools_plugin_api::SearchResponse {
    mytools_plugin_api::SearchResponse {
        error: Some(i18n::error(error)),
        ..Default::default()
    }
}
