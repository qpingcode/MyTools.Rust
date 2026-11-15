mod codec;

use mytools_plugin_api::{Resource, SearchItem, i18n::Catalog, text};

fn search(query: &str, decode: bool, literal: bool, catalog: &Catalog) -> Vec<SearchItem> {
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
        .filter(|(name, _)| !literal && algorithms.contains(name))
        .map(|(name, input)| (Some(name), input))
        .unwrap_or((None, query));
    let input = match text::read_input(raw, literal) {
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
    let command = if decode { "decode-text" } else { "encode-text" };
    mytools_plugin_api::serve_requests(
        |request| mytools_plugin_api::SearchResponse {
            items: search(&request.query, decode, request.literal, &catalog),
            ..Default::default()
        },
        |action| text::search_attachment(action, command, catalog.get("input.error")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_literal_selection_keeps_the_algorithm_name_in_the_text() {
        let catalog = Catalog::from_json("{}", "{}").unwrap();
        let selection = "base64 a";
        let items = search(selection, false, true, &catalog);
        assert!(items.len() > 1);
        assert!(items.iter().any(|item| {
            item.description == "base64"
                && item.resource
                    == Resource::Text {
                        text: "YmFzZTY0IGE=".into(),
                    }
        }));
    }

    #[test]
    fn explicit_format_preserves_input_and_results_copy_the_output() {
        let catalog = Catalog::from_json("{}", "{}").unwrap();
        let items = search("base64  a ", false, false, &catalog);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].resource,
            mytools_plugin_api::Resource::Text {
                text: "IGEg".into()
            }
        );
        assert!(
            search("base64 !!!", true, false, &catalog)[0]
                .actions
                .as_ref()
                .unwrap()
                .is_empty()
        );
    }
}
