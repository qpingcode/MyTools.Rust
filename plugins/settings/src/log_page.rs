//! Wires the log page: shows the records `logs` reads, newest first, and handles its
//! filters, expanding, copying, pausing and opening the log folder.
use crate::{
    LogRow, SettingsWindow, i18n,
    logs::{self, Filter, Logs},
};
use mytools_plugin_api::log::{HOST, Level};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{cell::RefCell, collections::HashSet, rc::Rc};

const LOGS_PAGE: i32 = 1;

struct Page {
    logs: Logs,
    filter: Filter,
    /// Choices of the source box, in order: all, MyTools, then each plugin.
    sources: Vec<Option<String>>,
    /// Plugin ids to names; MyTools and unknown ids show as they are.
    names: Vec<(String, String)>,
    expanded: HashSet<u64>,
    paused: bool,
}

pub struct LogPage(Rc<RefCell<Page>>);

impl LogPage {
    /// `plugins` are (id, name) pairs for the source box.
    pub fn wire(ui: &SettingsWindow, mut plugins: Vec<(String, String)>) -> Self {
        plugins.sort_by_key(|(_, name)| name.to_lowercase());
        let mut sources = vec![None, Some(HOST.to_owned())];
        sources.extend(plugins.iter().map(|(id, _)| Some(id.clone())));
        let labels: Vec<SharedString> = sources
            .iter()
            .map(|source| match source {
                None => i18n::t("logs.all_sources").into(),
                Some(id) => display_name(&plugins, id).into(),
            })
            .collect();
        ui.set_log_sources(ModelRc::new(VecModel::from(labels)));
        let page = Rc::new(RefCell::new(Page {
            logs: Logs::open(mytools_plugin_api::crash::directory()),
            filter: Filter {
                source: None,
                level: Level::Info,
                text: String::new(),
            },
            sources,
            names: plugins,
            expanded: HashSet::new(),
            paused: false,
        }));
        let this = Self(page);
        this.callbacks(ui);
        this
    }

    fn callbacks(&self, ui: &SettingsWindow) {
        let on = |action: fn(&mut Page, &SettingsWindow, i32)| {
            let (weak, page) = (ui.as_weak(), self.0.clone());
            move |value: i32| {
                if let Some(ui) = weak.upgrade() {
                    action(&mut page.borrow_mut(), &ui, value);
                }
            }
        };
        ui.on_page_changed(on(|page, ui, index| {
            if index == LOGS_PAGE {
                page.show(ui);
            }
        }));
        ui.on_log_source_changed(on(|page, ui, index| {
            page.filter.source = usize::try_from(index)
                .ok()
                .and_then(|index| page.sources.get(index).cloned())
                .flatten();
            page.show(ui);
        }));
        ui.on_log_level_changed(on(|page, ui, index| {
            page.filter.level = match index {
                0 => Level::Debug,
                2 => Level::Warn,
                3 => Level::Error,
                _ => Level::Info,
            };
            page.show(ui);
        }));
        ui.on_log_toggle(on(|page, ui, id| {
            let id = id as u64;
            if !page.expanded.remove(&id) {
                page.expanded.insert(id);
            }
            page.show(ui);
        }));
        ui.on_log_copy_row(on(|page, ui, id| {
            let text = page
                .logs
                .entries
                .iter()
                .find(|entry| entry.id == id as u64)
                .map(|entry| logs::plain(&entry.record));
            if let Some(text) = text {
                copy(ui, &text, 1);
            }
        }));
        let (weak, page) = (ui.as_weak(), self.0.clone());
        ui.on_log_search_edited(move |text| {
            if let Some(ui) = weak.upgrade() {
                let mut page = page.borrow_mut();
                page.filter.text = text.into();
                page.show(&ui);
            }
        });
        let (weak, page) = (ui.as_weak(), self.0.clone());
        ui.on_log_pause_toggled(move || {
            if let Some(ui) = weak.upgrade() {
                let mut page = page.borrow_mut();
                page.paused = !page.paused;
                ui.set_log_paused(page.paused);
                if !page.paused {
                    page.logs.refresh();
                    page.show(&ui);
                }
            }
        });
        let (weak, page) = (ui.as_weak(), self.0.clone());
        ui.on_log_copy_shown(move || {
            if let Some(ui) = weak.upgrade() {
                let page = page.borrow();
                let shown: Vec<String> = page
                    .logs
                    .entries
                    .iter()
                    .filter(|entry| page.filter.matches(&entry.record))
                    .map(|entry| logs::plain(&entry.record))
                    .collect();
                copy(&ui, &shown.join("\n"), shown.len());
            }
        });
        let page = self.0.clone();
        ui.on_log_open_folder(move || {
            open_folder(page.borrow().logs.directory());
        });
    }

    /// Picks up new records; the list is rebuilt only while the page is shown.
    pub fn tick(&self, ui: &SettingsWindow) {
        let mut page = self.0.borrow_mut();
        if !page.paused && page.logs.refresh() && ui.get_page() == LOGS_PAGE {
            page.show(ui);
        }
    }
}

impl Page {
    fn show(&self, ui: &SettingsWindow) {
        let rows: Vec<LogRow> = self
            .logs
            .entries
            .iter()
            .rev()
            .filter(|entry| self.filter.matches(&entry.record))
            .map(|entry| {
                let record = &entry.record;
                LogRow {
                    id: entry.id as i32,
                    // "2026-11-11 11:06:06.309" shows as "11-11 11:06:06".
                    time: record.time.get(5..19).unwrap_or(&record.time).into(),
                    stamp: record.time.clone().into(),
                    level: match record.level {
                        Level::Debug => "debug",
                        Level::Info => "info",
                        Level::Warn => "warn",
                        Level::Error => "error",
                    }
                    .into(),
                    source: display_name(&self.names, &record.source).into(),
                    event: record.event.clone().into(),
                    message: record.message.clone().into(),
                    detail: record.detail.clone().unwrap_or_default().into(),
                    expanded: self.expanded.contains(&entry.id),
                }
            })
            .collect();
        ui.set_log_summary(
            i18n::format(
                "logs.summary",
                &[
                    ("shown", &rows.len().to_string()),
                    ("total", &self.logs.entries.len().to_string()),
                ],
            )
            .into(),
        );
        ui.set_log_rows(ModelRc::new(VecModel::from(rows)));
    }
}

fn display_name(names: &[(String, String)], source: &str) -> String {
    if source == HOST {
        return "MyTools".into();
    }
    names
        .iter()
        .find(|(id, _)| id == source)
        .map_or_else(|| source.to_owned(), |(_, name)| name.clone())
}

fn copy(ui: &SettingsWindow, text: &str, count: usize) {
    let copied = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text));
    ui.set_log_summary(
        match copied {
            Ok(()) => i18n::format("logs.copied", &[("count", &count.to_string())]),
            Err(error) => i18n::format("logs.copy_failed", &[("error", &error.to_string())]),
        }
        .into(),
    );
}

fn open_folder(directory: &std::path::Path) {
    let _ = std::fs::create_dir_all(directory);
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(directory).spawn();
}
