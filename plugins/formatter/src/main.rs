mod formats;
use formats::Format;
use mytools_plugin_api::{SearchItem, i18n::Catalog, text};

/// Formats the text after the alias, recognizing JSON or XML from its first character.
fn search(query: &str, catalog: &Catalog) -> Vec<SearchItem> {
    if query.trim().is_empty() {
        return vec![text::hint(catalog.get("usage"), catalog.get("formats"))];
    }
    let input = match text::input(query.trim_start()) {
        Ok(input) => input,
        Err(error) => return vec![text::hint(catalog.get("input.error"), error)],
    };
    let Some(format) = formats::detect(&input) else {
        return vec![text::hint(catalog.get("unknown"), catalog.get("formats"))];
    };
    let (result, invalid) = match format {
        Format::Json => ("result.json", "invalid.json"),
        Format::Xml => ("result.xml", "invalid.xml"),
    };
    match formats::format(format, &input) {
        Ok(formatted) => vec![text::editor_result(
            catalog.get(result),
            formatted.clone(),
            formatted,
        )],
        Err(error) => vec![text::hint(catalog.get(invalid), error)],
    }
}

fn main() -> std::io::Result<()> {
    mytools_plugin_api::crash::install();
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    mytools_plugin_api::serve(|query| search(query, &catalog))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(query: &str) -> (String, String) {
        let catalog = Catalog::from_json(include_str!("../locales/en.json"), "{}").unwrap();
        let items = search(query, &catalog);
        (items[0].title.clone(), items[0].description.clone())
    }

    #[test]
    fn the_format_comes_from_the_text_alone() {
        assert_eq!(titles(r#"{"a":1}"#).0, "Formatted JSON");
        assert_eq!(titles("\n  <a><b/></a>").0, "Formatted XML");
        assert_eq!(titles("{broken").0, "Not valid JSON");
        assert_eq!(titles("<a>").0, "Not valid XML");
        // The old format prefix is just text now.
        assert_eq!(titles(r#"json {"a":1}"#).0, "Not JSON or XML");
        assert_eq!(titles("  ").0, "Paste JSON or XML on the left");
    }
}
