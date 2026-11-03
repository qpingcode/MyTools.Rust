use mytools_plugin_api::{Resource, SearchItem, serve};

mod expression;

fn search(query: &str) -> Vec<SearchItem> {
    let Some((expression, value)) = expression::preview(query) else {
        return vec![];
    };
    let value = if value == 0.0 {
        "0".to_owned()
    } else {
        value.to_string()
    };
    vec![SearchItem {
        intent_match: true,
        title: value.clone(),
        icon: None,
        description: format!("{expression} = {value}"),
        resource: Resource::Text { text: value },
        actions: None,
        preview: None,
    }]
}

fn main() -> std::io::Result<()> {
    serve(search)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_numbers_and_expressions_always_signal_intent() {
        for query in ["1+1", "123", "0", "1.5", "-42", " 42 ", "(3*5)/2"] {
            let results = search(query);
            assert_eq!(results.len(), 1, "{query}");
            assert!(results[0].intent_match, "{query}");
        }
        for query in ["", "1/0", "hello"] {
            assert!(search(query).is_empty(), "{query}");
        }
    }

    #[test]
    fn incomplete_input_keeps_computed_answer_and_intent() {
        for (query, value) in [
            ("32-1-", "31"),
            ("10+", "10"),
            ("5*5*", "25"),
            ("5+6*(", "11"),
        ] {
            let results = search(query);
            assert_eq!(results.len(), 1);
            assert!(results[0].intent_match);
            assert_eq!(results[0].title, value);
            assert_eq!(results[0].resource, Resource::Text { text: value.into() });
        }
        assert_eq!(search("5+6*(")[0].description, "5+6 = 11");
    }

    #[test]
    fn returns_result_and_copy_value() {
        let results = search("1+1");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "2");
        assert_eq!(results[0].resource, Resource::Text { text: "2".into() });
        assert!(search("not an expression").is_empty());
        assert!(search("").is_empty());
    }
}
