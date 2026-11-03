use mytools_core::Action;
use mytools_plugin_api::i18n::{Catalog, language};
use std::sync::OnceLock;

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let english = include_str!("../locales/en.json");
        Catalog::from_json(
            if language() == "zh-CN" {
                include_str!("../locales/zh-CN.json")
            } else {
                english
            },
            english,
        )
        .expect("bundled launcher translations must be valid")
    })
}

pub fn t(key: &str) -> String {
    catalog().get(key).into()
}
pub fn format(key: &str, values: &[(&str, &str)]) -> String {
    catalog().format(key, values)
}
pub fn action_title(action: &Action) -> String {
    action.title.clone().unwrap_or_else(|| t(action.title_key))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    #[test]
    fn bundled_languages_have_the_same_keys_and_placeholders() {
        let en: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../locales/en.json")).unwrap();
        let zh: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../locales/zh-CN.json")).unwrap();
        assert_eq!(en.keys().collect::<Vec<_>>(), zh.keys().collect::<Vec<_>>());
        for (key, value) in &en {
            for placeholder in [
                "{count}",
                "{name}",
                "{action}",
                "{text}",
                "{error}",
                "{shortcut}",
            ] {
                assert_eq!(
                    value.contains(placeholder),
                    zh[key].contains(placeholder),
                    "{key}"
                );
            }
        }
    }
}
