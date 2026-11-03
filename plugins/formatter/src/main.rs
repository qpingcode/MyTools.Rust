mod formats;
use mytools_plugin_api::{i18n::Catalog, text};

fn search(query: &str, catalog: &Catalog) -> Vec<mytools_plugin_api::SearchItem> {
    let Some((format, input)) = query.split_once(char::is_whitespace) else {
        return vec![text::hint(catalog.get("usage"), catalog.get("formats"))];
    };
    let input = match text::input(input) {
        Ok(input) => input,
        Err(error) => return vec![text::hint(catalog.get("input.error"), error)],
    };
    let formatted = match formats::format(format, &input) {
        Ok(output) => output,
        Err(error) => return vec![text::hint(catalog.get("invalid"), error)],
    };
    vec![text::item(
        catalog.get("result"),
        formatted,
        catalog.get("copy"),
    )]
}

fn main() -> std::io::Result<()> {
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    mytools_plugin_api::serve(|query| search(query, &catalog))
}
