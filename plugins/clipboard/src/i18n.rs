use mytools_plugin_api::i18n::Catalog;
use std::sync::OnceLock;

pub fn text(key: &str) -> String {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let english = include_str!("../locales/en.json");
            let locale = if mytools_plugin_api::i18n::language() == "zh-CN" {
                include_str!("../locales/zh-CN.json")
            } else {
                english
            };
            Catalog::from_json(locale, english).expect("bundled clipboard locales")
        })
        .lookup(key)
        .unwrap_or(key)
        .into()
}

pub fn format(key: &str, values: &[(&str, &str)]) -> String {
    let mut value = text(key);
    for (key, replacement) in values {
        value = value.replace(&format!("{{{key}}}"), replacement);
    }
    value
}
