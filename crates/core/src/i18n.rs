//! Localized diagnostics only; action labels are resolved by the launcher.
use mytools_plugin_api::i18n::{Catalog, language};
use std::sync::OnceLock;

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let en = include_str!("../locales/en.json");
        Catalog::from_json(
            if language() == "zh-CN" {
                include_str!("../locales/zh-CN.json")
            } else {
                en
            },
            en,
        )
        .expect("bundled core translations must be valid")
    })
}

pub fn t(key: &str) -> String {
    catalog().get(key).into()
}
pub fn format(key: &str, values: &[(&str, &str)]) -> String {
    catalog().format(key, values)
}
