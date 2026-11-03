mod codec;

use mytools_plugin_api::{Resource, SearchItem, i18n::Catalog, text};

fn search(query: &str, decode: bool, catalog: &Catalog) -> Vec<SearchItem> {
    if query.is_empty() {
        return vec![text::hint(catalog.get("usage"), catalog.get("formats"))];
    }
    let algorithms = if decode {
        &codec::DECODE_FORMATS[..]
    } else {
        &codec::ENCODE_FORMATS[..]
    };
    // A leading format selects one result; otherwise show every applicable format.
    let (selected, raw) = query
        .split_once(char::is_whitespace)
        .filter(|(name, _)| algorithms.contains(name))
        .map(|(name, input)| (Some(name), input))
        .unwrap_or((None, query));
    let input = match text::input(raw) {
        Ok(input) => input,
        Err(error) => return vec![text::hint(catalog.get("input.error"), error)],
    };
    algorithms
        .iter()
        .filter(|name| selected.is_none_or(|selected| selected == **name))
        .map(|name| match codec::transform(name, &input, decode) {
            Ok(output) => SearchItem {
                intent_match: false,
                title: output.clone(),
                description: (*name).into(),
                resource: Resource::Text { text: output },
                icon: None,
                actions: None,
                preview: None,
            },
            Err(()) => text::hint(catalog.get("invalid"), *name),
        })
        .collect()
}

pub fn run(decode: bool, english: &str, chinese: &str) -> std::io::Result<()> {
    let catalog = Catalog::for_language(english, chinese).expect("bundled codec locales");
    mytools_plugin_api::serve(|query| search(query, decode, &catalog))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_format_preserves_input_and_results_copy_the_output() {
        let catalog = Catalog::from_json("{}", "{}").unwrap();
        let items = search("base64  a ", false, &catalog);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].resource,
            mytools_plugin_api::Resource::Text {
                text: "IGEg".into()
            }
        );
        assert!(
            search("base64 !!!", true, &catalog)[0]
                .actions
                .as_ref()
                .unwrap()
                .is_empty()
        );
    }
}
