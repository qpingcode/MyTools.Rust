//! Starts Chrome with Gemini in Chrome ("Glic") enabled outside the regions Google rolls it
//! out to: rewrites the eligibility state Chrome caches in its profile files, then starts
//! Chrome with feature switches that skip the country and locale filters.
use mytools_plugin_api::{
    PluginActionRequest, PluginEffect, Resource, SearchItem, SearchResponse, i18n::Catalog, text,
};

mod chrome;
mod network;
mod policy;
mod profile;

const LAUNCH: &str = "launch";
const LAUNCH_UNCHECKED: &str = "launch-unchecked";

enum Failure {
    CountryUnknown,
    Country(String),
    ChromeMissing,
    Close(String),
    Repair(String),
    Start(String),
}

impl Failure {
    fn message(&self, catalog: &Catalog) -> String {
        let with_error = |key, error: &str| catalog.format(key, &[("error", error)]);
        match self {
            Self::CountryUnknown => catalog.get("country.unknown").into(),
            Self::Country(country) => catalog.format("country.other", &[("country", country)]),
            Self::ChromeMissing => catalog.get("chrome.missing").into(),
            Self::Close(error) => with_error("close.failed", error),
            Self::Repair(error) => with_error("repair.failed", error),
            Self::Start(error) => with_error("start.failed", error),
        }
    }
}

fn search(catalog: &Catalog) -> SearchResponse {
    // Restarting closes the user's windows, so the title says so before Enter is pressed.
    let key = if chrome::is_running() {
        "restart"
    } else {
        "launch"
    };
    let title = catalog.get(key).to_owned();
    let item = SearchItem {
        intent_match: false,
        description: catalog.get(&format!("{key}.detail")).into(),
        icon: Some("builtin:sparkles".into()),
        actions: Some(vec![
            text::command(LAUNCH, title.clone()),
            text::command(LAUNCH_UNCHECKED, catalog.get("unchecked").into()),
        ]),
        resource: Resource::PluginItem { id: LAUNCH.into() },
        preview: None,
        title,
    };
    SearchResponse {
        items: vec![item],
        ..Default::default()
    }
}

fn run(request: PluginActionRequest, catalog: &Catalog) -> SearchResponse {
    let check_network = match request.command.as_str() {
        LAUNCH => true,
        LAUNCH_UNCHECKED => false,
        _ => return error(catalog.get("invalid").into()),
    };
    match launch(check_network) {
        Ok(()) => SearchResponse {
            effect: Some(PluginEffect {
                message: catalog.get("started").into(),
                ..PluginEffect::hide()
            }),
            ..Default::default()
        },
        Err(failure) => error(failure.message(catalog)),
    }
}

fn launch(check_network: bool) -> Result<(), Failure> {
    if check_network {
        check_country(network::country())?;
    }
    let executable = chrome::executable().ok_or(Failure::ChromeMissing)?;
    let user_data = chrome::user_data_dir().ok_or(Failure::ChromeMissing)?;
    if chrome::is_running() {
        chrome::close().map_err(Failure::Close)?;
    }
    // After Chrome exits, so its final writes are patched rather than overwriting the patch.
    profile::repair(&user_data).map_err(Failure::Repair)?;
    let arguments = chrome::glic_arguments(&profile::last_used(&user_data));
    chrome::start(&executable, &arguments).map_err(Failure::Start)
}

/// Gemini's servers still refuse other regions, so a non-US exit is caught before Chrome
/// is closed for nothing.
fn check_country(country: Option<String>) -> Result<(), Failure> {
    match country {
        Some(code) if code.eq_ignore_ascii_case("US") => Ok(()),
        Some(code) => Err(Failure::Country(code)),
        None => Err(Failure::CountryUnknown),
    }
}

fn error(message: String) -> SearchResponse {
    SearchResponse {
        error: Some(message),
        ..Default::default()
    }
}

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    mytools_plugin_api::serve_interactive(|_| search(&catalog), |request| run(request, &catalog))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_us_exit_passes_the_network_check() {
        assert!(check_country(Some("us".into())).is_ok());
        assert!(matches!(
            check_country(Some("CN".into())),
            Err(Failure::Country(code)) if code == "CN"
        ));
        assert!(matches!(check_country(None), Err(Failure::CountryUnknown)));
    }

    #[test]
    fn unknown_commands_are_rejected() {
        let catalog = Catalog::from_json(r#"{"invalid":"Unknown"}"#, "{}").unwrap();
        let request: PluginActionRequest =
            serde_json::from_value(serde_json::json!({"command": "other"})).unwrap();
        assert_eq!(run(request, &catalog).error.as_deref(), Some("Unknown"));
    }
}
