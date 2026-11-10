//! The settings window: edits plugin aliases and enabled state, then saves them for the
//! host to apply on its next search.
use crate::{
    I18n, PluginRow, SettingsWindow, i18n,
    plugins::{self, Entry, Paths},
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc};

struct State {
    paths: Paths,
    /// As last loaded or saved.
    saved: Vec<Entry>,
    /// As edited in the form.
    edited: Vec<Entry>,
}

pub fn run() -> anyhow::Result<()> {
    let ui = SettingsWindow::new()?;
    ui.global::<I18n>().on_text(|key| i18n::t(&key).into());
    let paths = Paths::current()?;
    let entries = plugins::list(&paths)?;
    let state = Rc::new(RefCell::new(State {
        paths,
        saved: entries.clone(),
        edited: entries,
    }));
    show(&ui, &state.borrow());

    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_aliases_edited(move |index, text| {
        edit(&weak, &copy, index, |entry| {
            entry.aliases = plugins::parse_aliases(&text)
        });
    });
    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_toggled(move |index, enabled| {
        edit(&weak, &copy, index, |entry| entry.enabled = enabled);
    });
    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_filter_edited(move |_| {
        if let Some(ui) = weak.upgrade() {
            filter(&ui, &copy.borrow());
        }
    });
    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_discard(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = copy.borrow_mut();
        state.edited = state.saved.clone();
        show(&ui, &state);
    });
    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_save(move || {
        let Some(ui) = weak.upgrade() else { return };
        let mut state = copy.borrow_mut();
        match plugins::save(&state.paths, &state.edited).and_then(|_| plugins::list(&state.paths)) {
            Ok(entries) => {
                state.saved = entries.clone();
                state.edited = entries;
                show(&ui, &state);
                ui.set_status(i18n::t("status.saved").into());
            }
            Err(error) => {
                ui.set_failed(true);
                ui.set_status(format!("{error:#}").into());
            }
        }
    });
    ui.run()?;
    Ok(())
}

/// Applies one change from the form without rebuilding the list, so focus stays put.
fn edit(
    weak: &slint::Weak<SettingsWindow>,
    state: &Rc<RefCell<State>>,
    index: i32,
    change: impl FnOnce(&mut Entry),
) {
    let Some(ui) = weak.upgrade() else { return };
    let mut state = state.borrow_mut();
    let Some(entry) = usize::try_from(index)
        .ok()
        .and_then(|i| state.edited.get_mut(i))
    else {
        return;
    };
    change(entry);
    let row = row(entry, &ui.get_filter());
    if let Some(model) = ui
        .get_plugins()
        .as_any()
        .downcast_ref::<VecModel<PluginRow>>()
    {
        let index = index as usize;
        // Keep the text as typed; only state the field does not own is refreshed.
        let mut current = model.row_data(index).unwrap_or(row.clone());
        current.enabled = row.enabled;
        current.aliases = row.aliases;
        model.set_row_data(index, current);
    }
    summarize(&ui, &state);
}

/// Rebuilds the whole list, e.g. after loading, saving or discarding.
fn show(ui: &SettingsWindow, state: &State) {
    let filter = ui.get_filter();
    let rows: Vec<PluginRow> = state
        .edited
        .iter()
        .map(|entry| row(entry, &filter))
        .collect();
    ui.set_plugins(ModelRc::new(VecModel::from(rows)));
    ui.set_status("".into());
    ui.set_failed(false);
    summarize(ui, state);
}

fn filter(ui: &SettingsWindow, state: &State) {
    let filter = ui.get_filter();
    let model = ui.get_plugins();
    for (index, entry) in state.edited.iter().enumerate() {
        if let Some(mut current) = model.row_data(index) {
            current.shown = matches(entry, &filter);
            model.set_row_data(index, current);
        }
    }
    summarize(ui, state);
}

fn summarize(ui: &SettingsWindow, state: &State) {
    let enabled = state.edited.iter().filter(|entry| entry.enabled).count();
    ui.set_summary(
        i18n::format(
            "plugins.summary",
            &[
                ("enabled", &enabled.to_string()),
                ("total", &state.edited.len().to_string()),
            ],
        )
        .into(),
    );
    ui.set_dirty(state.edited != state.saved);
    let filter = ui.get_filter();
    let shown = state
        .edited
        .iter()
        .filter(|entry| matches(entry, &filter))
        .count();
    ui.set_shown_count(shown as i32);
    // A new edit clears an old save result or error.
    if state.edited != state.saved {
        ui.set_status("".into());
        ui.set_failed(false);
    }
}

fn row(entry: &Entry, filter: &str) -> PluginRow {
    PluginRow {
        id: entry.id.clone().into(),
        name: entry.name.clone().into(),
        icon: entry.icon.clone().into(),
        aliases: entry.aliases.join(" ").into(),
        default_aliases: entry.default_aliases.join(" ").into(),
        enabled: entry.enabled,
        locked: entry.locked(),
        shown: matches(entry, filter),
    }
}

fn matches(entry: &Entry, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty()
        || [&entry.name, &entry.id]
            .into_iter()
            .chain(&entry.aliases)
            .any(|text| mytools_plugin_api::matching::rank(text, filter) < 4)
}
