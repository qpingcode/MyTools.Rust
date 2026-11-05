use mytools_plugin_api::i18n::Catalog;
use std::sync::OnceLock;
fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        Catalog::for_language(
            include_str!("../locales/en.json"),
            include_str!("../locales/zh-CN.json"),
        )
        .expect("script translations")
    })
}
pub fn t(key: &str) -> String {
    catalog().get(key).into()
}
pub fn format(key: &str, values: &[(&str, &str)]) -> String {
    catalog().format(key, values)
}
pub fn error(error: impl std::fmt::Display) -> String {
    catalog().format("error.detail", &[("error", &error.to_string())])
}
