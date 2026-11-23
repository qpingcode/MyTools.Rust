//! ediProd helpers. "Fix Job Hyperlink on Clipboard" rewrites a copied job hyperlink so
//! plain-text targets paste "WI01147547 - Title" and rich-text targets paste a link with
//! that title, instead of the bare URL CargoWise puts in the plain text.
use mytools_plugin_api::{PluginActionRequest, PluginEffect, SearchResponse, i18n::Catalog};

#[cfg(windows)]
mod clipboard;
mod link;

const FIX_JOB_LINK: &str = "fix-job-link";

fn run(request: PluginActionRequest, catalog: &Catalog) -> SearchResponse {
    if request.command != FIX_JOB_LINK {
        return error(catalog.get("unsupported").into());
    }
    match fix_job_link(catalog) {
        Ok(title) => SearchResponse {
            effect: Some(PluginEffect {
                message: catalog.format("fixed", &[("title", &title)]),
                ..PluginEffect::hide()
            }),
            ..Default::default()
        },
        Err(message) => error(message),
    }
}

#[cfg(windows)]
fn fix_job_link(catalog: &Catalog) -> Result<String, String> {
    let failed = |error: String| catalog.format("clipboard_failed", &[("error", &error)]);
    let content = clipboard::read().map_err(failed)?;
    let job = link::find(content.html.as_deref(), content.text.as_deref())
        .map_err(|problem| problem_message(problem, catalog))?;
    let html = link::cf_html(&link::html(&job));
    clipboard::write(&job.title, &html, &link::rtf(&job)).map_err(failed)?;
    Ok(job.title)
}

#[cfg(not(windows))]
fn fix_job_link(catalog: &Catalog) -> Result<String, String> {
    Err(catalog.get("unsupported").into())
}

fn problem_message(problem: link::Problem, catalog: &Catalog) -> String {
    catalog
        .get(match problem {
            link::Problem::NoLink => "no_link",
            link::Problem::NoTitle => "no_title",
            link::Problem::SeveralLinks => "several_links",
        })
        .into()
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
    // The plugin has no alias: it only offers its command in global search.
    mytools_plugin_api::serve_interactive(
        |_| SearchResponse::default(),
        |request| run(request, &catalog),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_commands_are_rejected() {
        let catalog = Catalog::from_json(r#"{"unsupported":"Unknown"}"#, "{}").unwrap();
        let request: PluginActionRequest =
            serde_json::from_value(serde_json::json!({"command": "other"})).unwrap();
        assert_eq!(run(request, &catalog).error.as_deref(), Some("Unknown"));
    }

    /// Rewrites the real clipboard; copy a job hyperlink first, then run with `--ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn fixes_the_job_link_on_the_clipboard() {
        let catalog = Catalog::from_json("{}", "{}").unwrap();
        let title = fix_job_link(&catalog).unwrap();
        // Clipboard monitors react to the change first; read once they are done.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let content = clipboard::read().unwrap();
        assert_eq!(content.text.as_deref(), Some(title.as_str()));
        // The rewritten HTML still holds the same link, so running it again changes nothing.
        let job = link::find(content.html.as_deref(), content.text.as_deref()).unwrap();
        assert_eq!(job.title, title);
        assert_eq!(fix_job_link(&catalog).unwrap(), title);
    }
}
