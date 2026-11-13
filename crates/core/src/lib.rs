//! UI-independent plugin discovery, search aggregation and application launching.

mod actions;
mod catalog;
mod chat;
mod commands;
mod history;
mod i18n;
mod platform;
mod plugins;
mod search_process;
mod status;

pub use actions::{Action, ActionEffect, resolve_actions};
pub use catalog::{AliasTable, Catalog, Entry, LoadOptions, SearchOptions, SearchResults};
pub use chat::{ChatSession, ChatUpdates};
pub use commands::{HotkeyBinding, HotkeyTarget};
pub use mytools_plugin_api::{ActionId, ActionSpec, Resource};
pub use platform::{open_application, open_url};
pub use plugins::{migrate_legacy_hotkeys, startup_timeout};
pub use status::Notice;
