//! The settings window: edits plugin aliases, shortcuts, startup and enabled state, then saves them
//! for the host to apply; shows each plugin's live state and can ask for a restart.
use crate::{
    HotkeyRow, I18n, PluginRow, SettingsWindow, i18n,
    plugins::{self, Entry, Paths},
};
use mytools_plugin_api::{
    plugin_settings::Startup,
    plugin_status::{self, PluginStatus, State as PluginState, StatusFile},
};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{cell::RefCell, rc::Rc, time::Duration};

/// How often the window looks for state changes and new log records from MyTools.
const STATUS_POLL: Duration = Duration::from_millis(500);

struct State {
    paths: Paths,
    /// As last loaded or saved.
    saved: Vec<Entry>,
    /// As edited in the form.
    edited: Vec<Entry>,
    /// What MyTools last published; empty when it is not running.
    status: StatusFile,
}

pub fn run() -> anyhow::Result<()> {
    let ui = SettingsWindow::new()?;
    crate::fonts::prefer_ui_font_for_chinese();
    ui.global::<I18n>().on_text(|key| i18n::t(&key).into());
    let paths = Paths::current()?;
    let entries = plugins::list(&paths)?;
    let status = plugin_status::load(&plugin_status::path(&paths.settings)).unwrap_or_default();
    let state = Rc::new(RefCell::new(State {
        paths,
        saved: entries.clone(),
        edited: entries,
        status,
    }));
    show(&ui, &state.borrow());
    let names = state
        .borrow()
        .saved
        .iter()
        .map(|entry| (entry.id.clone(), entry.name.clone()))
        .collect();
    let log_page = crate::log_page::LogPage::wire(&ui, names);
    ui.on_open_data_folder(|| crate::open_folder(&mytools_plugin_api::dirs::root()));

    let (weak, copy) = (ui.as_weak(), state.clone());
    let poll = slint::Timer::default();
    poll.start(slint::TimerMode::Repeated, STATUS_POLL, move || {
        if let Some(ui) = weak.upgrade() {
            refresh_status(&ui, &mut copy.borrow_mut());
            log_page.tick(&ui);
        }
    });
    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_restart(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let state = copy.borrow();
        let Some(entry) = usize::try_from(index)
            .ok()
            .and_then(|i| state.edited.get(i))
        else {
            return;
        };
        match plugin_status::request_restart(&state.paths.settings, &entry.id) {
            Ok(()) => {
                ui.set_failed(false);
                ui.set_status(i18n::format("status.restarting", &[("name", &entry.name)]).into());
            }
            Err(error) => {
                ui.set_failed(true);
                ui.set_status(format!("{error:#}").into());
            }
        }
    });
    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_startup_toggled(move |index| {
        edit(&weak, &copy, index, |entry| {
            entry.startup = entry.startup.map(|startup| match startup {
                Startup::Eager => Startup::Lazy,
                Startup::Lazy => Startup::Eager,
            });
        });
    });

    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_aliases_edited(move |index, text| {
        edit(&weak, &copy, index, |entry| {
            entry.aliases = plugins::parse_aliases(&text)
        });
    });
    let (weak, copy) = (ui.as_weak(), state.clone());
    ui.on_hotkey_edited(move |index, row, text| {
        edit(&weak, &copy, index, |entry| {
            if let Some(hotkey) = usize::try_from(row)
                .ok()
                .and_then(|row| entry.hotkeys.get_mut(row))
            {
                hotkey.hotkey = text.trim().to_owned();
            }
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
    let state = &mut *state;
    let Some(entry) = usize::try_from(index)
        .ok()
        .and_then(|i| state.edited.get_mut(i))
    else {
        return;
    };
    change(entry);
    let row = row(entry, &ui.get_filter(), state.status.get(&entry.id));
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
        current.startup = row.startup;
        // The same shortcut model stays, so the fields keep their text and focus.
        for (position, hotkey) in entry.hotkeys.iter().enumerate() {
            if let Some(mut shown) = current.hotkeys.row_data(position)
                && shown.hotkey != hotkey.hotkey.as_str()
            {
                shown.hotkey = hotkey.hotkey.clone().into();
                current.hotkeys.set_row_data(position, shown);
            }
        }
        model.set_row_data(index, current);
    }
    summarize(&ui, state);
}

/// Rebuilds the whole list, e.g. after loading, saving or discarding.
fn show(ui: &SettingsWindow, state: &State) {
    let filter = ui.get_filter();
    let rows: Vec<PluginRow> = state
        .edited
        .iter()
        .map(|entry| row(entry, &filter, state.status.get(&entry.id)))
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

/// Picks up state changes MyTools published, updating rows in place so edits and focus stay.
fn refresh_status(ui: &SettingsWindow, state: &mut State) {
    let status =
        plugin_status::load(&plugin_status::path(&state.paths.settings)).unwrap_or_default();
    if status == state.status {
        return;
    }
    state.status = status;
    let model = ui.get_plugins();
    for (index, entry) in state.edited.iter().enumerate() {
        if let Some(mut current) = model.row_data(index) {
            let (live, text) = describe(state.status.get(&entry.id));
            if current.state != live || current.status_text != text {
                current.state = live.into();
                current.status_text = text.into();
                model.set_row_data(index, current);
            }
        }
    }
}

/// The dot's state name and the words shown when hovering it.
fn describe(status: Option<&PluginStatus>) -> (&'static str, String) {
    let Some(status) = status else {
        return ("", i18n::t("state.unknown"));
    };
    let (name, key) = match status.state {
        PluginState::Ready => ("ready", "state.ready"),
        PluginState::Starting => ("starting", "state.starting"),
        PluginState::Idle => ("idle", "state.idle"),
        PluginState::Failed => ("failed", "state.failed"),
        PluginState::Disabled => ("disabled", "state.disabled"),
    };
    let text = match &status.detail {
        Some(detail) => i18n::format(
            "state.detail",
            &[("state", &i18n::t(key)), ("detail", detail)],
        ),
        None => i18n::t(key),
    };
    (name, text)
}

fn row(entry: &Entry, filter: &str, status: Option<&PluginStatus>) -> PluginRow {
    let (live, status_text) = describe(status);
    PluginRow {
        id: entry.id.clone().into(),
        name: entry.name.clone().into(),
        icon: entry.icon.clone().into(),
        aliases: entry.aliases.join(" ").into(),
        default_aliases: entry.default_aliases.join(" ").into(),
        enabled: entry.enabled,
        locked: entry.locked(),
        shown: matches(entry, filter),
        state: live.into(),
        status_text: status_text.into(),
        startup: match entry.startup {
            Some(Startup::Eager) => "eager",
            Some(Startup::Lazy) => "lazy",
            None => "",
        }
        .into(),
        hotkeys: ModelRc::new(VecModel::from(
            entry
                .hotkeys
                .iter()
                .map(|hotkey| HotkeyRow {
                    label: hotkey.label.clone().into(),
                    hotkey: hotkey.hotkey.clone().into(),
                    default_hotkey: hotkey.default.clone().into(),
                })
                .collect::<Vec<_>>(),
        )),
    }
}

fn matches(entry: &Entry, filter: &str) -> bool {
    let filter = filter.trim();
    filter.is_empty()
        || [&entry.name, &entry.id]
            .into_iter()
            .chain(&entry.aliases)
            .chain(entry.hotkeys.iter().map(|hotkey| &hotkey.label))
            .any(|text| mytools_plugin_api::matching::rank(text, filter) < 4)
}
