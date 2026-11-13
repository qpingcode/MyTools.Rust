//! Fonts for the processes that show Slint windows: MyTools, the settings and the script
//! manager each include this file with `#[path]`, next to the shared `.slint` files.
use slint::fontique_011::{fontique, shared_collection};

/// The font every window uses; it covers Latin and Simplified Chinese.
pub const UI_FONT: &str = if cfg!(windows) {
    "Microsoft YaHei UI"
} else {
    "PingFang SC"
};

/// Draws Chinese characters that the requested font lacks (e.g. in code shown in Consolas)
/// with [`UI_FONT`]. Otherwise Slint asks the system for a Han font without saying which
/// language the text is in, and Windows may pick a Japanese font that has no simplified
/// characters, so those show as boxes. Call once the first window exists.
pub fn prefer_ui_font_for_chinese() {
    let mut collection = shared_collection();
    let Some(ui_font) = collection.family_id(UI_FONT) else {
        return;
    };
    let key = || fontique::FallbackKey::new(fontique::Script::from_bytes(*b"Hani"), None);
    let others: Vec<_> = collection
        .fallback_families(key())
        .filter(|family| *family != ui_font)
        .collect();
    collection.set_fallbacks(key(), std::iter::once(ui_font).chain(others));
}
