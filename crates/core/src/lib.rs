//! UI-independent plugin discovery, search aggregation and application launching.

mod actions;
mod catalog;
mod history;
mod i18n;
mod platform;
mod plugins;
mod search_process;

pub use actions::{Action, ActionEffect, resolve_actions};
pub use catalog::{Catalog, Entry, SearchResults};
pub use mytools_plugin_api::{ActionId, ActionSpec, Resource};
pub use platform::open_application;
pub use plugins::{ActivationShortcut, activation_shortcuts};
