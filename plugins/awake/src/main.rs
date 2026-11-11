//! Keeps the computer awake and the screen on for as long as the plugin runs: enabling it
//! keeps the computer from sleeping, disabling it (or quitting MyTools) restores the power
//! plan. Nothing is changed in the power settings, so nothing needs restoring after a crash.
use mytools_plugin_api::{SearchItem, i18n::Catalog, text};

mod platform;

fn status(result: &Result<(), String>, catalog: &Catalog) -> Vec<SearchItem> {
    let mut item = match result {
        Ok(()) => text::hint(catalog.get("active"), catalog.get("active.detail")),
        Err(error) => text::hint(
            catalog.get("failed"),
            catalog.format("failed.detail", &[("error", error)]),
        ),
    };
    item.icon = Some(
        if result.is_ok() {
            "builtin:sun"
        } else {
            "builtin:triangle-alert"
        }
        .into(),
    );
    vec![item]
}

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    // Held by the main thread, which lives as long as the process: Windows ties the request
    // to the thread that made it, and both systems drop it when the process ends.
    let awake = platform::keep_awake();
    if let Err(error) = &awake {
        mytools_plugin_api::log::error("awake.failed", error);
    }
    mytools_plugin_api::serve(|_| status(&awake, &catalog))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_whether_the_request_was_granted() {
        let catalog = Catalog::from_json(
            r#"{"active":"On","failed":"Off","failed.detail":"Refused: {error}"}"#,
            "{}",
        )
        .unwrap();
        let on = status(&Ok(()), &catalog);
        assert_eq!(on[0].title, "On");
        let off = status(&Err("denied".into()), &catalog);
        assert_eq!(off[0].title, "Off");
        assert_eq!(off[0].description, "Refused: denied");
        // A status line has nothing to run.
        assert!(on[0].actions.as_ref().unwrap().is_empty());
    }
}
