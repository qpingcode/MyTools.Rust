use mytools_plugin_api::{SearchItem, i18n::Catalog, text};
use uuid::Uuid;

fn generate(query: &str, catalog: &Catalog) -> Vec<SearchItem> {
    let parts: Vec<_> = query.split_whitespace().collect();
    let (version, count) = match parts.as_slice() {
        [] => ("v4", 1),
        [version @ ("v4" | "v7")] => (*version, 1),
        [count] => ("v4", count.parse::<usize>().unwrap_or(0)),
        [version @ ("v4" | "v7"), count] => (*version, count.parse::<usize>().unwrap_or(0)),
        _ => ("", 0),
    };
    if !(1..=50).contains(&count) {
        return vec![text::hint(catalog.get("usage"), "uuid [v4|v7] [1..50]")];
    }
    let values: Vec<_> = (0..count)
        .map(|_| {
            if version == "v7" {
                Uuid::now_v7()
            } else {
                Uuid::new_v4()
            }
            .to_string()
        })
        .collect();
    if count > 1 {
        let mut items = vec![text::item(
            catalog.get("batch"),
            values.join("\n"),
            catalog.get("copy"),
        )];
        items.extend(
            values
                .into_iter()
                .map(|value| text::item(value.clone(), value, version)),
        );
        items.truncate(50);
        items
    } else {
        let value = &values[0];
        [
            value.clone(),
            value.to_uppercase(),
            value.replace('-', ""),
            format!("{{{value}}}"),
        ]
        .into_iter()
        .map(|value| text::item(value.clone(), value, catalog.get("copy")))
        .collect()
    }
}

fn main() -> std::io::Result<()> {
    let catalog = Catalog::for_language(
        include_str!("../locales/en.json"),
        include_str!("../locales/zh-CN.json"),
    )
    .unwrap();
    mytools_plugin_api::serve(|query| generate(query, &catalog))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generates_real_versions_and_limits_batches() {
        let catalog = Catalog::from_json("{}", "{}").unwrap();
        for (version, number) in [("v4", 4), ("v7", 7)] {
            let items = generate(version, &catalog);
            let value = Uuid::parse_str(&items[0].title).unwrap();
            assert_eq!(value.get_version_num(), number);
        }
        assert_eq!(generate("v4 3", &catalog).len(), 4);
        assert!(
            generate("v7 51", &catalog)[0]
                .actions
                .as_ref()
                .unwrap()
                .is_empty()
        );
    }
}
